-- `pkg.dialogue.*` 自测（22 条行为用例；perf 在 `pkg-dialogue-perf.lua`，同样计数）。
--
-- # 为什么自带 mock 而不复用 mod 的 harness
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）。这里就地构造一个**语义等价**的
-- `ygg` 宿主面，五条都是内核的**真实行为**（照抄 `crates/ygg-sandbox/src/host_api.rs`）：
--
--   1. `kv.set` 只收四个标量型（表写入即 `unsupported payload type: table`）；
--   2. `ygg.emit` 的载荷同样只收标量；
--   3. `ygg.tool.call(tool_id, args)` 的 `args` 只收**扁平标量**（`tool_args_from_lua`：
--      表的键必须是字符串、值必须是标量），返回值是
--      `{ ok, unmounted, code, data|error }`；
--   4. `ygg.on` 只在装载期可用（seal 后再注册即报错）——所以本包的桥必须在
--      `install` 里注册完，用例直接调 mock 记下的函数来模拟派发；
--   5. `ygg.ticks()` 有 API 无推进者——本包一次都不该读（d11 用调用计数钉）。
--
-- # 隔离口径
--
-- 每条用例 `new_world()` 一次：新建 mock + 清 `package.loaded` 重新 require
-- （`install` 记的 host / 配置 / 适配器归零）。用例之间不共享任何状态。

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

local MODULES = {
  "pkg.dialogue",
  "pkg.dialogue.store",
  "pkg.dialogue.handlers",
  "pkg.dialogue.dispatch",
  "pkg.dialogue.memory",
}

-- ── mock 宿主面（语义对齐内核；见文件头注）───────────────────────────────────

local function new_world()
  local kv = {}
  local writes = 0
  local events = {}
  local registrations = {}
  local tool_calls = {}
  local tool_replies = {}
  local entity_calls = 0
  local ticks_calls = 0
  local data_calls = 0
  local narrative_calls = 0
  local world = { kv = kv, events = events, tick = 0, tool_replies = tool_replies }

  local function reject_non_scalar(what, value)
    local kind = type(value)
    if kind == "table" or kind == "function" or kind == "nil" then
      error(what .. " must be a scalar, got " .. kind, 0)
    end
  end

  world.host = {
    kv = {
      get = function(key)
        return kv[key]
      end,
      set = function(key, value)
        if type(value) == "table" or type(value) == "function" then
          error("unsupported payload type: " .. type(value), 0)
        end
        writes = writes + 1
        kv[key] = value
      end,
    },
    emit = function(name, payload)
      reject_non_scalar("emit payload", payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
    on = function(name, fn)
      registrations[#registrations + 1] = { name = name, fn = fn }
    end,
    tool = {
      call = function(tool_id, args)
        for key, value in pairs(args or {}) do
          eq(type(key), "string", "tool args 的键类型")
          reject_non_scalar("tool args[" .. key .. "]", value)
        end
        tool_calls[#tool_calls + 1] = { tool = tool_id, args = args }
        local reply = tool_replies[tool_id]
        if reply == nil then
          return { ok = true, unmounted = false, code = 0, data = {} }
        end
        if type(reply) == "function" then
          return reply(args)
        end
        return reply
      end,
    },
    entity = {
      spawn = function()
        entity_calls = entity_calls + 1
        error("dialogue 不该调用 ygg.entity.spawn", 0)
      end,
      get_attribute = function()
        entity_calls = entity_calls + 1
        error("dialogue 不该调用 ygg.entity", 0)
      end,
    },
    data = function(id)
      data_calls = data_calls + 1
      error("dialogue 不该调用 ygg.data（文案是数据，包只搬键）：" .. tostring(id), 0)
    end,
    narrative = {
      node = function(id)
        narrative_calls = narrative_calls + 1
        world.last_node = id
      end,
      ["goto"] = function(id)
        narrative_calls = narrative_calls + 1
        world.last_node = id
      end,
    },
    ticks = function()
      ticks_calls = ticks_calls + 1
      return world.tick
    end,
  }

  world.writes_now = function()
    return writes
  end
  world.registrations = registrations
  world.tool_calls = tool_calls
  world.tool_call_count = function()
    return #tool_calls
  end
  world.entity_calls = function()
    return entity_calls
  end
  world.ticks_calls = function()
    return ticks_calls
  end
  world.data_calls = function()
    return data_calls
  end
  world.narrative_calls = function()
    return narrative_calls
  end
  return world
end

--- 取一份**全新**的模块实例并装上 mock（清缓存 = REGISTRY / 适配器 / 配置归零）。
local function fresh(world, deps)
  for i = 1, #MODULES do
    package.loaded[MODULES[i]] = nil
  end
  local dialogue = require("pkg.dialogue")
  dialogue.install(deps or { host = world.host })
  return dialogue
end

local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
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

--- 一个 builtin handler 的 spec（`fn` 进 REGISTRY，只留在 spec 上）。
local function builtin(name, opts, fn)
  return { kind = "builtin", name = name, opts = opts or {}, fn = fn }
end

local function agent(name, opts)
  return { kind = "agent", name = name, opts = opts or {} }
end

-- ── 用例 1-4：注册面与分发骨架 ──────────────────────────────────────────────

run("pkg.dialogue.on-off-shape", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "entity", window = 3 }, function()
    return true
  end))

  -- KV 面只有 spec：名表 + kind + opts 名表 + opts 值。**没有函数**（mock 的 set 会拒）。
  local specs = dialogue.handlers("greet")
  eq(#specs, 1, "handler 数量")
  eq(specs[1].kind, "builtin", "kind 落 KV")
  eq(specs[1].name, "npc", "name 落 KV")
  names_equal(sorted_keys(specs[1].opts), { "memory", "window" }, "opts 键集")
  eq(specs[1].opts.memory, "entity", "opts.memory 落 KV")
  eq(specs[1].opts.window, 3, "opts.window 落 KV")
  for key in pairs(world.kv) do
    local kind = type(world.kv[key])
    is_true(kind == "number" or kind == "string" or kind == "boolean", "KV 值非标量：" .. key)
  end

  -- off 后条目从名表消失（KV 里的墓碑键留在原处——宿主没有删除面）。
  is_true(dialogue.off("greet", "npc"), "off 命中")
  eq(#dialogue.handlers("greet"), 0, "off 后 handler 列表为空")
  eq(dialogue.off("greet", "npc"), false, "二次 off 返回 false")
  eq(world.kv["dlg.types"], "", "最后一个 handler 摘掉后 type 名表为空串（KV 无删除面）")

  -- 名与 kind 的拒绝路径。
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("Greet", builtin("npc", {}, function() end))
  end, "大写 type")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", { kind = "remote", name = "npc" })
  end, "词表外的 kind")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", { kind = "builtin", name = "bad name" })
  end, "名字带空格")
  expect_error("YGG-PKG-DIALOGUE-WINDOW", function()
    dialogue.on("greet", builtin("w0", { window = 0 }, function() end))
  end, "window=0")
end)

run("pkg.dialogue.order-is-registration", function()
  local world = new_world()
  local dialogue = fresh(world)
  local first_calls, second_calls = 0, 0
  dialogue.on("greet", builtin("first", {}, function()
    first_calls = first_calls + 1
    return true
  end))
  dialogue.on("greet", builtin("second", {}, function()
    second_calls = second_calls + 1
    return true
  end))

  -- I7：注册序即执行序，第一个认领的终止链。
  local receipt = dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  eq(receipt.by, "first", "第一个注册的认领")
  eq(first_calls, 1, "first 调用次数")
  eq(second_calls, 0, "second 一次都没被调")

  -- 摘掉 first 后 order 前移到 second（名表序，不是数字下标）。
  dialogue.off("greet", "first")
  local second_receipt = dialogue.emit("greet", 1, 2)
  eq(second_receipt.by, "second", "摘掉 first 后由 second 认领")
  eq(second_calls, 1, "second 调用次数")

  -- 同名重复注册 = -DUP（不是「后覆盖前」：那会让注册序随装载序漂）。
  expect_error("YGG-PKG-DIALOGUE-DUP", function()
    dialogue.on("greet", builtin("second", {}, function() end))
  end, "同名重复注册")
end)

run("pkg.dialogue.emit-receipt", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))

  local receipt = dialogue.emit("greet", 7, 8, { text_key = "beats.greet" })
  names_equal(sorted_keys(receipt), { "by", "depth", "handled" }, "回执键集恰好三个")
  eq(receipt.handled, true, "handled")
  eq(receipt.by, "npc", "by")
  eq(receipt.depth, 1, "顶层链的深度 = 1")

  -- 帧形状（d14 的同一断言面，这里先钉「不泄漏进回执」）。
  local frame = dialogue.frame("greet", 7, 8)
  names_equal(sorted_keys(frame), { "payload", "source", "target", "type", "v" }, "帧键集")
  eq(frame.v, 1, "帧版本")
  names_equal(sorted_keys(frame.payload), {}, "缺省 payload = 空表")
end)

run("pkg.dialogue.fallback-chain", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = false, unmounted = false, code = 1, error = "timeout after 900ms" }
  local builtin_calls = 0
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent", credential_key = "NPC_TOKEN" }))
  dialogue.on("greet", builtin("npc-fallback", {}, function()
    builtin_calls = builtin_calls + 1
    return true
  end))

  local receipt = dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  eq(receipt.by, "npc-fallback", "agent 失败后回退到 builtin")
  eq(receipt.handled, true, "回退后仍算已处理")
  -- 跳数 = 2（agent 一次 + builtin 一次），不是「调用成功」那么弱。
  eq(world.tool_call_count(), 1, "agent 跳数")
  eq(builtin_calls, 1, "builtin 跳数")
  eq(#events_named(world, "dialogue.failed"), 0, "回退成功不得发 failed")
end)
