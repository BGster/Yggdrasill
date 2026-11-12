-- src/runtime/flow.lua —— 转场与拒绝的唯二出口（`advance` / `refuse`）+ 状态机票据 + 拒绝留痕。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块写的 KV 前缀：`reject.*`（拒绝留痕，只读查询用）；
-- `fsm.stage` 由状态机独占写，本模块只 `emit` 转场事件。

local M = {}

local menus = require("src.view.menus")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local CONFIG_UI, MENU =
  menus.CONFIG_UI, menus.MENU
local render =
  render_mod.receipt
local run_phase, run_phase_label, stage =
  reqs.run_phase, reqs.run_phase_label, reqs.stage
local contains, fill, join =
  util.contains, util.fill, util.join

-- 转场票据：`stage.advance` 本身是可被外部直派的事件（`--event stage.advance`）。
-- D-MA23（R1）：内核 FSM 唯一转场是 live → retired；不设闸就能把 live 直接搬进
-- retired，绕过 review.note 武装路径与全部前置校验。
-- 只给 advance() 盖章，FSM 的 when 取证即消费——外部直派没有章，一律静默 no-op。
local stage_ticket = nil
local reject_count = 0

local function advance(target)
  -- 唯一合法落点：live → retired（review.note 武装路径）。其它目标一律拒绝执行——
-- advance 不再是通用搬阶段工具，协调/盘点/发布是 run.phase 微状态，不经 FSM。
  if target ~= reqs.STAGE.retired or stage() ~= reqs.STAGE.live then
    return
  end
  -- 转场即离开二级页（子页跟着阶段走，不跨阶段）。
  CONFIG_UI.set_page("")
  if target ~= stage() then
    stage_ticket = target
    ygg.emit("stage.advance", target)
  end
  ygg.narrative["goto"]("node:render")
end

-- 状态机 when 的唯一判据：目标态正确 ∧ 本步由 advance() 发起（取章即消费）。
function M.allowed(target, expected)
  if target ~= expected then
    return false
  end
  if stage_ticket ~= expected then
    return false
  end
  stage_ticket = nil
  return true
end

local function allowed_list(current)
  -- 只看相位菜单本体：拒绝文案里不能列出 config 子页的项（子页是展示层，不是相位能力）。
  local builder = MENU[current]
  local items = nil
  if builder ~= nil then
    items = builder()
  end
  if items == nil then
    return TEXTS.beats.allowed_none
  end
  local names = {}
  for i = 1, #items do
    if not contains(names, items[i].action) then
      names[#names + 1] = items[i].action
    end
  end
  return join(names, ", ")
end

-- ============================================================================
-- §唯一后继自动推进的落点已改为响应式：见 `src/core/hooks.lua`（useEffect + flush_effects）。
-- 本模块只负责转场与拒绝的出口；状态驱动的连锁反应不再写在各 handler 末尾——
-- 否则「已产生 choice 的 handler 里再 emit」会被内核丢弃（transition.rs 排空规则）。
-- ============================================================================


local function refuse(template, action, detail)
  -- 拒绝口径 = 运行级微状态（真状态只有 live/retired，辨析力全在相位层）。
  local current = run_phase()
  -- 拒绝留痕：宿主按 pack.json 的 rejectionPrefixes 判「未生效」，这里再给可审计计数。
  reject_count = reject_count + 1
  ygg.kv.set("reject.count", reject_count)
  ygg.kv.set("reject.last", tostring(action) .. ": " .. (detail or TEXTS.beats.rejected_default))
  ygg.narrate(fill(template, {
    action = tostring(action),
    stage = current,
    allowed = allowed_list(current),
    detail = detail or TEXTS.beats.rejected_default,
  }))
  render()
end

local function reject(action, detail)
  return refuse(TEXTS.beats.rejected, action, detail)
end

local function reject_payload(action, detail)
  return refuse(TEXTS.beats.rejected_payload, action, detail)
end

-- 相位门（§2.7 拒绝面的机器面）：action 只在给定微状态集合内可用；否则可读拒绝
-- （detail = 期望相位中文名，拒绝文案用中文名——代码标识符不进文案）。
-- 调用方传 RUN_PHASE 标识符数组；返回 true = 放行（false 时已拒绝并 render）。
local function phase_guard(action, phases)
  local current = run_phase()
  for i = 1, #phases do
    if phases[i] == current then
      return true
    end
  end
  local labels = {}
  for i = 1, #phases do
    labels[#labels + 1] = run_phase_label(phases[i])
  end
  return refuse(TEXTS.beats.rejected, action, fill(TEXTS.beats.phase_expected, {
    expected = join(labels, "/"),
    current = run_phase_label(current),
  }))
end

function M.install(deps)
  TEXTS = deps.texts
  local saved = ygg.kv.get("reject.count")
  if type(saved) == "number" then
    reject_count = saved
  end
end

M.advance = advance
M.allowed_list = allowed_list
M.phase_guard = phase_guard
M.refuse = refuse
M.reject = reject
M.reject_payload = reject_payload

return M
