-- src/domain/gates.lua —— 门禁与收口判据（只读选择器）：plan_gate / accept_gate / coordinate.submit 共用同一套。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：无（只读）。

local M = {}

local clock = require("src.core.clock")
local config = require("src.core.config")
local props = require("src.domain.proposals")
local readset = require("src.domain.readset")
local readsig = require("src.domain.readsig")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local P_PENDING, prop_count, prop_pending_total, prop_req, prop_status =
  props.P_PENDING, props.prop_count, props.prop_pending_total, props.prop_req, props.prop_status
local job_capacity, job_total, req_all_done, req_conflict, req_conflict_total, req_count, req_gate, req_id, req_job, req_key, req_needs_amend, req_open_total, req_rev, req_status =
  reqs.job_capacity, reqs.job_total, reqs.req_all_done, reqs.req_conflict, reqs.req_conflict_total, reqs.req_count, reqs.req_gate, reqs.req_id, reqs.req_job, reqs.req_key, reqs.req_needs_amend, reqs.req_open_total, reqs.req_rev, reqs.req_status
local fill, join, trim =
  util.fill, util.join, util.trim

-- 门禁种类现在由 `req.gate`（""/"plan"/"accept"）承载，不再是一个状态。
local function gate_kind(index)
  local kind = req_gate(index)
  if kind == "" then
    return nil
  end
  return kind
end

local function gate_next()
  for i = 1, req_count() do
    if gate_kind(i) ~= nil then
      return i
    end
  end
  return nil
end

local function gate_total()
  local total = 0
  for i = 1, req_count() do
    if gate_kind(i) ~= nil then
      total = total + 1
    end
  end
  return total
end

local function req_artifacts(index)
  return text_of(req_key(index, "artifacts"), "")
end

local function gate_evidence_problem(index, kind)
  if kind == "accept" and req_artifacts(index) == "" then
    return fill(TEXTS.beats.req_gate_no_evidence, { id = req_id(index) })
  end
  return nil
end

local function gate_rev_ok(index)
  -- gate_rev 从不清零，所以「仍在门禁上」是硬前提（否则已批准的需求会被重复批准）。
  return gate_kind(index) ~= nil and num(req_key(index, "gate_rev"), 0) == req_rev(index)
end

local function gate_problem()
  -- r2 rev2（B 方案⑥）空局保护两条款：章程非空 ∧ 需求数 ≥1，否则三个收口动作
  -- （coordinate.submit / verify.submit / review.note）共用同一条可读拒绝。
  if trim(text_of("task.title", "")) == "" then
    return TEXTS.beats.gate_charter_first
  end
  if req_count() == 0 then
    return TEXTS.beats.gate_charter_first
  end
  if prop_pending_total() > 0 then
    return fill(TEXTS.beats.req_gate_props, { pending = tostring(prop_pending_total()) })
  end
  if job_total() > 0 then
    return fill(TEXTS.beats.req_gate_jobs, { jobs = tostring(job_total()), cap = tostring(job_capacity()) })
  end
  if req_conflict_total() > 0 then
    return fill(TEXTS.beats.req_gate_conflicts, { conflicts = tostring(req_conflict_total()) })
  end
  if not req_all_done() then
    return fill(TEXTS.beats.req_gate_open, { open = tostring(req_open_total()) })
  end
  return nil
end

-- 需求自身的 pending 提案数（gate_problem_for 的计数口径；与 requirements.req_has_pending
-- 同读一张 prop.* 表、同一个 pending 状态，布尔版只给真值不给数量，这里要有 {{count}} 文案）。
local function req_pending_total(index)
  local total = 0
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING and prop_req(i) == index then
      total = total + 1
    end
  end
  return total
end

-- 单需求门禁判据（推进类专用，与 gate_problem 的 run 收口/证据全局屏障分层）：
-- rN 的门禁只被 rN 自身事态阻塞——其它需求的 pending 提案 / 在跑 job / 冲突不进本判定。
-- 依序返回第一个命中（人类可读细节），否则 nil：
--   1) 自身 pending 提案（防御性：正常流被 submit_slot_ok 的 gate 守卫 + req_prop_busy 挡住）
--   2) 自身在跑 job（防御性：提案提交时已清 job——plan 交付即门禁，dev 落受理幕）
--   3) 自身未解决冲突 / needs_amend（唯一真实可共存：冲突雷达 / resolve revise 可在待批期落）
-- 显式不含：stale / gate_rev 失配（gate.approve 的 gate_rev_ok 已查，避免双报）、
-- req_all_done / req_open_total（收口专用）与一切全局计数。
local function gate_problem_for(index)
  local own = req_pending_total(index)
  if own > 0 then
    return fill(TEXTS.beats.req_gate_own_props, { id = req_id(index), count = tostring(own) })
  end
  local job = req_job(index)
  if job ~= "" then
    return fill(TEXTS.beats.req_gate_own_job, { id = req_id(index), job = job })
  end
  if req_conflict(index) ~= "" or req_needs_amend(index) then
    return fill(TEXTS.beats.req_gate_own_conflict, { id = req_id(index) })
  end
  return nil
end

-- 局末清理前件（收口专用，与 gate_problem 分层）：
-- gate_problem 管需求侧（待验收提案 / 在跑 job / 未解决冲突 / 未 done）；
-- 这里管 run 级结算（进化复核 / subagent 会话 / 主会话 Todo）。全部通过才允许 review.note。
-- Lua 沙箱没有文件、会话与 Todo API：本函数只校登记断言，不证明实际复核或清理发生过。
-- ============================================================================
-- 读签四维判据（r4）：`gate_problem` 的**同层兄弟**，不是它的一部分。
--
-- 为什么必须分开：`gate_problem` 是局收口判据，被 coordinate.submit / verify.submit(true)
-- / review.note 共用；把读签塞进去会连带拦住退役终局（武装后的 review.note），而那是唯一
-- 出口——死锁。这里按**动作**定必读集，只影响三个推进动作，收口语义一字不动。
--
-- 四维（无时钟，全用事件序号与 KV 版本；序号由 src/core/clock.lua 落 KV）：
--   ① 覆盖  —— 必读集内每件都有签名，且签名 ver == 台账 ver（文档改版 ⇒ 视为未签）
--   ② 主体  —— 签名晚于「该需求当前 rev」的进入 seq（amend / 打回推进 rev ⇒ 作废）
--   ③ 新鲜  —— 签名晚于本幕 / 本阶段纪元的进入 seq（跨幕、跨阶段 ⇒ 作废）
--   ④ 遗忘  —— now - sign ≤ window（窗口可配，缺省 80）
-- 每条拒绝都必须自解除：报「隔了多少派发」「当前窗口」「确切命令」。
-- 生成区缺席（必读集为空）⇒ 直接放行，不叙事：判据 inert 是分阶段上线的安全阀。

local function sign_command(doc, ver)
  local shown = ver
  if shown == "" then
    shown = "<ver 抄自 docs/context/" .. tostring(doc) .. ".md 文件头>"
  end
  return "config.set read." .. tostring(doc) .. "=" .. shown
end

local function raise_command(window)
  local target = window * 2
  if target < window + 10 then
    target = window + 10
  end
  if target > readset.MAX_WINDOW then
    target = readset.MAX_WINDOW
  end
  return "config.set readGate.window=" .. tostring(target)
end

-- 本动作的「作用域进入点」：幕与阶段取最大者（rev 单列为主体维度，不混进来）。
-- 返回整个纪元条目，而不只是 seq——拒绝文案里「什么时候开始的」必须报**真正赢得比较那个**
-- 纪元的名字（幕还是阶段），否则会出现「说 scene 却指向阶段边界」的错报。
local function scope_epoch_of(epochs)
  local best = nil
  for i = 1, #epochs do
    local epoch = epochs[i]
    if epoch.kind ~= "rev" and (best == nil or epoch.seq > best.seq) then
      best = epoch
    end
  end
  return best
end

local function rev_epoch_of(epochs)
  for i = 1, #epochs do
    if epochs[i].kind == "rev" then
      return epochs[i]
    end
  end
  return nil
end

local function read_problem(action, ctx)
  ctx = ctx or {}
  readset.observe()
  local docs = readset.bearing(action, ctx)
  if #docs == 0 then
    return nil
  end
  local now_seq = clock.now()
  local window = readset.window()
  local raise = raise_command(window)
  local epochs = readset.epochs(ctx)
  local scope_epoch = scope_epoch_of(epochs)
  local scope = scope_epoch ~= nil and scope_epoch.scope or tostring(action)
  if not readset.has_ledger() then
    ygg.narrate(fill(TEXTS.beats.read_gate_unvered, {
      action = tostring(action),
      count = tostring(#docs),
    }))
  end

  -- ① 覆盖：未签 + 版本漂移（台账 ver ≠ 签名 ver）都算「没有有效签名」。
  local missing, commands = {}, {}
  for i = 1, #docs do
    local doc = docs[i]
    local want = readset.doc_ver(doc)
    local got = readsig.signed(doc)
    if got == nil then
      missing[#missing + 1] = doc
      commands[#commands + 1] = sign_command(doc, want)
    elseif want ~= "" and got ~= want then
      missing[#missing + 1] = doc .. " (signed " .. got .. ", now " .. want .. ")"
      commands[#commands + 1] = sign_command(doc, want)
    end
  end
  if #missing > 0 then
    local gap = now_seq - (scope_epoch ~= nil and scope_epoch.seq or 0)
    if gap < 0 then
      gap = 0
    end
    return fill(TEXTS.beats.read_gate_missing, {
      action = tostring(action),
      count = tostring(#missing),
      scope = scope,
      gap = tostring(gap),
      window = tostring(window),
      docs = join(missing, " / "),
      commands = join(commands, " && "),
      raise = raise,
    })
  end

  -- ②③④ 逐件判序（主体优先于新鲜，新鲜优先于遗忘：越具体越先报）。
  local entry = scope_epoch ~= nil and scope_epoch.seq or 0
  local rev_epoch = rev_epoch_of(epochs)
  for i = 1, #docs do
    local doc = docs[i]
    local _, _, value_seq = readsig.signed(doc)
    local sign = 0
    if value_seq ~= nil and value_seq > 0 then
      sign = value_seq
    else
      sign = clock.sign_seq(doc)
    end
    -- 序号未知（r3 时代签的存档 / 只读局）⇒ 该件不判序，只保留「签过」这一事实。
    if sign > 0 then
      local command = sign_command(doc, readset.doc_ver(doc))
      if rev_epoch ~= nil and rev_epoch.seq > sign then
        return fill(TEXTS.beats.read_gate_subject, {
          doc = doc,
          sign = tostring(sign),
          req = tostring(rev_epoch.req),
          rev = tostring(rev_epoch.rev),
          entry = tostring(rev_epoch.seq),
          gap = tostring(rev_epoch.seq - sign),
          window = tostring(window),
          command = command,
          raise = raise,
        })
      end
      if entry > sign then
        return fill(TEXTS.beats.read_gate_stale, {
          doc = doc,
          sign = tostring(sign),
          scope = scope,
          entry = tostring(entry),
          gap = tostring(entry - sign),
          window = tostring(window),
          command = command,
          raise = raise,
        })
      end
      if now_seq - sign > window then
        return fill(TEXTS.beats.read_gate_forgotten, {
          doc = doc,
          gap = tostring(now_seq - sign),
          now = tostring(now_seq),
          window = tostring(window),
          command = command,
          raise = raise,
        })
      end
    end
  end
  return nil
end

local function settlement_problem()
  if ygg.kv.get("verify.passed") ~= true then
    return TEXTS.beats.settlement_verify_first
  end
  local missing = {}
  if trim(text_of("settlement.evolution.summary", "")) == "" then
    missing[#missing + 1] = TEXTS.beats.settlement_evolution_missing
  end
  if text_of("settlement.sessions.destination", "") == "" then
    missing[#missing + 1] = TEXTS.beats.settlement_sessions_missing
  end
  if num("settlement.todo.remaining", -1) ~= 0 then
    missing[#missing + 1] = TEXTS.beats.settlement_todo_missing
  end
  if #missing > 0 then
    return join(missing, "; ")
  end
  return nil
end

function M.install(deps)
  TEXTS = deps.texts
end

M.gate_evidence_problem = gate_evidence_problem
M.gate_kind = gate_kind
M.gate_next = gate_next
M.gate_problem = gate_problem
M.gate_problem_for = gate_problem_for
M.gate_rev_ok = gate_rev_ok
M.gate_total = gate_total
M.read_problem = read_problem
M.req_artifacts = req_artifacts
M.settlement_problem = settlement_problem

return M
