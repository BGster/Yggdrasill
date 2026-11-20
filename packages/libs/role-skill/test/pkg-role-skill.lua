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
  world.reads_now = function()
    return reads
  end
  world.writes_now = function()
    return writes
  end
  return world
end

--- 取一份**全新**的包实例并装上 mock（清缓存 = 定义表 / 索引缓存 / 订阅表归零）。
local function fresh(world)
  package.loaded["pkg.role_skill"] = nil
  for name in pairs(package.loaded) do
    if type(name) == "string" and name:find("^pkg%.role_skill%.") then
      package.loaded[name] = nil
    end
  end
  local role = require("pkg.role_skill")
  role.install({ host = world.host, entity = world.entity, codec = codec })
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
