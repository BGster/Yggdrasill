-- .ygg/mods/agent-seed/tests/harness.lua
--
-- 纯 Lua 自检的宿主替身（mock `ygg`）与断言工具。
--
-- 边界：这里跑的是**普通 Lua**，不是内核沙箱。`ygg.*` 是可观测的替身，
-- 因此断言能覆盖 mod 的业务语义，但覆盖不到 driver 层（事务回滚 / fuel /
-- save receipt / 事件路由）。见 docs/design/mod/testing-strategy.md。

------------------------------------------------------------------------------
-- 1. 收集面
------------------------------------------------------------------------------

local KV = {}
local NARRATED = {}
local EMITTED = {}
local HANDLERS = {}
local QUEUE = {}
local DRAINING = false

-- stamp-only 路由：宿主在声明路由上盖 `|actor=` 章（pack.json inboxRoutes 的 actor:true）。
local STAMP_ONLY = {
  ["proposal.submit"] = true,
  ["config.set"] = true,
  ["gate.approve"] = true,
  ["coordinate.submit"] = true,
}
local ACTOR = "selftest"

local FSM = nil

--- 最小 FSM：`ygg.fsm.define` 的 transitions 里 from/to/when 匹配即推进 `fsm.stage`。
local function apply_fsm(name, payload)
  if FSM == nil or name ~= FSM.event then
    return false
  end
  local from = KV["fsm.stage"] or FSM.initial
  for _, tr in ipairs(FSM.transitions or {}) do
    if tr.from == from and tr.to == payload and (tr.when == nil or tr.when(payload)) then
      KV["fsm.stage"] = tr.to
      return true
    end
  end
  return false
end

--- 投递一个事件到 handler，按路由语义决定是否盖 actor 章。
local function deliver(name, payload)
  local fn = HANDLERS[name]
  if fn == nil then
    return nil
  end
  local body = payload
  if STAMP_ONLY[name] and type(body) == "string" and not body:find("|actor=", 1, true) then
    body = body .. "|actor=" .. ACTOR
  end
  return fn(body)
end

--- 按 FIFO 把 emit 收集到的事件派发出去（重入安全：外层循环会继续消费）。
local function drain()
  if DRAINING then
    return
  end
  DRAINING = true
  while #QUEUE > 0 do
    local event = table.remove(QUEUE, 1)
    deliver(event.name, event.payload)
  end
  DRAINING = false
end

ygg = {
  engine_version = "selftest",
  ticks = function()
    return 0
  end,
  kv = {
    get = function(key)
      return KV[key]
    end,
    set = function(key, value)
      KV[key] = value
    end,
  },
  narrate = function(line)
    NARRATED[#NARRATED + 1] = line
  end,
  -- 沙箱里 `ygg.emit` 是**收集面**：driver 在 handler 返回后按队列 dispatch。
  -- 这里用同步队列复刻同一语义（否则 `choice_selected` 发出的动作会悬空）。
  emit = function(name, payload)
    EMITTED[#EMITTED + 1] = { name = name, payload = payload }
    if apply_fsm(name, payload) then
      -- 真转场后必须重新渲染：`[ENDING ...]` 一类叙述由 RENDER[retired] 在渲染时产生，
      -- 不是 handler 直接 narrate 的。mock 里 render 的唯一可达入口是 agent.summary。
      __SELFTEST.lastChoice = nil
      local summary = HANDLERS["agent.summary"]
      if summary ~= nil then
        summary("")
      end
      return
    end
    QUEUE[#QUEUE + 1] = { name = name, payload = payload }
    drain()
  end,
  on = function(name, fn)
    HANDLERS[name] = fn
  end,
  data = function(id)
    return __ASSETS[id]
  end,
  choice = function(prompt, options)
    __SELFTEST.lastChoice = { prompt = prompt, options = options }
  end,
  meta = function(key, value)
    __SELFTEST.meta[key] = value
  end,
  narrative = {
    node = function() end,
    ["goto"] = function() end,
  },
  fsm = {
    define = function(def)
      __SELFTEST.fsm = def
      FSM = def
    end,
  },
  fs = {},
  tool = {
    call = function() end,
  },
  choice_selected = function() end,
  choice_selector = function() end,
}

------------------------------------------------------------------------------
-- 2. 用例收集
------------------------------------------------------------------------------

__SELFTEST = {
  pass = 0,
  fail = 0,
  names = {},
  meta = {},
  lastChoice = nil,
  fsm = nil,
}

local function describe(err)
  if type(err) == "string" then
    return err
  end
  return tostring(err)
end

--- 注册并立即执行一条用例。用例失败不中断同文件内的其它用例。
function test(name, fn)
  local ok, err = pcall(fn)
  if ok then
    __SELFTEST.pass = __SELFTEST.pass + 1
    __SELFTEST.names[#__SELFTEST.names + 1] = name
  else
    __SELFTEST.fail = __SELFTEST.fail + 1
    print(("  FAIL  %s\n        %s"):format(name, describe(err)))
  end
end

------------------------------------------------------------------------------
-- 3. 断言
------------------------------------------------------------------------------

function assert_true(value, message)
  if not value then
    error(message or "expected truthy", 2)
  end
end

function assert_nil(value, message)
  if value ~= nil then
    error((message or "expected nil") .. (" (got %s)"):format(tostring(value)), 2)
  end
end

function assert_eq(actual, expected, message)
  if actual ~= expected then
    error(
      ("%s\n        expected: %s\n        actual:   %s"):format(
        message or "values differ",
        tostring(expected),
        tostring(actual)
      ),
      2
    )
  end
end

function assert_contains(haystack, needle, message)
  if type(haystack) ~= "string" or not haystack:find(needle, 1, true) then
    error(
      ("%s\n        expected to contain: %s\n        actual: %s"):format(
        message or "missing substring",
        tostring(needle),
        tostring(haystack)
      ),
      2
    )
  end
end

------------------------------------------------------------------------------
-- 4. 观测工具
------------------------------------------------------------------------------

--- 最近一次注册的 handler（按事件名直接调用 = 白盒驱动）。
function handler(name)
  return HANDLERS[name]
end

--- 调用一个 handler；未注册即失败（防止「测试悄悄空跑」）。
function fire(name, payload)
  assert_true(HANDLERS[name] ~= nil, ("handler not registered: %s"):format(name))
  return deliver(name, payload)
end

--- 单发直派：**不**盖 actor 章（模拟 `--event` 直派路，无宿主身份段）。
function fire_direct(name, payload)
  assert_true(HANDLERS[name] ~= nil, ("handler not registered: %s"):format(name))
  return HANDLERS[name](payload)
end

function kv(key)
  return KV[key]
end

function set_kv(key, value)
  KV[key] = value
end

--- 清空收集面（每个用例开头调用，避免串味）。
function reset()
  NARRATED = {}
  EMITTED = {}
  __SELFTEST.lastChoice = nil
  return NARRATED
end

function narrated()
  return NARRATED
end

function emitted()
  return EMITTED
end

--- 叙述里是否出现过某个子串。
function narrated_contains(needle)
  for _, line in ipairs(NARRATED) do
    if line:find(needle, 1, true) then
      return true
    end
  end
  return false
end

--- 清空 KV 与收集面，然后重新开局（等价于 Rust 侧每个用例新建一个 Game）。
--- handlers 不动：它们是 mod 加载时注册的，跨用例复用。
function fresh()
  -- 保真：Rust 侧每个用例都是一个全新内核。这里用「清模块缓存 + 重载 mod」等价实现，
  -- 否则模块级 local（config 缓存、lanes 台账等）会在用例之间串味。
  for name in pairs(package.loaded) do
    if type(name) == "string" and name:match("^src%.") then
      package.loaded[name] = nil
    end
  end
  for key in pairs(KV) do
    KV[key] = nil
  end
  NARRATED = {}
  EMITTED = {}
  __SELFTEST.lastChoice = nil
  __SELFTEST.meta = {}
  HANDLERS = {}
  dofile(__MOD_DIR .. "/mod.lua")
  fire("event.start", "0")
end

--- 读一个文件；失败即抛错（避免「读不到就静默通过」）。
function read_file(path)
  local handle = io.open(path, "r")
  assert_true(handle ~= nil, ("file must be readable: %s"):format(path))
  local text = handle:read("*a")
  handle:close()
  return text
end

--- 路径是否存在（文件或目录）。
function exists(path)
  local handle = io.open(path, "r")
  if handle then
    handle:close()
    return true
  end
  local probe = io.popen(('test -e "%s" && echo yes'):format(path))
  local out = probe:read("*l")
  probe:close()
  return out == "yes"
end

------------------------------------------------------------------------------
-- 5. 读签门禁模拟（镜像 src/domain/readset.lua 的 bearing/ensure_reads）
--
-- 口径：幕维度 readPlan.<scene>，阶段维度 readGate.reqActs.<stage>，
-- 门禁维度 readGate.gates.<kind>，批边界 readPlan.settle；ver 一律从 readGate.ver 读。
------------------------------------------------------------------------------

local function mod_config()
  return __ASSETS["ygg:agent/config"]
end

local function current_scene()
  local phase = KV["run.phase"]
  if type(phase) == "string" and phase ~= "" then
    return phase
  end
  return "coordinate"
end

--- 待批门禁：第一个非空 `req.N.gate`（与 gate_actions.pick_gate 同口径）。
local function pending_gate()
  for i = 1, 64 do
    local kind = KV["req." .. i .. ".gate"]
    if type(kind) == "string" and kind ~= "" then
      return i, kind
    end
  end
  return nil
end

local function bearing(action, stage, req)
  local config = mod_config()
  local plan = config.readPlan or {}
  local gate = config.readGate or {}
  local docs, seen = {}, {}
  local function push(group)
    if type(group) == "table" then
      for _, doc in ipairs(group) do
        if type(doc) == "string" and not seen[doc] then
          seen[doc] = true
          docs[#docs + 1] = doc
        end
      end
    end
  end

  if action == "proposal.submit" then
    push(plan[current_scene()])
    if stage and gate.reqActs then
      push(gate.reqActs[stage])
    end
  elseif action == "gate.approve" then
    local index = req or pending_gate()
    if index then
      local kind = KV["req." .. index .. ".gate"]
      if type(kind) == "string" and gate.gates then
        push(gate.gates[kind])
      end
    end
  elseif action == "coordinate.submit" then
    push(plan.settle)
  end

  local out = {}
  for _, doc in ipairs(docs) do
    out[#out + 1] = { doc = doc, ver = (gate.ver and gate.ver[doc]) or "" }
  end
  return out
end

local function scope_epoch(req)
  local epoch = KV["settings.epoch.act.seq"] or 0
  if req then
    local stage = KV["settings.epoch.req." .. req .. ".stage.seq"] or 0
    if stage > epoch then
      epoch = stage
    end
  end
  return epoch
end

local function sign_seq_of(doc, value)
  local third = tonumber(tostring(value):match("^[^|]*|[^|]*|(%d+)"))
  if third and third > 0 then
    return third
  end
  return KV["settings.sign_seq." .. doc] or 0
end

--- 把该动作的必读集签成新鲜的（未签 / 跨纪元 / 超窗口 → 重签）。
function ensure_reads(action, stage, req)
  local now = KV["settings.clock_seq"] or 0
  local window = KV["settings.readGate.window"] or 80
  local index = req or pending_gate()

  for _, item in ipairs(bearing(action, stage, index)) do
    local doc, ver = item.doc, item.ver
    if ver ~= "" then
      local signed = KV["settings.read." .. doc]
      local fresh = false
      if type(signed) == "string" and signed ~= "" then
        local head = signed:match("^([^|]*)")
        local seq = sign_seq_of(doc, signed)
        if head == ver and seq > scope_epoch(index) and (now - seq) <= window then
          fresh = true
        end
      end
      if not fresh then
        fire("config.set", "read." .. doc .. "=" .. ver)
      end
    end
  end
end

--- 重新渲染并取菜单项（mock 的 ygg.choice 收集面）。
function render_options()
  -- 必须**重新渲染**再读：`ensure_reads` 补签时会 fire `config.set`，把菜单缓存改成
  -- 配置页的选项。Rust 侧 `options()` 用宿主的 `state` 命令重新查询，这里用
  -- `agent.summary`（唯一会把 render() 走一遍的 handler）等价复刻。
  __SELFTEST.lastChoice = nil
  fire("agent.summary", "")
  local choice = __SELFTEST.lastChoice
  assert_true(choice ~= nil, "no pending choice after agent.summary render")
  return choice.options
end

--- 点菜单：先按动作补签，再按前缀找序号，最后发 ygg.choice_selected。
function click(prefix)
  local action = prefix:match("^([%w%._%-]+)")
  if action then
    ensure_reads(action, nil, nil)
  end
  local options = render_options()
  local index
  for i, text in ipairs(options) do
    if text:sub(1, #prefix) == prefix then
      index = i
      break
    end
  end
  assert_true(index ~= nil, ("no menu entry starts with `%s`"):format(prefix))
  return fire("ygg.choice_selected", tostring(index))
end

--- 章程三件套 → 开工（Rust 侧的 charter_and_begin）。
function charter_and_begin()
  click("charter.set — ")
  click("run.begin — ")
end

--- 提交 r1 的提案（先按 stage 补签，再发 7 段载荷）。
function submit_req(id, rev, stage, ext)
  local index = tonumber(id:match("r(%d+)"))
  ensure_reads("proposal.submit", stage, index)
  local payload = ("%s|%d|%s|%s proposal summary|artifacts/%s.md||%s"):format(id, rev, stage, stage, stage, ext)
  return fire("proposal.submit", payload)
end

function submit(rev, stage, ext)
  return submit_req("r1", rev, stage, ext)
end

--- 切换当前 actor（模拟宿主在不同命令上盖不同身份章）。
function as_actor(name)
  ACTOR = name or "test-agent"
end

function actor()
  return ACTOR
end

--- 一条在册需求走到 plan job（Rust 的 start_requirement）。
function start_requirement()
  charter_and_begin()
  fire("requirement.add", "synthetic requirement|synthetic description|semantics-r1")
  fire("requirement.dispatch", "0")
  assert_eq(kv("req.1.status"), "open", "status stays open until integration")
  assert_eq(kv("req.1.job"), "plan", "the plan job is hung")
  return phase_of("r1")
end

--- 走到计划门：需求在册 + 一条 plan 提案待受理。
function drive_to_plan_gate()
  fresh()
  local phase = start_requirement()
  assert_eq(phase, "planning", "start_requirement lands in planning")
  submit(1, "plan", "semantics-p1")
  assert_eq(kv("req.1.gate"), "plan", "the plan gate is up")
  assert_eq(kv("req.1.gate_rev"), 1, "gate_rev pins the revision")
  assert_eq(kv("req.1.job"), "", "the job slot frees once the gate is up")
end

--- 从 requirement.list 的行里取某需求的微状态名（`[planning]` → "planning"）。
function phase_of(id)
  -- 每次独立回执（与 Rust 的 narrate 一致）：否则会读到上一次 requirement.list 的旧行。
  reset()
  fire("requirement.list", "0")
  local needle = " " .. id .. " ["
  for _, line in ipairs(narrated()) do
    if line:find(needle, 1, true) then
      local open = line:find("[", 1, true)
      local close = line:find("]", open, true)
      if open and close then
        return line:sub(open + 1, close - 1)
      end
    end
  end
  error("no " .. id .. " in the requirement list")
end

--- 计划门 → 开发交付待受理（Rust 的 drive_to_dev_pending）。
function drive_to_dev_pending(ext)
  drive_to_plan_gate()
  click("gate.approve — ")
  assert_eq(kv("req.1.plan_ok"), true, "approving the plan gate sets plan_ok")
  assert_eq(kv("req.1.job"), "development", "the development job starts in the same turn")
  assert_eq(kv("req.1.lane"), 1, "the lane is bound")
  submit(1, "development", ext or "semantics-p2")
end

--- 受理交付 → 验收门（Rust 的 drive_to_accept）。
function drive_to_accept()
  click("proposal.accept — ")
  assert_eq(kv("req.1.gate"), "accept", "the accept gate is up")
  assert_eq(kv("req.1.gate_rev"), 1, "gate_rev pins the revision")
  assert_eq(phase_of("r1"), "accept_gate", "the row sits on the accept gate")
end

--- 验收门 → 集成中（lane 仍绑定，R7）。
function drive_to_integrating()
  click("gate.approve — ")
  assert_eq(kv("req.1.integrating"), true, "approving the accept gate opens act 8")
  assert_eq(kv("req.1.status"), "open", "true status only moves at the integration confirm")
  assert_eq(kv("req.1.lane"), 1, "the lane stays bound through acceptance")
  assert_eq(phase_of("r1"), "integrating", "the row is integrating")
end

--- 集成中 → done（integration.confirm 携带 develop@<sha>）。
function drive_to_done()
  reset()
  fire("integration.confirm", "r1|develop@abc1234")
  assert_true(narrated_contains("[INTEGRATION CONFIRMED]"), "the confirm must narrate its beat")
  assert_eq(kv("req.1.status"), "done", "the true state moves to done")
  assert_eq(kv("req.1.develop_sha"), "abc1234", "the develop sha is recorded")
  assert_eq(kv("req.1.integrating"), false, "the integrating flag clears")
  assert_eq(kv("req.1.lane"), 0, "the lane is released")
  assert_eq(phase_of("r1"), "done", "the row is done")
end

--- 走到盘点幕（Rust 的 drive_to_settle）。
function drive_to_settle()
  drive_to_dev_pending("semantics-p2")
  drive_to_accept()
  drive_to_integrating()
  drive_to_done()
  reset()
  click("coordinate.submit — ")
  assert_eq(kv("run.phase"), "settle", "coordinate.submit opens the settle act")
end

--- 走到发布幕（Rust 的 drive_to_release）。
function drive_to_release()
  drive_to_settle()
  click("verify.submit all_green=true — ")
  assert_eq(kv("verify.passed"), true, "verify.submit all_green=true records the evidence")
  fire(
    "settlement.evolution",
    "|candidates screened against the evolution chapter|c=0/0|" ..
      "sessions=1/0/~/.pi/agent/sessions-trash/semantics|todo=0/re-checked the todo list"
  )
end

--- 带 `exec=` 尾段提交 r1 提案（内联执行者台账）。
function submit_with_exec(rev, stage, ext, exec)
  ensure_reads("proposal.submit", stage, 1)
  local payload = ("r1|%d|%s|%s proposal summary|artifacts/%s.md||%s|exec=%s"):format(
    rev, stage, stage, stage, ext, exec
  )
  return fire("proposal.submit", payload)
end

--- requirement.list 的图头行（per-micro-state 计数面板 = useMemo 派生面）。
function panel()
  reset()
  fire("requirement.list", "0")
  for _, line in ipairs(narrated()) do
    if line:sub(1, #"[REQ GRAPH]") == "[REQ GRAPH]" then
      return line
    end
  end
  error("requirement.list must start with the graph header")
end

--- 带 exec / actor / actor_session 三段的提案提交（模拟宿主盖章的完整尾链）。
function submit_with_identity(rev, stage, ext, exec, actor, session)
  -- 读签按人（r9）：宿主盖的 actor 章决定门禁查谁的签名，所以签名也必须换成同一个人。
  local prev = ACTOR
  if actor then
    as_actor(actor)
  end
  ensure_reads("proposal.submit", stage, 1)
  local payload = ("r1|%d|%s|%s proposal summary|artifacts/%s.md||%s"):format(rev, stage, stage, stage, ext)
  if exec then
    payload = payload .. "|exec=" .. exec
  end
  if actor then
    payload = payload .. "|actor=" .. actor
  end
  if session then
    payload = payload .. "|actor_session=" .. session
  end
  local ret = fire("proposal.submit", payload)
  ACTOR = prev
  return ret
end
