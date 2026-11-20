-- `pkg.quest.*` 自测（20 条行为用例；perf 在 `pkg-quest-perf.lua`，同样计数）。
--
-- # 为什么自带 mock 而不复用 mod 的 harness
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）。这里就地构造一个**语义等价**的
-- `ygg` 宿主面，四条都是内核的**真实行为**（照抄 `crates/ygg-sandbox/src/host_api.rs`）：
--
--   1. `kv.set` 只收四个标量型，表写入即 `unsupported payload type: table`；
--   2. `kv.get` 每次经 serde_json 重新编组 ⇒ 拿到的**不是**同一张表（标量无所谓，
--      但 mock 仍按「不共享引用」写，免得将来有人往里塞表时用例假绿）；
--   3. `ygg.emit` 的载荷同样只收标量（`value_from_lua` 同一份实现）；
--   4. `ygg.ticks()` 有 API 无推进者（恒 0）——本包**一次都不该读**，q17 用调用计数钉。
--
-- mock 若不允许这两条「只收标量」的拒绝，本包最核心的一条形状约束（扁平标量键）
-- 就永远测不到：把表塞进 KV 的实现在这里绿、在真实跑局红，正是 E6 那类假绿的形状。
--
-- # 隔离口径
--
-- 每条用例 `new_world()` 一次：新建 mock + 清 `package.loaded` 重新 require
-- （`install` 记的 host 归零）。用例之间不共享任何状态，失败不会串味。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-quest.lua`，`--filter pkg-quest` 只跑本包。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function check(name, ok, detail)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. (detail and (": " .. tostring(detail)) or ""))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected or math.type(actual) ~= math.type(expected) then
    error(
      ("%s: expected %s (%s), got %s (%s)"):format(
        label or "eq",
        tostring(expected),
        tostring(math.type(expected)),
        tostring(actual),
        tostring(math.type(actual))
      ),
      2
    )
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy, got " .. tostring(value), 2)
  end
end

local function is_nil(value, label)
  if value ~= nil then
    error((label or "is_nil") .. ": expected nil, got " .. tostring(value), 2)
  end
end

local function expect_error(code, fn, label)
  local ok, err = pcall(fn)
  if ok then
    error((label or "expect_error") .. ": expected " .. code .. ", but the call succeeded", 2)
  end
  if not tostring(err):find(code, 1, true) then
    error(("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)), 2)
  end
end

local function run(name, fn)
  local ok, err = pcall(fn)
  check(name, ok, ok and nil or err)
end

-- ── mock 宿主面（语义对齐内核；见文件头注）───────────────────────────────────

--- 新建一个世界：mock `ygg` + 观测面（写/读计数、KV、事件流、外部面调用计数）。
local function new_world()
  local kv = {}
  local writes = 0
  local reads = 0
  local events = {}
  local entity_calls = 0
  local ticks_calls = 0
  local world = { kv = kv, events = events, tick = 0 }

  local function scalar_or_fail(kind, value)
    if type(value) == "table" or type(value) == "function" then
      error("unsupported payload type: " .. type(value), 0)
    end
    return kind, value
  end

  -- `ygg.entity.*` 一律计数：契约 I4 说本包「只把 giver/taker 当 ref 校验」，
  -- 一次调用都不该发生（q13 断 = 0）。这里不实现语义，只记「有没有被碰」。
  local entity = {}
  for _, name in ipairs({ "spawn", "destroy", "exists", "get_attribute", "set_attribute", "has_tag" }) do
    entity[name] = function()
      entity_calls = entity_calls + 1
      error("quest 不该调用 ygg.entity." .. name, 0)
    end
  end

  world.host = {
    kv = {
      get = function(key)
        reads = reads + 1
        return kv[key]
      end,
      set = function(key, value)
        scalar_or_fail("kv", value)
        writes = writes + 1
        kv[key] = value
      end,
    },
    emit = function(name, payload)
      if type(name) ~= "string" then
        error("emit 的事件名必须是字符串", 0)
      end
      scalar_or_fail("emit", payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
    entity = entity,
    ticks = function()
      ticks_calls = ticks_calls + 1
      return world.tick
    end,
  }

  world.writes_now = function()
    return writes
  end
  world.reads_now = function()
    return reads
  end
  world.entity_calls = function()
    return entity_calls
  end
  world.ticks_calls = function()
    return ticks_calls
  end
  return world
end

--- 取一份**全新**的模块实例并装上 mock（清缓存 = `install` 记的 host 归零）。
local function fresh(world)
  for _, name in ipairs({
    "pkg.quest",
    "pkg.quest.store",
    "pkg.quest.types",
    "pkg.quest.flow",
  }) do
    package.loaded[name] = nil
  end
  local quest = require("pkg.quest")
  quest.install({ host = world.host })
  return quest
end

--- 升序键列表（测试里允许 `table.sort`：这里跑的是普通 Lua，不是沙箱）。
local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

--- 事件流里按名字筛（保序）。
local function events_named(world, name)
  local out = {}
  for i = 1, #world.events do
    if world.events[i].name == name then
      out[#out + 1] = world.events[i]
    end
  end
  return out
end

--- 事件名数组（保序）——「名字 + 次数」的精确断言用它，不用「包含」。
local function event_names(world)
  local out = {}
  for i = 1, #world.events do
    out[#out + 1] = world.events[i].name
  end
  return out
end

local function names_equal(actual, expected, label)
  eq(#actual, #expected, (label or "names_equal") .. " 数量")
  for i = 1, #expected do
    eq(actual[i], expected[i], (label or "names_equal") .. "[" .. i .. "]")
  end
end

--- 一个三步骤 + 两个 meta 键的标准流程（多数用例共用）。
local function errand_spec(steps)
  return {
    steps = steps or {
      { id = "accept", guide = "quests.errand.accept" },
      { id = "deliver", guide = "quests.errand.deliver", on_enter = "quest.check.deliver" },
      { id = "reward", guide = "quests.errand.reward" },
    },
    meta_spec = { source = "string", weight = "number", urgent = "boolean", npc = "ref" },
  }
end

-- ── 用例 1-5：定义与推进骨架 ────────────────────────────────────────────────

run("pkg.quest.define-idempotent", function()
  local world = new_world()
  local quest = fresh(world)
  local spec = errand_spec()

  is_true(quest.define("errand", spec), "首次 define 必须返回 true")
  eq(world.kv["quest.types"], "errand", "类型名表")
  local writes_after_first = world.writes_now()
  local bytes_after_first = world.kv["quest.types"]

  is_true(quest.define("errand", spec), "同 spec 二次 define 必须返回 true")
  eq(world.writes_now(), writes_after_first, "同 spec 二次 define 必须是零写")
  eq(world.kv["quest.types"], bytes_after_first, "类型名表字节不变")

  -- 异 spec 不覆盖：换一步、换一个 meta 类型，两种都必须被拒。
  expect_error("YGG-PKG-QUEST-TYPE", function()
    quest.define("errand", errand_spec({
      { id = "accept", guide = "quests.errand.accept" },
      { id = "deliver", guide = "quests.errand.deliver" },
    }))
  end, "异 steps 覆盖被拒")
  expect_error("YGG-PKG-QUEST-TYPE", function()
    quest.define("errand", {
      steps = { { id = "accept", guide = "quests.errand.accept" } },
      meta_spec = { weight = "string" },
    })
  end, "异 meta_spec 覆盖被拒")
  eq(world.kv["quest.types"], bytes_after_first, "被拒的 define 不改类型名表")
  eq(quest.type("errand").meta_spec.weight, "number", "被拒的 define 不改 meta 类型词")

  -- 名字与模板形状的拒绝路径（断错误码，不断「没报错」）。
  expect_error("YGG-PKG-QUEST-NAME", function()
    quest.define("Errand", spec)
  end, "大写开头的类型名被拒")
  expect_error("YGG-PKG-QUEST-TEMPLATE", function()
    quest.define("empty", { steps = {} })
  end, "空 steps 被拒")
  expect_error("YGG-PKG-QUEST-TEMPLATE", function()
    quest.define("dup", {
      steps = { { id = "same" }, { id = "same" } },
    })
  end, "类型内重复步 id 被拒")
  expect_error("YGG-PKG-QUEST-TEMPLATE", function()
    quest.define("bogus", {
      steps = { { id = "one" } },
      meta_spec = { weird = "table" },
    })
  end, "词表外的 meta 类型被拒")
end)

run("pkg.quest.template-not-inherited", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")
  eq(quest.get(id).step, 1, "新实例从第 1 步起")

  -- I1：实例只存 step 下标，不复制步骤内容——KV 里没有 `guide` / `on_enter` 的副本。
  local inst_keys = {}
  for key in pairs(world.kv) do
    if key:find("quest.inst." .. id .. ".", 1, true) == 1 then
      inst_keys[#inst_keys + 1] = key
    end
  end
  table.sort(inst_keys)
  names_equal(inst_keys, {
    "quest.inst." .. id .. ".giver",
    "quest.inst." .. id .. ".state",
    "quest.inst." .. id .. ".step",
    "quest.inst." .. id .. ".taker",
    "quest.inst." .. id .. ".type",
  }, "实例键集")

  local before = {}
  for i = 1, #inst_keys do
    before[inst_keys[i]] = world.kv[inst_keys[i]]
  end

  -- 模拟「有人改了数据里的模板」：加一步、改第 1 步的指引键。实例键必须一字不动。
  world.kv["quest.t.errand.steps"] = 4
  world.kv["quest.t.errand.s.4.id"] = "epilogue"
  world.kv["quest.t.errand.s.4.guide"] = "quests.errand.epilogue"
  world.kv["quest.t.errand.s.4.on_enter"] = ""
  world.kv["quest.t.errand.s.1.guide"] = "quests.errand.accept.v2"

  for i = 1, #inst_keys do
    eq(world.kv[inst_keys[i]], before[inst_keys[i]], "实例键未随模板改动：" .. inst_keys[i])
  end
  -- 而读数按新模板解释——「同一份存档在新模板下的进度」由此可判。
  eq(quest.step(id).guide, "quests.errand.accept.v2", "step().guide 来自模板")
  eq(quest.get(id).step, 1, "实例自己的步下标没变")
end)

run("pkg.quest.id-monotonic", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  local first = quest.open("errand")
  local second = quest.open("errand")
  local third = quest.open("errand")
  eq(first, 1, "首个 id")
  eq(second, 2, "第二个 id")
  eq(third, 3, "第三个 id")
  is_true(second > first and third > second, "id 严格递增")
  eq(world.kv["quest.seq"], 3, "seq = 已分配的最大 id")

  -- 放弃不回收 id（I1：只增不减、不复用）。
  quest.abandon(second, "dropped")
  eq(quest.get(second).state, "abandoned", "放弃后的状态")
  eq(quest.open("errand"), 4, "放弃后新开的 id 继续递增")
  eq(world.kv["quest.seq"], 4, "seq 随之推进")
  eq(quest.get(1).state, "open", "放弃别的任务不影响它")
end)

run("pkg.quest.advance-monotonic", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")

  -- I2：每次恰好 +1，返回值是推进后的 state。
  eq(quest.advance(id), "open", "非末步 advance 返回 open")
  eq(quest.get(id).step, 2, "advance 后步下标")
  eq(quest.advance(id), "open", "第二步 advance 返回 open")
  eq(quest.get(id).step, 3, "第二步后的步下标")

  -- 「无传步号入口」：advance 只收 id，多给的实参不改变推进量。
  local fourth = quest.open("errand")
  quest.advance(fourth, 99)
  eq(quest.get(fourth).step, 2, "advance 不接受步号实参（只 +1）")

  -- 步下标是 1 基，且 step() 的 index 与实例一致。
  eq(quest.step(fourth).index, 2, "step().index")
  eq(quest.step(fourth).id, "deliver", "step().id 来自模板")
end)

run("pkg.quest.advance-to-done", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("short", {
    steps = { { id = "one" }, { id = "two" } },
  })
  local id = quest.open("short")

  eq(quest.advance(id), "open", "第一步 advance")
  eq(quest.advance(id), "done", "末步 advance 转 done")
  eq(quest.get(id).state, "done", "实例状态落盘")

  -- 事件名恰好这三个（顺序也钉）。
  names_equal(event_names(world), {
    "quest.opened",
    "quest.advanced",
    "quest.completed",
  }, "事件序")
  eq(#events_named(world, "quest.completed"), 1, "completed 恰好一次")
  eq(events_named(world, "quest.completed")[1].payload, tostring(id), "completed 载荷 = id")
end)

-- ── 用例 6-12：终态、跳步与事件面 ────────────────────────────────────────────

run("pkg.quest.terminal-once", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("short", { steps = { { id = "one" }, { id = "two" } } })
  local id = quest.open("short")
  quest.advance(id)
  eq(quest.advance(id), "done", "到 done")
  local events_before = #world.events

  -- I3：终态后一切原语被拒，且**零事件**（重复设置终态会让奖励双发）。
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.advance(id)
  end, "done 后 advance")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.jump(id, "one")
  end, "done 后 jump")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.abandon(id, "late")
  end, "done 后 abandon")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.assign(id, 7, 8)
  end, "done 后 assign")
  eq(#world.events, events_before, "被拒的调用不得产生事件")
  eq(quest.get(id).state, "done", "状态不得被改写")
  eq(quest.get(id).step, 2, "步下标不得被改写")

  -- abandoned 同样是终态（两态各自独立判）。
  local other = quest.open("short")
  quest.abandon(other, "cancelled")
  expect_error("YGG-PKG-QUEST-TERMINAL", function()
    quest.advance(other)
  end, "abandoned 后 advance")
end)

run("pkg.quest.abandon", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")

  is_true(quest.abandon(id, "giver-left"), "abandon 返回 true")
  eq(quest.get(id).state, "abandoned", "状态落盘")
  eq(#events_named(world, "quest.abandoned"), 1, "quest.abandoned 恰好一次")
  eq(events_named(world, "quest.abandoned")[1].payload, id .. "|giver-left", "载荷 = id|reason")

  -- reason 缺省是空串（载荷形状仍定形，不出现 nil）。
  local second = quest.open("errand")
  quest.abandon(second)
  eq(events_named(world, "quest.abandoned")[2].payload, second .. "|", "缺省 reason 落空串")

  -- 放弃的任务不再出现在 open_ids。
  eq(#quest.open_ids(), 0, "全部任务已放弃")
end)

run("pkg.quest.jump-audit", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")

  is_true(quest.jump(id, "reward"), "jump 到第三步")
  eq(quest.get(id).step, 3, "步下标被显式改写")
  eq(quest.get(id).state, "open", "跳步不进终态")
  eq(#events_named(world, "quest.jumped"), 1, "quest.jumped 恰好一次")
  eq(events_named(world, "quest.jumped")[1].payload, id .. "|1|3", "载荷 = id|from|to")

  -- 跳步留痕后仍可继续推进（末步 advance → done）。
  eq(quest.advance(id), "done", "跳到末步后 advance 转 done")
end)

run("pkg.quest.jump-unknown-step", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())
  local id = quest.open("errand")
  local events_before = #world.events

  expect_error("YGG-PKG-QUEST-STEP", function()
    quest.jump(id, "no-such-step")
  end, "未知步 id")
  eq(quest.get(id).step, 1, "被拒的 jump 不改步下标")
  eq(#world.events, events_before, "被拒的 jump 不发事件")
  eq(#events_named(world, "quest.jumped"), 0, "没有 jumped 事件")
end)

run("pkg.quest.meta-not-in-flow", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  -- I5：meta 里塞满三个键，推进结果与不塞时逐字段相同。
  local with_meta = quest.open("errand", {
    meta = { source = "guild", weight = 3, urgent = true, npc = 42 },
  })
  local bare = quest.open("errand")
  eq(quest.step(with_meta).id, quest.step(bare).id, "起始步相同")
  eq(quest.advance(with_meta), quest.advance(bare), "第一次推进的 state 相同")
  eq(quest.get(with_meta).step, quest.get(bare).step, "推进后的步下标相同")
  eq(quest.get(with_meta).state, quest.get(bare).state, "推进后的 state 相同")
  eq(quest.step(with_meta).id, quest.step(bare).id, "推进后读到的步 id 相同")
  eq(quest.advance(with_meta), quest.advance(bare), "第二次推进的 state 相同")

  -- meta 值本身原样可读（它只是数据，不参与判定）。
  eq(quest.get(with_meta).meta.source, "guild", "string meta")
  eq(quest.get(with_meta).meta.weight, 3, "number meta")
  eq(quest.get(with_meta).meta.urgent, true, "boolean meta")
  eq(quest.get(with_meta).meta.npc, 42, "ref meta")

  -- 事件载荷只有 id 与步下标——不含任何 meta 字段。
  local advanced = events_named(world, "quest.advanced")[1]
  eq(advanced.payload, with_meta .. "|2", "advanced 载荷只带 id 与步下标")
end)

run("pkg.quest.meta-spec-gate", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  -- 未声明的键：拒绝（不是「忽略」——忽略会让存档里出现模板从不认识的字段）。
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { difficulty = 5 } })
  end, "未声明的 meta 键")
  -- 类型不符：四个词表逐一拒绝。
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { weight = "heavy" } })
  end, "number 位塞字符串")
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { urgent = 1 } })
  end, "boolean 位塞数字")
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { npc = 0 } })
  end, "ref 位塞 0")
  expect_error("YGG-PKG-QUEST-META", function()
    quest.open("errand", { meta = { npc = 2.5 } })
  end, "ref 位塞非整数")
  -- 被拒的 open 不分配 id（否则 seq 会被空转推进）。
  eq(world.kv["quest.seq"], nil, "被拒的 open 不写 seq")
  eq(#quest.open_ids(), 0, "被拒的 open 不产生实例")

  -- giver / taker 的 ref 校验走独立错误码。
  expect_error("YGG-PKG-QUEST-REF", function()
    quest.open("errand", { giver = -1 })
  end, "负 giver")
  expect_error("YGG-PKG-QUEST-REF", function()
    quest.open("errand", { taker = "7" })
  end, "字符串 taker")
end)

run("pkg.quest.events-exact", function()
  local world = new_world()
  local quest = fresh(world)
  quest.define("errand", errand_spec())

  local a = quest.open("errand")
  quest.advance(a)
  quest.jump(a, "reward")
  quest.advance(a)
  local b = quest.open("errand")
  quest.abandon(b, "nope")

  -- I6：「名字集合恰好相等」= 名字数组逐项相等（含次数与顺序），不是「包含」。
  names_equal(event_names(world), {
    "quest.opened",
    "quest.advanced",
    "quest.jumped",
    "quest.completed",
    "quest.opened",
    "quest.abandoned",
  }, "事件全序")
  eq(#events_named(world, "quest.opened"), 2, "opened 两次")
  eq(#events_named(world, "quest.advanced"), 1, "advanced 一次")
  eq(#events_named(world, "quest.jumped"), 1, "jumped 一次")
  eq(#events_named(world, "quest.completed"), 1, "completed 一次")
  eq(#events_named(world, "quest.abandoned"), 1, "abandoned 一次")

  -- 载荷全部是字符串（标量纪律），且 opened 的第二段就是类型名。
  for i = 1, #world.events do
    eq(type(world.events[i].payload), "string", "事件 " .. world.events[i].name .. " 的载荷是字符串")
  end
  eq(events_named(world, "quest.opened")[2].payload, b .. "|errand", "opened 载荷 = id|type")
end)
