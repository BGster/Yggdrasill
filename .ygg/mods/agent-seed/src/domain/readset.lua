-- src/domain/readset.lua —— 「推进必签名」的**必读集与纪元**读取面（r4 数据层）。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）。
-- 本模块写的 KV 前缀：无（只经 src/core/clock.lua 封存纪元；自身零直写）。
--
-- 分层（与 gates 的契约）：本模块只回答「该读什么 / 当前 ver 是什么 / 窗口多大 / 纪元何时开始」，
-- **不做判定、不产文案**——四维判定与四条拒台词在 `src/domain/gates.lua` 的 read_problem。
-- 这样 gates 是唯一裁判，本模块可被 render / 菜单 / 测试单独复用。
--
-- 数据来源（全部**内嵌在 data/config.json 的生成区**，由 scripts/gen-scene-readfirst.py
-- 与文档固定头同一批产出；Mod 无 IO，永远不读 docs/）：
--   readPlan            { "<幕>": ["<幕段>/<slug>", …] }        幕维度（r2/r3 已定契约）
--   readGate.ver        { "<doc-id>": "<6hex>" }                版本台账（r4：文档改版即作废签名）
--   readGate.reqActs    { "<阶段>": ["<doc-id>", …] }           需求阶段维度（源 = scenes.json reqStage）
--   readGate.gates      { "plan"|"accept": ["<doc-id>", …] }    门禁维度（= 该门禁交出下一幕的 req-acts）
--   readGate.window     遗忘窗口（缺省 80，运行期 `config.set readGate.window=<n>` 覆盖）
-- 生成区缺席 ⇒ 必读集为空 ⇒ 判据整体 inert（不拦人、不叙事）：这是分阶段上线的安全阀。

local M = {}

local clock = require("src.core.clock")
local config = require("src.core.config")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local effective =
  config.effective
local split, trim =
  util.split, util.trim
local req_count, req_key, req_plan_ok, req_rev, req_id, run_phase =
  reqs.req_count, reqs.req_key, reqs.req_plan_ok, reqs.req_rev, reqs.req_id, reqs.run_phase

-- 遗忘窗口缺省（用户裁决：50 → 80；四维拒台词里逐条报当前值）。
local DEFAULT_WINDOW = 80
local MAX_WINDOW = 1000

-- 必读集条目形状兜底：只收非空字符串、去重、保序（序 = 生成表序 = 断言可预期）。
local function push_doc(out, seen, raw)
  local doc = trim(type(raw) == "string" and raw or "")
  if doc == "" or seen[doc] then
    return
  end
  seen[doc] = true
  out[#out + 1] = doc
end

-- 生城区里的一维清单：缺失 / 形状不对 → 空表（**降级而非报错**：判据不能因表坏而死锁）。
local function docs_at(path)
  local raw = effective(path)
  if type(raw) ~= "table" then
    return {}
  end
  local out, seen = {}, {}
  for i = 1, #raw do
    push_doc(out, seen, raw[i])
  end
  return out
end

-- 需求「阶段」复合标量：plan_ok 与 integrating 一有变化就是新阶段纪元。
-- 刻意不含 job / gate：它们每次提案都动，含进来会让「刚签完就被判跨阶段」。
local function stage_flag(index)
  local integrating = ygg.kv.get(req_key(index, "integrating")) == true
  return tostring(req_plan_ok(index)) .. "|" .. tostring(integrating)
end

-- 纪元边沿探测：每次派发跑一遍（render 的收口 + 判据入口各一次，幂等）。
-- 不碰任何转场代码：写方（run.begin / gate.approve / amend / integration.* …）不需要知道 r4 存在。
function M.observe()
  clock.seal("act", tostring(run_phase()))
  local total = req_count()
  for i = 1, total do
    clock.seal("req." .. tostring(i) .. ".rev", tostring(req_rev(i)))
    clock.seal("req." .. tostring(i) .. ".stage", stage_flag(i))
  end
end

-- 遗忘窗口（`settings.readGate.window` 覆盖静态；越界夹紧，绝不因坏值变成「永不遗忘」）。
function M.window()
  local raw = effective("readGate.window")
  local value = tonumber(raw)
  if value == nil then
    return DEFAULT_WINDOW
  end
  value = math.floor(value)
  if value < 0 then
    return 0
  end
  if value > MAX_WINDOW then
    return MAX_WINDOW
  end
  return value
end

M.DEFAULT_WINDOW = DEFAULT_WINDOW
M.MAX_WINDOW = MAX_WINDOW

-- 版本台账（文档头 ver 的投影）。缺席 → ""（覆盖维度降级为「签过即算」，由 gates 叙事告警）。
-- 必用 `effective` 而不是 `text_of`：后者是**裸 KV 读**（`settings.` 前缀已剥），拿不到
-- 静态生成区；台账住在 data/config.json 里，只有 `effective` 才回落到 CONFIG 静态默认。
function M.doc_ver(doc)
  local value = effective("readGate.ver." .. tostring(doc))
  if type(value) == "string" then
    return trim(value)
  end
  return ""
end

-- 版本台账是否在场（判据据此决定能否给出「确切的 config.set 命令」）。
function M.has_ledger()
  return type(effective("readGate.ver")) == "table"
end

-- 本动作的必读集（三动作各一条口径；未知动作 → 空 = 不拦）。
function M.bearing(action, ctx)
  local out, seen = {}, {}
  if action == "proposal.submit" then
    -- 本幕幕前必读（acts/* + 本幕 req-acts）+ 本提案 stage 对应的 req-acts。
    for _, doc in ipairs(docs_at("readPlan." .. tostring(run_phase()))) do
      push_doc(out, seen, doc)
    end
    for _, doc in ipairs(docs_at("readGate.reqActs." .. tostring(ctx.stage or ""))) do
      push_doc(out, seen, doc)
    end
  elseif action == "gate.approve" then
    -- 当前门禁交出的是「下一幕的规则」：plan 门禁 → req-acts/delivery，accept 门禁 → req-acts/integration。
    for _, doc in ipairs(docs_at("readGate.gates." .. tostring(ctx.kind or ""))) do
      push_doc(out, seen, doc)
    end
  elseif action == "coordinate.submit" then
    -- 批边界：结算幕必读（acts/settle + rituals/cleanup 由生成器写在 readPlan.settle 里）。
    for _, doc in ipairs(docs_at("readPlan.settle")) do
      push_doc(out, seen, doc)
    end
  end
  return out
end

-- 一个动作涉及的纪元（供 gates 组装四维判据；顺序 = 判定优先级）。
-- 纪元集合只由 ctx.req 决定（幕级动作没有需求级维度），所以签名只收 ctx：
--   act    —— 运行幕（跨幕作废）
--   stage  —— 该需求的阶段（plan → development → integrating）
--   rev    —— 该需求的修订号（amend / 打回：主体维度）
function M.epochs(ctx)
  local scene = tostring(run_phase())
  local out = {
    { id = "act", seq = clock.epoch_seq("act"), scope = "scene " .. scene, kind = "scene" },
  }
  if ctx.req ~= nil then
    local index = ctx.req
    -- 阶段标签用人看的名字（plan / development），不用内部复合值。
    local stage_name = req_plan_ok(index) and "development" or "plan"
    out[#out + 1] = {
      id = "req." .. tostring(index) .. ".stage",
      seq = clock.epoch_seq("req." .. tostring(index) .. ".stage"),
      scope = "stage " .. stage_name,
      kind = "stage",
    }
    out[#out + 1] = {
      id = "req." .. tostring(index) .. ".rev",
      seq = clock.epoch_seq("req." .. tostring(index) .. ".rev"),
      scope = "rev " .. tostring(req_rev(index)),
      kind = "rev",
      req = req_id(index),
      rev = req_rev(index),
    }
  end
  return out
end

-- 幕内已签清单（叙事 / 菜单用；不从判定路径走）。
function M.signed_docs()
  local raw = ygg.kv.get("settings.read_index")
  local out = {}
  for _, part in ipairs(split(type(raw) == "string" and raw or "", ",")) do
    local doc = trim(part)
    if doc ~= "" then
      out[#out + 1] = doc
    end
  end
  return out
end

return M
