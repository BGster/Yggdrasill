-- `pkg.role-skill.*` 自测。契约 `docs/design/packages/role-skill.md` §7。
--
-- # 为什么自带 mock 与计数
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）：这里就地造一个**语义等价**的
-- 宿主面——`kv` 只收标量、`emit` 记账、`on` 记 handler；`entity` 只暴露本包允许
-- 调用的两个读面（`get` / `has`），**没有**写面：接口缝一旦被越过，代用面直接
-- `nil` 索引报错，比事后核对调用清单更硬。
--
-- 编码面用**真的** `pkg.codec`：KV 载荷字符串就是它产的，mock 一份编码器等于把
-- 「载荷是不是确定性字节」这条一起 mock 掉。
--
-- # 隔离口径
--
-- 每条用例 `fresh()` 一次：清 `pkg.role_skill*` 全部 `package.loaded` 项 + 新建一份
-- mock。用例之间不共享任何状态（含 `role.index` 与订阅表）。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-role-skill.lua`，`--filter pkg-role-skill` 跑到它。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function run(name, body)
  local ok, err = pcall(body)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(err))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error(
      ("%s: expected %s, got %s"):format(tostring(label or "eq"), tostring(expected), tostring(actual)),
      2
    )
  end
end

local function eq_seq(actual, expected, label)
  eq(#actual, #expected, (label or "eq_seq") .. " 长度")
  for i = 1, #expected do
    eq(actual[i], expected[i], (label or "eq_seq") .. "[" .. i .. "]")
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy", 2)
  end
end

local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

local function expect_error(code, body, label)
  local ok, err = pcall(body)
  if ok then
    error((label or "expect_error") .. ": expected " .. code .. ", but the call succeeded", 2)
  end
  if not tostring(err):find(code, 1, true) then
    error(("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)), 2)
  end
end

local codec = require("pkg.codec")
codec.install()

-- ── mock 宿主面（语义对齐内核；见文件头）───────────────────────────────────────

local function new_world()
  local kv = {}
  local reads, writes = 0, 0
  local events = {}
  local handlers = {}
  local attrs = {}
  local tags = {}

  local entity = {
    get = function(id, key)
      local bag = attrs[id]
      if bag == nil then
        return nil
      end
      return bag[key]
    end,
    has = function(id, tag)
      local set = tags[id]
      return set ~= nil and set[tag] == true
    end,
  }

  local world = {
    kv = kv,
    events = events,
    handlers = handlers,
    entity = entity,
  }

  world.host = {
    emit = function(name, payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
    kv = {
      get = function(key)
        reads = reads + 1
        return kv[key]
      end,
      set = function(key, value)
        writes = writes + 1
        kv[key] = value
      end,
    },
    on = function(name, handler)
      handlers[name] = handler
    end,
  }

  -- 测试侧的布置面（不走被测代码）：属性 / 标签只经这两个口子改。
  world.set_attr = function(id, key, value)
    if attrs[id] == nil then
      attrs[id] = {}
    end
    attrs[id][key] = value
  end
  world.add_tag = function(id, tag)
    if tags[id] == nil then
      tags[id] = {}
    end
    tags[id][tag] = true
  end
  -- 事件日志是**脚手架**：真实宿主把派生事件排队、派发后释放。泄漏用例要测的是包的
  -- 增长面，不清日志就变成测「日志数组增长」——那是 mock 的性质，不是包的性质。
  world.clear_events = function()
    for i = #events, 1, -1 do
      events[i] = nil
    end
  end
  world.reads_now = function()
    return reads
  end
  world.writes_now = function()
    return writes
  end
  return world
end

--- 取一份**全新**的包实例并装上 mock（清缓存 = 定义表 / 索引缓存 / 订阅表归零）。
local function fresh(world, events, window_cap)
  package.loaded["pkg.role_skill"] = nil
  for name in pairs(package.loaded) do
    if type(name) == "string" and name:find("^pkg%.role_skill%.") then
      package.loaded[name] = nil
    end
  end
  local role = require("pkg.role_skill")
  role.install({
    codec = codec,
    entity = world.entity,
    events = events,
    host = world.host,
    window_cap = window_cap,
  })
  return role
end

local function events_named(world, name)
  local out = {}
  for i = 1, #world.events do
    if world.events[i].name == name then
      out[#out + 1] = world.events[i]
    end
  end
  return out
end

--- 一次 `grant` 之后把记录解码出来（载荷必须是字符串这件事另有用例钉）。
local function record_of(world, id)
  local raw = world.kv["role.actor." .. id]
  if raw == nil then
    return nil
  end
  return codec.decode(raw)
end

-- ── 用例 1：KV 载荷形状（字符串 + 恰好三个键）────────────────────────────────

run("pkg.role-skill.kv-string-shape", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  is_true(role.grant(1, "warrior"), "grant 成功")

  local raw = world.kv["role.actor.1"]
  eq(type(raw), "string", "记录的载荷必须是字符串（KV 只收标量）")
  local payload = record_of(world, 1)
  eq_seq(sorted_keys(payload), { "counters", "role", "skills" }, "记录键集恰好三个")
  eq(payload.role, "warrior", "role 字段")
  eq_seq(sorted_keys(payload.skills), {}, "新记录没有技能槽位")
  eq_seq(sorted_keys(payload.counters), {}, "新记录没有计数")

  local index = codec.decode(world.kv["role.index"])
  eq(type(index), "table", "索引载荷也是字符串（解码后是表）")
  eq(index[1], 1, "索引第 1 项是 id 1")
  eq(#index, 1, "索引长度 1")
end)

-- ── 用例 2：授予幂等（I3）─────────────────────────────────────────────────────

run("pkg.role-skill.grant-idempotent", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", {})

  is_true(role.grant(1, "warrior"), "首次 grant")
  local writes = world.writes_now()
  is_true(role.grant(1, "warrior"), "再次 grant 仍返回 true（返回的是状态）")
  eq(world.writes_now(), writes, "幂等路径零写")
  eq(#events_named(world, "role.granted"), 1, "role.granted 只发一次")
  eq(#codec.decode(world.kv["role.index"]), 1, "索引只插入一次")
end)

-- ── 用例 3：撤销语义（未持有 false / 技能集不动）──────────────────────────────

run("pkg.role-skill.revoke-absent-false", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", {})

  eq(role.revoke(1, "warrior"), false, "未持有时 revoke 返回 false")
  eq(#world.events, 0, "未持有 ⇒ 零事件")

  is_true(role.grant(1, "warrior"), "grant")
  is_true(role.revoke(1, "warrior"), "持有时 revoke 返回 true")
  eq(role.has(1, "warrior"), false, "撤销后 has 为 false")
  eq(#events_named(world, "role.revoked"), 1, "role.revoked 发过一次")
  eq(role.revoke(1, "warrior"), false, "再撤销返回 false（记录保留、只清引用面）")
  is_true(record_of(world, 1) ~= nil, "记录未被删除（KV 无删除面）")
end)

-- ── 用例 4：未定义的角色 ─────────────────────────────────────────────────────

run("pkg.role-skill.grant-unknown-role-errors", function()
  local world = new_world()
  local role = fresh(world)
  expect_error("YGG-PKG-ROLE-UNKNOWN", function()
    role.grant(1, "nobody")
  end, "未定义角色")
  expect_error("YGG-PKG-ROLE-DEF", function()
    role.define("", {})
  end, "空角色名")
end)

-- ── 用例 5：引用面闸门（I7：只看 tag，不写不发）───────────────────────────────

run("pkg.role-skill.requires-tags-gate", function()
  local world = new_world()
  local role = fresh(world)
  role.define("mage", { requires_tags = { "awakened" } })

  local before = sorted_keys(world.kv)
  eq(role.grant(2, "mage"), false, "tag 不满足 ⇒ false")
  eq_seq(sorted_keys(world.kv), before, "被拒时 KV 键集合不变")
  eq(#world.events, 0, "被拒时零事件")

  world.add_tag(2, "awakened")
  is_true(role.grant(2, "mage"), "满足后可授予")
  eq(role.has(2, "mage"), true, "has 反映状态")
end)

-- ── 用例 6：引用建立即事件（升级不是建立）────────────────────────────────────

run("pkg.role-skill.skill-gain-once", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("fireball", { attrs_spec = { magic = 5 }, charges = 3 })

  is_true(role.grant_skill(1, "fireball"), "首次 grant_skill")
  eq(#events_named(world, "role.skill_gained"), 1, "首次发一次事件")
  local slot = record_of(world, 1).skills.fireball
  eq(slot.rank, 1, "缺省 rank = 1")
  eq(slot.charges, 3, "按声明充满")

  is_true(role.grant_skill(1, "fireball", 4), "带新 rank 再授一次")
  eq(#events_named(world, "role.skill_gained"), 1, "事件仍 1 次（升级不是建立）")
  eq(record_of(world, 1).skills.fireball.rank, 4, "rank 已更新")

  is_true(role.grant_skill(1, "fireball"), "缺省 rank 再授")
  eq(record_of(world, 1).skills.fireball.rank, 4, "缺省保持原值")
end)

-- ── 用例 7：PP 语义（零不减负 / 无声明不参与）──────────────────────────────────

run("pkg.role-skill.charge-floor", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("flash", { charges = 2 })
  role.grant_skill(1, "flash")

  eq(role.charge(1, "flash"), 1, "2 → 1")
  eq(role.charge(1, "flash"), 0, "1 → 0")
  eq(role.charge(1, "flash"), 0, "0 不再减负")

  role.skill_define("passive", {})
  role.grant_skill(1, "passive")
  local writes = world.writes_now()
  eq(role.charge(1, "passive"), 0, "未声明 charges ⇒ 0")
  eq(world.writes_now(), writes, "未声明 charges ⇒ 不写")
  eq(role.charge(1, "nope"), 0, "无槽位 ⇒ 0")
end)

-- ── 用例 8：充能上限取声明值 ─────────────────────────────────────────────────

run("pkg.role-skill.recharge-cap", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("flash", { charges = 3 })
  role.grant_skill(1, "flash")

  role.charge(1, "flash")
  eq(role.recharge(1, "flash"), 3, "无参 ⇒ 充满")
  role.charge(1, "flash")
  role.charge(1, "flash")
  role.charge(1, "flash")
  eq(role.recharge(1, "flash", 1), 1, "给 1 就加 1")
  eq(role.recharge(1, "flash", 99), 3, "夹到声明上限")

  role.skill_define("passive", {})
  role.grant_skill(1, "passive")
  eq(role.recharge(1, "passive", 5), 0, "未声明 charges ⇒ 0")
end)

-- ── 用例 9：合成而非写回（I2，本包最重要的一条）──────────────────────────────

run("pkg.role-skill.compose-no-writeback", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  role.skill_define("power", { attrs_spec = { attack = 2 } })
  role.skill_define("fury", { attrs_spec = { attack = 4 } })
  world.set_attr(1, "attack", 10)

  is_true(role.grant(1, "warrior"), "grant")
  is_true(role.grant_skill(1, "power"), "grant_skill power")
  is_true(role.grant_skill(1, "fury"), "grant_skill fury")

  eq(role.effective(1, "attack"), 19, "10 + 3 + 2 + 4")
  eq(world.entity.get(1, "attack"), 10, "基础值未被写回（I2）")

  is_true(role.revoke_skill(1, "fury"), "卸下 fury")
  eq(role.effective(1, "attack"), 15, "10 + 3 + 2（卸下即还原，无需撤销历史）")
  eq(world.entity.get(1, "attack"), 10, "基础值仍未变")
end)

-- ── 用例 10：自定义合成规则收到的加成顺序 ─────────────────────────────────────

run("pkg.role-skill.compose-custom-order", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  role.skill_define("zeta", { attrs_spec = { attack = 7 } })
  role.skill_define("alpha", { attrs_spec = { attack = 5 } })
  role.skill_define("mid", { attrs_spec = { attack = 6 } })
  world.set_attr(1, "attack", 10)

  role.grant(1, "warrior")
  -- 授予顺序刻意打乱：合成顺序由**名字**决定，不由授予顺序（否则两个游戏的同一套
  -- 技能会因装配顺序不同而算出不同加成）。
  role.grant_skill(1, "zeta")
  role.grant_skill(1, "alpha")
  role.grant_skill(1, "mid")

  local seen = nil
  is_true(role.compose(function(base, bonuses)
    local parts = { tostring(base) }
    for i = 1, #bonuses do
      parts[#parts + 1] = tostring(bonuses[i])
    end
    seen = table.concat(parts, ",")
    return base
  end), "compose 装自定义规则")

  eq(role.effective(1, "attack"), 10, "自定义规则原样返回 base")
  eq(seen, "10,3,5,6,7", "顺序 = base、角色加成、技能名升序（alpha 5 / mid 6 / zeta 7）")
  expect_error("YGG-PKG-ROLE-COMPOSE", function()
    role.compose(42)
  end, "非函数合成规则")
end)

-- ── 用例 11：属性缺失记 0（不是 nil，不报错）──────────────────────────────────

run("pkg.role-skill.effective-missing-attr-zero", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  role.skill_define("power", { attrs_spec = { magic = 5 } })
  role.skill_define("fury", { attrs_spec = { magic = 2 } })
  role.grant(1, "warrior")
  role.grant_skill(1, "power")
  role.grant_skill(1, "fury")

  eq(world.entity.get(1, "magic"), nil, "布置：实体上没有 magic")
  eq(role.effective(1, "magic"), 7, "缺失属性 base 记 0，加成照加（5 + 2）")
  eq(role.effective(1, "luck"), 0, "无基础无加成 ⇒ 0")
  eq(role.effective(99, "attack"), 0, "无记录的实体 ⇒ 0")
end)

-- ── 用例 12：计数器只增（订阅面 + 选择器两种形态）────────────────────────────

run("pkg.role-skill.counters-increment", function()
  local world = new_world()
  local role = fresh(world, {
    tick = function(payload)
      return payload.who
    end,
    turn = "entity",
  })
  role.define("warrior", {})
  role.grant(1, "warrior")
  role.grant(2, "warrior")

  is_true(type(world.handlers.turn) == "function", "turn 已订阅")
  is_true(type(world.handlers.tick) == "function", "tick 已订阅")

  for _ = 1, 3 do
    world.handlers.turn({ entity = 1 })
  end
  eq(role.count(1, "turn"), 3, "三次 turn ⇒ 3")
  eq(role.count(2, "turn"), 0, "实体 2 未被计数")
  eq(role.count(1, "never"), 0, "未订阅的事件名 ⇒ 0")

  -- 解析不出 id（payload 里没有那个字段）⇒ 不写
  local writes = world.writes_now()
  world.handlers.turn({ other = 1 })
  eq(world.writes_now(), writes, "id 为 nil ⇒ 不写")
  eq(role.count(1, "turn"), 3, "计数不变")

  -- 函数选择器读的是 payload 里的自定义字段
  world.handlers.tick({ who = 2 })
  eq(role.count(2, "tick"), 1, "函数选择器解析出的 id")

  -- 没有记录的实体：统计事件不凭空建记录
  world.handlers.turn({ entity = 99 })
  eq(world.kv["role.actor.99"], nil, "没有记录就不建记录")
end)

-- ── 用例 13：显式清零（带 event / 不带 event）────────────────────────────────

run("pkg.role-skill.counters-reset", function()
  local world = new_world()
  local role = fresh(world, { turn = "entity", tick = "entity" })
  role.define("warrior", {})
  role.grant(1, "warrior")
  world.handlers.turn({ entity = 1 })
  world.handlers.turn({ entity = 1 })
  world.handlers.tick({ entity = 1 })

  is_true(role.reset_counters(1, "turn"), "带 event 清零")
  eq(role.count(1, "turn"), 0, "turn 归零")
  eq(role.count(1, "tick"), 1, "别的计数不动")

  -- 已经是 0：返回 true 且不写
  local writes = world.writes_now()
  is_true(role.reset_counters(1, "turn"), "零值再清仍 true")
  eq(world.writes_now(), writes, "零值再清不写")

  is_true(role.reset_counters(1), "不带 event 清全部")
  eq(role.count(1, "tick"), 0, "tick 也归零")
  eq(#sorted_keys(record_of(world, 1).counters), 0, "counters 清空")

  -- 无记录：视为已清零，不写
  local before = sorted_keys(world.kv)
  is_true(role.reset_counters(99), "无记录返回 true")
  eq_seq(sorted_keys(world.kv), before, "无记录不写")
end)

-- ── 用例 14：静态面（零时间源、零属性写回面）──────────────────────────────────

--- 包的源文件清单：来自 manifest 的 packages 段（内容寻址，天然排除 test/**）。
local function pkg_sources(id)
  local manifest = rawget(_G, "__MANIFEST")
  if type(manifest) ~= "table" or type(manifest.packages) ~= "table" then
    error("manifest 缺 packages 段（载体 pack.json 未声明包或未重算 manifest）", 0)
  end
  for i = 1, #manifest.packages do
    local package = manifest.packages[i]
    if package.id == id then
      local out = {}
      for j = 1, #package.files do
        out[#out + 1] = package.files[j].path
      end
      return out
    end
  end
  error("manifest 的 packages 段没有包 " .. id, 0)
end

local function source_text(rel)
  local full = PKG_ROOT .. "/" .. rel
  local handle = assert(io.open(full, "rb"), "读不到 " .. full)
  local text = handle:read("a")
  handle:close()
  return text
end

run("pkg.role-skill.static-no-clock", function()
  local paths = pkg_sources("role-skill")
  is_true(#paths >= 7, "manifest 里 role-skill 源文件 ≥ 7，实得 " .. #paths)
  local needles = {
    "ygg.ticks",
    "os.clock",
    "os.time",
    "math.random",
    "set_attribute",
    "entity.set(",
  }
  local problems = {}
  for i = 1, #paths do
    local text = source_text(paths[i])
    for j = 1, #needles do
      if text:find(needles[j], 1, true) ~= nil then
        problems[#problems + 1] = paths[i] .. " 含 " .. needles[j]
      end
    end
  end
  is_true(#problems == 0, table.concat(problems, "; "))
end)

-- ── 用例 15：100 轮授予/撤销不增长（索引只增，技能集回零）────────────────────

run("pkg.role-skill.leak-100-rounds", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  is_true(role.grant(1, "warrior"), "grant")
  for i = 1, 6 do
    role.skill_define("skill" .. i, { attrs_spec = { attack = i } })
  end

  local index_before = #codec.decode(world.kv["role.index"])
  local keys_before = #sorted_keys(world.kv)
  collectgarbage("collect")
  collectgarbage("collect")
  local before = collectgarbage("count")

  for _ = 1, 100 do
    for i = 1, 6 do
      role.grant_skill(1, "skill" .. i)
    end
    for i = 1, 6 do
      role.revoke_skill(1, "skill" .. i)
    end
    world.clear_events()
  end

  collectgarbage("collect")
  collectgarbage("collect")
  local growth = (collectgarbage("count") - before) / before * 100
  print(string.format("perf.role-skill.leak_delta_pct=%.3f", growth))

  eq_seq(sorted_keys(record_of(world, 1).skills), {}, "技能集回零")
  eq(#codec.decode(world.kv["role.index"]), index_before, "索引条目数不变（只增不摘）")
  eq(#sorted_keys(world.kv), keys_before, "KV 键数不变")
  is_true(growth <= 1, "100 轮后内存差 ≤ 1%，实得 " .. string.format("%.3f", growth) .. "%")
end)

-- ── 用例 16：技能集读面（升序名字数组 + 副本语义 + 空集边界）──────────────────

run("pkg.role-skill.read-skills-of", function()
  local world = new_world()
  local role = fresh(world)

  eq_seq(role.skills_of(1), {}, "无记录 ⇒ 空数组（不是 nil）")
  role.skill_define("zeta", { attrs_spec = {} })
  role.skill_define("alpha", { attrs_spec = {} })
  role.skill_define("mid", { attrs_spec = {} })
  role.define("warrior", { attrs_spec = {} })
  role.grant(1, "warrior")
  eq_seq(role.skills_of(1), {}, "有记录但无槽位 ⇒ 仍是空数组")

  -- 授予顺序与名字顺序故意相反：结果是升序，与装配顺序无关
  role.grant_skill(1, "zeta")
  role.grant_skill(1, "mid")
  role.grant_skill(1, "alpha")
  eq_seq(role.skills_of(1), { "alpha", "mid", "zeta" }, "升序名字数组")

  -- 副本语义：读面不能被调用方当成旁路写面
  local snapshot = role.skills_of(1)
  snapshot[1] = "tampered"
  snapshot[#snapshot + 1] = "extra"
  eq_seq(role.skills_of(1), { "alpha", "mid", "zeta" }, "改返回值不影响存储")

  role.revoke_skill(1, "mid")
  eq_seq(role.skills_of(1), { "alpha", "zeta" }, "撤销后从集合消失")

  -- `rank = 0` 的槽位也在：槽位存在与否是写面的事实，筛等级是 mod 的规则（I7）
  role.skill_define("zero", { attrs_spec = {} })
  role.grant_skill(1, "zero", 0)
  eq_seq(role.skills_of(1), { "alpha", "zero", "zeta" }, "rank=0 的槽位也在")
end)

-- ── 用例 17：单槽读面（字段只映槽位 + nil 边界 + 副本）─────────────────────────

run("pkg.role-skill.read-skill-state", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("fire", { attrs_spec = {}, charges = 7 })
  role.skill_define("plain", { attrs_spec = {} })
  role.grant_skill(1, "fire", 3)
  role.grant_skill(1, "plain")

  local fire = role.skill_state(1, "fire")
  eq(fire.rank, 3, "rank 与授予值相等")
  eq(fire.charges, 7, "声明了 charges ⇒ 槽位带 charges")
  eq(fire.capacity, nil, "未设容量 ⇒ 没有 capacity 字段")
  eq(role.skill_state(1, "plain").charges, nil, "未声明 charges ⇒ 没有 charges 字段")
  eq(role.skill_state(1, "nope"), nil, "无槽位 ⇒ nil（不是空表）")
  eq(role.skill_state(99, "fire"), nil, "无记录 ⇒ nil")
  expect_error(role.ERRORS.DEF, function()
    role.skill_state(1, "")
  end, "空技能名 ⇒ -DEF")

  local copy = role.skill_state(1, "fire")
  copy.rank = 99
  copy.charges = 0
  eq(role.skill_state(1, "fire").rank, 3, "改副本的 rank 不回写")
  eq(role.skill_state(1, "fire").charges, 7, "改副本的 charges 不回写")

  eq(role.set_capacity(1, "fire", 12), 12, "设容量")
  eq(role.skill_state(1, "fire").capacity, 12, "设容量后带 capacity")
  eq(role.skill_state(1, "fire").charges, 7, "放大不补满：余量仍是 7")
end)

-- ── 用例 18：实例容量放大（声明的 charges 不再是天花板）───────────────────────

run("pkg.role-skill.capacity-grow", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role.grant_skill(1, "pp")
  eq(role.skill_state(1, "pp").charges, 30, "建槽位时充满到声明的 30")

  eq(role.set_capacity(1, "pp", 60), 60, "set_capacity 返回生效后的容量")
  eq(role.skill_state(1, "pp").charges, 30, "放大不补满")
  eq(role.recharge(1, "pp"), 60, "recharge 充满到新上限")

  local left = nil
  for _ = 1, 60 do
    left = role.charge(1, "pp")
  end
  eq(left, 0, "连调 60 次才到 0（声明的 30 不再是天花板）")
  eq(role.charge(1, "pp"), 0, "0 不减负")
end)

-- ── 用例 19：实例容量缩容（余量夹紧 + 上限换成新值）───────────────────────────

run("pkg.role-skill.capacity-shrink", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role.grant_skill(1, "pp")

  eq(role.set_capacity(1, "pp", 10), 10, "缩容到 10")
  eq(role.skill_state(1, "pp").charges, 10, "余量夹紧到 10")
  for _ = 1, 10 do
    role.charge(1, "pp")
  end
  eq(role.charge(1, "pp"), 0, "打光后仍是 0（不减负）")
  eq(role.skill_state(1, "pp").charges, 0, "余量 0")
  eq(role.recharge(1, "pp"), 10, "回到新上限（不是声明的 30）")
end)

-- ── 用例 20：容量面的错误码与「未声明 charges」技能的下场 ──────────────────────

run("pkg.role-skill.capacity-errors", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("bare", { attrs_spec = {} })
  role.grant_skill(1, "bare")

  eq(role.charge(1, "bare"), 0, "未声明 charges 且无容量 ⇒ 不参与充能")
  eq(role.recharge(1, "bare"), 0, "同上（recharge 返回 0 且不写）")
  expect_error(role.ERRORS.UNKNOWN, function()
    role.set_capacity(1, "nope", 5)
  end, "无槽位 ⇒ -UNKNOWN")
  expect_error(role.ERRORS.UNKNOWN, function()
    role.set_capacity(99, "bare", 5)
  end, "无记录 ⇒ -UNKNOWN")
  expect_error(role.ERRORS.DEF, function()
    role.set_capacity(1, "bare", -1)
  end, "负 cap ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.set_capacity(1, "bare", 1.5)
  end, "小数 cap ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.set_capacity(1, "bare", "3")
  end, "字符串 cap ⇒ -DEF")

  eq(role.set_capacity(1, "bare", 3), 3, "未声明 charges 的技能也能设容量")
  eq(role.skill_state(1, "bare").charges, 3, "余量以 cap 初始化（与建槽位时充满同口径）")
  eq(role.charge(1, "bare"), 2, "设了容量就开始参与充能（D8 只在两者皆无时生效）")
end)

-- ── 用例 21：recharge 第 4 参（持久化容量；3 参路径逐字节同前）─────────────────

run("pkg.role-skill.capacity-recharge-cap", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role.grant_skill(1, "pp")

  -- 3 参路径：与本轮前的语义逐值相等，且不写 capacity 字段
  eq(role.recharge(1, "pp"), 30, "3 参：填到声明的 30")
  is_true(
    world.kv["role.actor.1"]:find("capacity", 1, true) == nil,
    "3 参路径的载荷不含 capacity 字段"
  )
  eq(role.recharge(1, "pp", 5), 30, "已满再充仍夹紧在 30")
  role.charge(1, "pp")
  eq(role.recharge(1, "pp", 2), 30, "n=2 充回上限")

  -- 第 4 参 = 持久化新容量（不是「临时上限」——那会造出第二个真源）
  eq(role.recharge(1, "pp", 1, 40), 31, "4 参：余量 30+1，容量同时换成 40")
  eq(role.skill_state(1, "pp").capacity, 40, "4 参把容量落进槽位")
  is_true(
    world.kv["role.actor.1"]:find("capacity", 1, true) ~= nil,
    "载荷里确实出现 capacity"
  )
  eq(role.recharge(1, "pp"), 40, "之后无参 recharge 以新容量为上限")

  -- 对照世界：只走 3 参 ⇒ 没有容量、上限仍是声明值
  local world2 = new_world()
  local role2 = fresh(world2)
  role2.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role2.grant_skill(1, "pp")
  eq(role2.recharge(1, "pp"), 30, "对照世界 3 参仍是 30")
  eq(role2.skill_state(1, "pp").capacity, nil, "对照世界没有 capacity")
end)

-- ── 用例 27：一次 effective 的读次数与实体总数无关（O(1) 的结构证据）───────────

run("pkg.role-skill.perf-o1-structural", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  for i = 1, 6 do
    role.skill_define("skill" .. i, { attrs_spec = { attack = 1 } })
  end

  --- 直接铺记录：这里测的是 `effective` 的复杂度（与实体**总数**无关）与载荷常驻
  --- 内存。`grant` 走 KV 时每建一个记录要重写整张 `role.index`（「一个键装整张索引」
  --- 的固有成本），拿它当 O(1) 的探针会把两件事混在一起——那属于契约 §5 已声明的
  --- 边界，不是 `effective` 的形状。
  local function seed(count, first)
    for id = first or 1, count do
      local skills = {}
      for i = 1, 6 do
        skills["skill" .. i] = { rank = 1 }
      end
      world.kv["role.actor." .. id] = codec.encode({
        counters = {},
        role = "warrior",
        skills = skills,
      })
      world.set_attr(id, "attack", 10)
    end
  end

  -- ① 1k actor（各 1 角色 + 6 技能）的载荷常驻内存
  collectgarbage("collect")
  collectgarbage("collect")
  local before_mem = collectgarbage("count")
  seed(1000)
  -- 读之前显式回收：`collectgarbage("count")` 含**未回收**的垃圾，而 seed 里每次
  -- `codec.encode` 都产一张临时表 —— 不回收时这个数随进程 GC 步进漂移（S8 实测同一
  -- 命令三次得 505.7 / 714.4 / 1098.4 KiB；普通 Lua 对照 live 面：老包 333.3 / 新包
  -- 332.6 KiB，差 0.2%）。判据量的是「载荷常驻」，不是「这次恰好留了多少垃圾」。
  collectgarbage("collect")
  collectgarbage("collect")
  local mem_kib = collectgarbage("count") - before_mem
  print(string.format("perf.role-skill.mem_1k_actors_kib=%.1f", mem_kib))
  is_true(mem_kib <= 1024, "1k actor 载荷 ≤ 1 MiB，实得 " .. string.format("%.1f", mem_kib) .. " KiB")

  -- ② 三档读次数与墙钟
  local function measure(count)
    seed(count)
    local reads_before = world.reads_now()
    local sample = role.effective(1, "attack")
    local ops = world.reads_now() - reads_before
    local began = os.clock()
    local total = 0
    for _ = 1, 200 do
      total = total + role.effective(1, "attack")
    end
    local per_call = (os.clock() - began) / 200
    return ops, per_call, sample, total
  end

  seed(1)
  local ops_one, time_one, sample, total = measure(1)
  local ops_hundred = measure(100)
  local ops_tenk, time_tenk = measure(10000)

  eq(ops_one, 3, "一次 effective 恰好三次 kv.get")
  eq(ops_hundred, ops_one, "N=100 同")
  eq(ops_tenk, ops_one, "N=10k 同")
  eq(sample, 19, "10 + 3 + 6×1（角色 + 六个技能的加成）")
  is_true(total > 0, "累加参与（不让 200 次调用被当成不可观测）")
  print(string.format("perf.role-skill.kv_ops_per_effective=%d", ops_one))

  local ratio = time_tenk / time_one
  print(string.format("perf.role-skill.time_ratio_10k_over_1=%.3f", ratio))
  -- 1.55 而不是 1.5：P-K r1 D-8 在同一台机器上两连同命令实测 1.46 / 1.50，判据压着
  -- 观测红值（S13）；余量按红值 +3.3% 取 1.55。O(N) 回归在本仪器上是 ~10×，判据仍挡得住。
  -- 1.2 更不行：GC 脉冲会被打成假红（r1 在 quest 的同类指标上实证，见 quest.md §5）。
  is_true(ratio <= 1.55, "墙钟比 ≤ 1.55×，实得 " .. string.format("%.3f", ratio))

end)

-- ── 用例 28：包加载（清缓存 = 定义表与订阅面一起归零）────────────────────────

run("pkg.role-skill.perf-load", function()
  local samples = {}
  for round = 1, 5 do
    package.loaded["pkg.role_skill"] = nil
    for name in pairs(package.loaded) do
      if type(name) == "string" and name:find("^pkg%.role_skill%.") then
        package.loaded[name] = nil
      end
    end
    local began = os.clock()
    local module = require("pkg.role_skill")
    module.install({ codec = codec })
    samples[round] = (os.clock() - began) * 1000
  end
  table.sort(samples)
  local median = samples[3]
  print(string.format("perf.role-skill.load_ms=%.3f", median))
  is_true(median < 10, "包加载 < 10ms，实得 " .. string.format("%.3f", median) .. "ms")
end)
