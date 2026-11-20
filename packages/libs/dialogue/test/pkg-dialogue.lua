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

-- ── 用例 5-10：失败面、深度帽、记忆窗口 ─────────────────────────────────────

run("pkg.dialogue.failed-event", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = false, unmounted = false, code = 1, error = "boom-42" }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent" }))

  local receipt = dialogue.emit("greet", 3, 4, { text_key = "beats.greet" })
  eq(receipt.handled, false, "无人认领")
  is_nil(receipt.by, "by 为空")
  eq(#events_named(world, "dialogue.failed"), 1, "dialogue.failed 恰好一次")
  local payload = events_named(world, "dialogue.failed")[1].payload
  eq(type(payload), "string", "载荷是字符串（标量纪律）")
  eq(payload, "greet|boom-42", "载荷 = <type>|<原错误>")
  -- I2：失败不静默吞——错误文本原样挂在载荷里。
  is_true(payload:find("boom-42", 1, true) ~= nil, "载荷带原错误")
end)

run("pkg.dialogue.agent-unmounted", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = false, unmounted = true, code = -1, error = "no host tool runner mounted" }
  local builtin_calls = 0
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent" }))
  dialogue.on("greet", builtin("npc-fallback", {}, function()
    builtin_calls = builtin_calls + 1
    return true
  end))

  local receipt = dialogue.emit("greet", 1, 2)
  eq(receipt.by, "npc-fallback", "unmounted 也走回退链（不静默吞）")
  eq(builtin_calls, 1, "回退命中一次")

  -- 无 builtin 可退：failed 的 reason 是 unmounted（可断言，不是「没响应」）。
  dialogue.off("greet", "npc-fallback")
  dialogue.emit("greet", 1, 2)
  eq(events_named(world, "dialogue.failed")[1].payload, "greet|unmounted", "reason = unmounted")
end)

run("pkg.dialogue.depth-capped", function()
  local world = new_world()
  local dialogue = fresh(world, { host = world.host, config = { max_depth = 3 } })
  local calls = 0
  dialogue.on("loop", builtin("recur", { memory = "none" }, function(frame)
    calls = calls + 1
    -- 同步嵌套：handler 内再 emit 才是深度 +1（ygg.emit 入队不算）。
    dialogue.emit("loop", frame.source, frame.target)
    return true
  end))

  local receipt = dialogue.emit("loop", 1, 2)
  eq(calls, 3, "总调用数 == max_depth")
  eq(#events_named(world, "dialogue.depth_capped"), 1, "depth_capped 恰好一次")
  eq(events_named(world, "dialogue.depth_capped")[1].payload, "loop|4", "载荷 = <type>|<超限深度>")
  eq(receipt.handled, true, "最外层仍算已处理")
  eq(receipt.depth, 1, "最外层深度 = 1")

  -- 链结束后深度复位：下一次 emit 仍从 1 起算（异常/截断都不得永久压帽）。
  local after = dialogue.emit("loop", 1, 2)
  eq(after.depth, 1, "截断后深度复位")
  eq(#events_named(world, "dialogue.depth_capped"), 2, "第二次链同样在 max_depth+1 截断")
end)

run("pkg.dialogue.thread-window", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", {}, function()
    return true
  end))

  for i = 1, 20 do
    dialogue.emit("greet", 5, 6, { text_key = "beats.n" .. i })
  end
  local turns = dialogue.history("5:6")
  eq(#turns, 12, "窗口缺省 12")
  eq(turns[1].text, "beats.n9", "保留最新 12 轮（丢最旧）")
  eq(turns[12].text, "beats.n20", "最后一轮是第 20 次")
  for i = 2, #turns do
    is_true(turns[i].seq > turns[i - 1].seq, "n 严格递增")
  end
  eq(turns[12].seq, 20, "轮次号 = 全局 seq（不因截断回退）")
  eq(world.kv["dlg.seq"], 20, "dlg.seq 是唯一写口")
  eq(world.kv["dlg.thread.5:6.turns"], 12, "轮数落 KV")

  -- `opts.window` 覆盖缺省：3 轮窗口时只留 3 条。
  local other = fresh(world)
  other.on("greet", builtin("tight", { window = 3 }, function()
    return true
  end))
  for i = 1, 5 do
    other.emit("greet", 1, 2, { text_key = "x" .. i })
  end
  eq(#other.history("1:2"), 3, "opts.window 覆盖缺省")
end)

run("pkg.dialogue.memory-none", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("shout", builtin("npc", { memory = "none" }, function()
    return true
  end))

  local receipt = dialogue.emit("shout", 9, 10, { text_key = "beats.shout" })
  eq(receipt.handled, true, "handler 仍被调")
  eq(#dialogue.history("9:10"), 0, "没有线程历史")
  for key in pairs(world.kv) do
    is_true(key:find("^dlg%.thread%.") == nil, "memory=none 不得写线程键：" .. key)
  end
  is_nil(world.kv["dlg.keys"], "没有线程名表")
  is_nil(world.kv["dlg.seq"], "没有轮次推进")
end)

run("pkg.dialogue.memory-entity", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "entity" }, function()
    return true
  end))

  dialogue.emit("greet", 5, 6, { text_key = "beats.one" })
  dialogue.emit("greet", 5, 7, { text_key = "beats.two" })

  -- entity 模式：键 = "<source>"，两次对同一 source 的对话进同一条线程。
  local turns = dialogue.history("5")
  eq(#turns, 2, "entity 模式按 source 聚合")
  eq(turns[1].text, "beats.one", "第一轮")
  eq(turns[2].text, "beats.two", "第二轮")
  eq(world.kv["dlg.keys"], "5", "线程名表里只有 source 键")
  eq(#dialogue.history("5:6"), 0, "pair 键不存在")
  names_equal(dialogue.threads(), { "5" }, "threads() 名表")
end)

-- ── 静态面：源文件清单来自 manifest 的 packages 段（内容寻址的那份）─────────────

local function pkg_sources(id)
  local manifest = rawget(_G, "__MANIFEST")
  if type(manifest) ~= "table" or type(manifest.packages) ~= "table" then
    error("manifest 缺 packages 段（载体 pack.json 未声明包或未重算 manifest）", 0)
  end
  for i = 1, #manifest.packages do
    local pkg = manifest.packages[i]
    if pkg.id == id then
      local out = {}
      for j = 1, #pkg.files do
        out[#out + 1] = pkg.files[j].path
      end
      return out
    end
  end
  error("manifest 的 packages 段没有包 " .. id, 0)
end

local function count_in_sources(id, needle)
  local hits = 0
  local where = {}
  local paths = pkg_sources(id)
  for i = 1, #paths do
    local path = PKG_ROOT .. "/" .. paths[i]
    local handle = assert(io.open(path, "rb"), "读不到 " .. path)
    local text = handle:read("a")
    handle:close()
    local start = 1
    while true do
      local pos = text:find(needle, start, true)
      if pos == nil then
        break
      end
      hits = hits + 1
      where[#where + 1] = paths[i] .. "@" .. pos
      start = pos + #needle
    end
  end
  return hits, where
end

-- ── 用例 11-16：无时钟、通道封闭、凭据只留键名、帧、adapter、文本键 ──────────

run("pkg.dialogue.no-clock", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))

  -- I5：静态面——包源文件里一次 `ticks(` 都没有。
  local hits, where = count_in_sources("dialogue", "ticks(")
  eq(hits, 0, "包源码里的 ticks( 命中数： " .. table.concat(where, ","))

  -- 行为面——把 mock 时钟推到 12345，回执与之一致，且一次都没读它。
  world.tick = 12345
  local receipt = dialogue.emit("greet", 1, 2)
  eq(receipt.handled, true, "结果与时钟无关")
  eq(receipt.depth, 1, "深度与时钟无关")
  eq(world.ticks_calls(), 0, "ygg.ticks 调用次数必须为 0")
end)

run("pkg.dialogue.no-second-egress", function()
  local world = new_world()
  local dialogue = fresh(world)

  -- I1：静态面——四类字面量命中数全为 0（每一条都能返回反例）。
  local needles = {
    "http",
    "socket",
    "os.execute",
    "io.",
  }
  for i = 1, #needles do
    local hits, where = count_in_sources("dialogue", needles[i])
    eq(hits, 0, "静态命中 " .. needles[i] .. "： " .. table.concat(where, ","))
  end

  -- 运行时面——agent 走 `ygg.tool.call`，其它宿主面调用计数全 0。
  world.tool_replies["npc.agent"] = { ok = true, unmounted = false, code = 0, data = {} }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent" }))
  local receipt = dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  eq(receipt.handled, true, "agent 认领")
  eq(world.tool_call_count(), 1, "ygg.tool.call 是唯一外部调用")
  eq(world.entity_calls(), 0, "ygg.entity 调用次数")
  eq(world.data_calls(), 0, "ygg.data 调用次数")
  eq(world.narrative_calls(), 0, "ygg.narrative 调用次数（节点图归宿主，包不碰）")
end)

run("pkg.dialogue.credential-key-only", function()
  local world = new_world()
  local dialogue = fresh(world)
  local SECRET = "sk-live-DO-NOT-LEAK"
  world.tool_replies["npc.agent"] = { ok = true, unmounted = false, code = 0, data = {} }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent", credential_key = "NPC_SHOPKEEPER_TOKEN" }))

  dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  local call = world.tool_calls[1]
  eq(call.args.credential_key, "NPC_SHOPKEEPER_TOKEN", "工具实参里只有键名")
  -- I6：凭据值不出现在任何 KV 值里（哪怕调用方把它塞进 payload）。
  for key in pairs(world.kv) do
    local value = world.kv[key]
    is_true(tostring(value):find(SECRET, 1, true) == nil, "KV 里出现了凭据值：" .. key)
  end
  for i = 1, #world.events do
    is_true(tostring(world.events[i].payload):find(SECRET, 1, true) == nil, "事件里出现了凭据值")
  end

  -- 键名形态是硬约束（.env 的键名形态）；非法形态在注册期就拒。
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", agent("bad-key", { tool = "npc.agent", credential_key = "npc-token" }))
  end, "小写 credential_key")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.on("greet", agent("bad-key2", { tool = "npc.agent", credential_key = SECRET }))
  end, "把凭据值当键名")
end)

run("pkg.dialogue.frame-shape", function()
  local world = new_world()
  local dialogue = fresh(world)

  local frame = dialogue.frame("greet", 1, 2, { text_key = "beats.greet" })
  names_equal(sorted_keys(frame), { "payload", "source", "target", "type", "v" }, "帧键集")
  eq(frame.v, 1, "版本")
  eq(frame.type, "greet", "type")
  eq(frame.source, 1, "source 原样（ref，不转字符串）")
  eq(frame.target, 2, "target 原样")
  eq(frame.payload.text_key, "beats.greet", "payload 原样")

  -- 纯函数：不写 KV、不推 seq、不发事件。
  eq(world.writes_now(), 0, "frame 不写 KV")
  eq(#world.events, 0, "frame 不发事件")
  is_nil(world.kv["dlg.seq"], "frame 不推 dlg.seq")

  -- handler 侧拿到的帧与 frame() 同形（同一构造函数）。
  local seen = nil
  dialogue.on("greet", builtin("npc", { memory = "none" }, function(f)
    seen = f
    return true
  end))
  dialogue.emit("greet", 1, 2)
  names_equal(sorted_keys(seen), { "payload", "source", "target", "type", "v" }, "handler 收到的帧键集")
end)

run("pkg.dialogue.adapters-registry", function()
  local world = new_world()
  local dialogue = fresh(world)
  local send_calls = 0
  local function adapter()
    return {
      send = function()
        send_calls = send_calls + 1
        return nil
      end,
      health = function()
        return true
      end,
    }
  end

  is_true(dialogue.register_adapter("term", adapter()), "注册 term")
  is_true(dialogue.register_adapter("echo", adapter()), "注册 echo")
  names_equal(dialogue.adapters(), { "echo", "term" }, "adapters() 升序副本")

  -- 副本：调用方改返回值不影响注册表。
  local copy = dialogue.adapters()
  copy[1] = "tampered"
  names_equal(dialogue.adapters(), { "echo", "term" }, "adapters() 是副本")

  expect_error("YGG-PKG-DIALOGUE-DUP", function()
    dialogue.register_adapter("term", adapter())
  end, "重名注册")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.register_adapter("bad", { health = function() end })
  end, "缺 send")
  expect_error("YGG-PKG-DIALOGUE-TYPE", function()
    dialogue.register_adapter("bad2", "nope")
  end, "adapter 不是表")

  -- 包不驱动 adapter：一次 emit 之后 send 调用计数仍是 0（装配归 mod/宿主）。
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))
  dialogue.emit("greet", 1, 2)
  eq(send_calls, 0, "adapter.send 调用次数必须为 0")
end)

run("pkg.dialogue.text-key-passthrough", function()
  local world = new_world()
  local dialogue = fresh(world)
  local key = "beats.dlg.greet.问候"
  dialogue.on("greet", builtin("npc", {}, function()
    return true
  end))

  dialogue.emit("greet", 1, 2, { text_key = key })
  local turns = dialogue.history("1:2")
  eq(#turns, 1, "记了一轮")
  eq(turns[1].text, key, "文本键原样进 turns（字节相同）")
  eq(#turns[1].text, #key, "文本键长度不变")
  eq(world.kv["dlg.thread.1:2.t.1.text"], key, "KV 里的 text 也是同一个键")

  -- 静态面：包内零 `ygg.data(`（文案的唯一副本在 mod 的 data/texts）。
  local hits, where = count_in_sources("dialogue", "ygg.data(")
  eq(hits, 0, "包源码里的 ygg.data( 命中数： " .. table.concat(where, ","))
  eq(world.data_calls(), 0, "运行时也没调 ygg.data")
end)

-- ── 用例 17-22：quest 桥、纯导出、重载、KV 形状、导出点名面 ────────────────────

run("pkg.dialogue.quest-bridge", function()
  local world = new_world()
  package.loaded["pkg.quest"] = nil
  local quest = require("pkg.quest")
  package.loaded["pkg.quest"] = nil

  local dialogue = fresh(world, { host = world.host, quest = quest })

  -- 桥只读 `quest.EVENTS`（五个名字），一个 `ygg.on` 都不多不少。
  eq(#world.registrations, 5, "ygg.on 注册数")
  local registered = {}
  for i = 1, #world.registrations do
    registered[i] = world.registrations[i].name
  end
  table.sort(registered)
  names_equal(registered, {
    "quest.abandoned",
    "quest.advanced",
    "quest.completed",
    "quest.jumped",
    "quest.opened",
  }, "注册的事件名（= quest.EVENTS）")

  -- 触发桥：host 派发 `quest.advanced` → 对应对白 handler 被调，链深从 1 起算。
  local seen = nil
  dialogue.on("quest.advanced", builtin("npc", { memory = "none" }, function(frame)
    seen = frame
    return true
  end))
  local receipt = nil
  for i = 1, #world.registrations do
    if world.registrations[i].name == "quest.advanced" then
      receipt = world.registrations[i].fn("1|2")
      break
    end
  end
  is_true(receipt ~= nil, "桥返回回执")
  eq(receipt.depth, 1, "跨事件不共享深度：桥发起的是新链")
  eq(receipt.by, "npc", "对白 handler 认领")
  eq(seen.type, "quest.advanced", "帧 type = 事件名")
  eq(seen.source, 0, "source = 0（系统）")
  eq(seen.target, 0, "target = 0（系统）")
  eq(seen.payload.text_key, "quest.advanced", "缺省文本键 = 事件名（仍是键，不造文案）")
  eq(seen.payload.quest_payload, "1|2", "quest 载荷原样透传")
end)

run("pkg.dialogue.bridge-absent", function()
  local world = new_world()
  local dialogue = fresh(world)
  eq(#world.registrations, 0, "不传 deps.quest 时不装桥（不猜名字）")

  -- 形状不合法的 deps.quest 是硬错，不是「静默不装」。
  expect_error("YGG-PKG-DIALOGUE-INSTALL", function()
    dialogue.install({ host = world.host, quest = { EVENTS = "quest.opened" } })
  end, "EVENTS 不是表")
  eq(#world.registrations, 0, "被拒的 install 不注册")
end)

run("pkg.dialogue.pure-export", function()
  local world = new_world()

  -- I8：require + install 不写任何 KV（连空名表都不写）。
  fresh(world)
  eq(world.writes_now(), 0, "require + install 的 KV 写次数")
  eq(#sorted_keys(world.kv), 0, "require + install 后的 KV 键集")

  -- 只有 `on` 才写 handlers（install 不预写）。
  local dialogue = fresh(world)
  eq(#sorted_keys(world.kv), 0, "二次 install 仍是零写")
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    return true
  end))
  is_true(world.writes_now() > 0, "on 才写 KV")
end)

run("pkg.dialogue.reload-unbound", function()
  local world = new_world()
  local dialogue = fresh(world)
  local calls = 0
  dialogue.on("greet", builtin("npc", { memory = "none" }, function()
    calls = calls + 1
    return true
  end))
  eq(dialogue.emit("greet", 1, 2).handled, true, "重载前正常")

  -- 模拟进程重载：清模块缓存重新 require（函数丢，KV 不动）。
  for i = 1, #MODULES do
    package.loaded[MODULES[i]] = nil
  end
  local reloaded = require("pkg.dialogue")
  reloaded.install({ host = world.host })

  local receipt = reloaded.emit("greet", 1, 2)
  eq(receipt.handled, false, "重载后有 spec 无函数 = 未绑定")
  eq(calls, 1, "旧函数不再被调（REGISTRY 已丢）")
  eq(#events_named(world, "dialogue.failed"), 1, "dialogue.failed 恰好一次")
  eq(events_named(world, "dialogue.failed")[1].payload, "greet|unbound", "reason = unbound")
end)

run("pkg.dialogue.kv-shape", function()
  local world = new_world()
  local dialogue = fresh(world)
  world.tool_replies["npc.agent"] = { ok = true, unmounted = false, code = 0, data = {} }
  dialogue.on("greet", agent("npc-agent", { tool = "npc.agent", credential_key = "NPC_TOKEN" }))
  dialogue.on("greet", builtin("npc", { memory = "pair", window = 4 }, function()
    return true
  end))
  dialogue.emit("greet", 3, 4, { text_key = "beats.greet" })
  dialogue.say("9:10", "beats.push")

  local allowed = {
    "^dlg%.seq$",
    "^dlg%.types$",
    "^dlg%.keys$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.names$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.n%.[A-Za-z_][A-Za-z0-9_.:-]*%.kind$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.n%.[A-Za-z_][A-Za-z0-9_.:-]*%.opts$",
    "^dlg%.h%.[a-z][a-z0-9_-]*%.n%.[A-Za-z_][A-Za-z0-9_.:-]*%.opt%.[A-Za-z_][A-Za-z0-9_.:-]*$",
    "^dlg%.thread%.[^%%s]+%.turns$",
    "^dlg%.thread%.[^%%s]+%.t%.%d+%.who$",
    "^dlg%.thread%.[^%%s]+%.t%.%d+%.text$",
    "^dlg%.thread%.[^%%s]+%.t%.%d+%.seq$",
  }
  for key in pairs(world.kv) do
    local ok = false
    for i = 1, #allowed do
      if key:find(allowed[i]) then
        ok = true
      end
    end
    is_true(ok, "越界键：" .. key)
    eq(key:sub(1, 4), "dlg.", "键前缀：" .. key)
    local kind = type(world.kv[key])
    is_true(kind == "number" or kind == "string" or kind == "boolean", "非标量值：" .. key)
  end

  -- 名表口径：handler 名表保注册序（不排序），线程名表升序。
  eq(world.kv["dlg.h.greet.names"], "npc-agent\nnpc", "handler 名表 = 注册序")
  eq(world.kv["dlg.keys"], "3:4\n9:10", "线程名表升序")
  eq(world.kv["dlg.types"], "greet", "type 名表")
end)

run("pkg.dialogue.keys-export", function()
  local world = new_world()
  local dialogue = fresh(world)
  dialogue.on("greet", builtin("npc", { memory = "pair" }, function()
    return true
  end))
  dialogue.emit("greet", 1, 2, { text_key = "beats.greet" })
  dialogue.say("5:6", "beats.push")

  local actual = {}
  for key in pairs(world.kv) do
    actual[#actual + 1] = key
  end
  table.sort(actual)
  names_equal(dialogue.keys(), actual, "keys() 与实际键集")
  names_equal(dialogue.keys(), dialogue.keys(), "keys() 稳定")

  -- codec 点名面（D5：`ns.keys` 是 r1 落地的等价枚举面，本包只消费）。
  package.loaded["pkg.codec"] = nil
  package.loaded["pkg.entity"] = nil
  local codec = require("pkg.codec")
  codec.install({ host = world.host })
  local bundle = codec.export({ entities = false, keys = dialogue.keys(), mod = "pkg-selftest" })
  local exported = {}
  for key in pairs(bundle.kv) do
    exported[#exported + 1] = key
  end
  table.sort(exported)
  names_equal(exported, actual, "bundle.kv 的键集 = keys()")
  package.loaded["pkg.codec"] = nil
  package.loaded["pkg.entity"] = nil
end)
