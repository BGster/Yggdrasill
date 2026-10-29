-- packages/mods/agent-seed/mod.lua
-- M-Agent 泳道 A：冷启动 seed mod —— 通用 agent 元工作流 Mod。
--
-- 定位：Yggdrasill 内核除游戏外亦作 **Agent harness**——任务与状态流转配置为
-- Mod，Agent 在 Mod 构造的工作流里「游玩」。本 Mod 是通用冷启动层：任何 Agent
-- 加载它 + 注入任务描述即可开工，零定制。核心理念：pendingChoice = 当前允许
-- 的下一步动作集（协议约束，不是提示词约束）；每个状态回执 = 处境叙事 +
-- 进度（KV 现算）+ 可选动作 + 指导文本（现场性）。
--
-- 状态机（六态，ygg.fsm 单机单事件 stage.advance，payload = 目标态）：
--
--   intake ──task.intake──▶ decompose ──plan.submit──▶ implement
--                                                         │  ▲
--     slice.report：非最后一片 → 同态继续（不转场）          │  │ verify.submit 失败
--     slice.report：最后一片   → verify                    ▼  │ （打回，重做最后一片）
--                                                       verify
--                                                         │
--                         verify.submit：全绿 / 门禁宽放 ▼
--                                                       review ──review.note──▶ done（局末【结局）
--
-- 动作协议（外部事件 = Agent 提交的结构化动作；载荷受内核约束只能是 JSON 标量，
-- 故本 Mod 定义字符串 wire format——见 §动作解析）：
--
--   task.intake   "title|description"                        intake    → decompose
--   plan.submit   "s1=标题; s2=标题; …"（1..max_slices 片）   decompose → implement
--   slice.report  "slice_id|summary|artifacts"               implement → implement / verify
--   verify.submit "true" / "false" / "all_green=…;report=…"  verify    → review / implement
--   review.note   "lesson|score(1-5)"                        review    → done（局末【结局）
--   config.set    "settings.<key>=<value>"                    任意态（常驻动作，不改主状态）
--   guide.show    "<section id>"（空 = 本阶段默认节）          任意态（常驻动作，不改主状态）
--
-- 时序纪律（两条内核语义，实证沉淀）：
--   1. `ygg.choice` 是 handler 的**最后一个输出**（终结语义）——本文件一律
--      「守卫 → 副作用 → narrate → advance()」，渲染（narrate + choice）只发生在
--      node:render 里，或拒绝/配置路径的末尾直调；
--   2. 派生事件按 FIFO 排空——`advance()` 先 emit `stage.advance`（fsm 转场）再
--      `goto node:render`（渲染），故渲染读到的一定是**已转场**的新态。
--
-- 两层配置：data/config.json（静态默认，ygg.data 装载期快照）+ KV `settings.*`
-- （config.set 动态覆盖）；判定恒为「动态覆盖 > 静态默认」。配置控制流转：
-- gates.require_all_green 决定门禁失败是否打回；evolution.mode 决定局末复盘的
-- 修订口径（off 仅报告 / suggest 建议包 / auto 白名单内自动修订）。

-- ============================================================================
-- §资源 装载期快照（ygg.data 在 host 之后、stdlib 之前安装，Mod 顶层即可读；
-- handler 只消费 local 闭包，资源只读、不入档）。
-- ============================================================================
local TEXTS = ygg.data("ygg:scene/texts")
local CONFIG = ygg.data("ygg:agent/config")
-- 现场规则手册（AGENTS.md 迁移来的规则资源；sections 为有序数组，见 §指南）。
local PLAYBOOK = ygg.data("ygg:agent/playbook")

-- ============================================================================
-- §工具 沙箱无 `table` 库（只暴露 base 纯计算子集；string 库经字符串元表仍可达），
-- 故切片/拼接/取数自备小工具——全部纯函数，无副作用。
-- ============================================================================
local function fill(template, vars)
  if type(template) ~= "string" then
    return ""
  end
  -- {{key}} 模板填充；未知键原样保留（漏填在叙事里可见 = 不静默丢字）。
  -- 字符类必须含下划线：`%w` 不含 `_`，而 done_ids / slice_id / verify_state
  -- 这类键全带下划线（初版漏 `_` 导致占位符原样漏进叙事，逐行走查揪出）。
  return (template:gsub("{{([%w_]+)}}", function(key)
    local value = vars[key]
    if value == nil then
      return "{{" .. key .. "}}"
    end
    return tostring(value)
  end))
end

local function split(text, sep)
  local out = {}
  if type(text) ~= "string" or text == "" then
    return out
  end
  local start = 1
  while true do
    local index, last = text:find(sep, start, true)
    if index == nil then
      out[#out + 1] = text:sub(start)
      return out
    end
    out[#out + 1] = text:sub(start, index - 1)
    start = last + 1
  end
end

local function cut(text, sep)
  local index, last = text:find(sep, 1, true)
  if index == nil then
    return text, nil
  end
  return text:sub(1, index - 1), text:sub(last + 1)
end

local function trim(text)
  if type(text) ~= "string" then
    return ""
  end
  return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function join(items, sep)
  local out = ""
  for i = 1, #items do
    if i > 1 then
      out = out .. sep
    end
    out = out .. tostring(items[i])
  end
  return out
end

local function contains(list, value)
  for i = 1, #list do
    if list[i] == value then
      return true
    end
  end
  return false
end

local function to_bool(text)
  if text == "true" then
    return true
  end
  if text == "false" then
    return false
  end
  return nil
end

-- ============================================================================
-- §状态 阶段单一事实源 = KV `fsm.stage`（由 ygg.fsm 写；未触发过时键缺席 →
-- 回落 initial，与 fsm 的 initial 声明同源）。
-- ============================================================================
local INITIAL = "intake"

-- 阶段常量（拼写集中于此，避免散落字符串漂移）。
local STAGE = {
  intake = "intake",
  decompose = "decompose",
  implement = "implement",
  verify = "verify",
  review = "review",
  done = "done",
}

local function stage()
  local value = ygg.kv.get("fsm.stage")
  if type(value) ~= "string" then
    return INITIAL
  end
  return value
end

-- ============================================================================
-- §配置 两层配置的判定面：动态覆盖（KV `settings.<path>`）> 静态默认（config.json）。
-- ============================================================================
local function static(path)
  local node = CONFIG
  for _, part in ipairs(split(path, ".")) do
    if type(node) ~= "table" then
      return nil
    end
    node = node[part]
  end
  return node
end

local function effective(path)
  local override = ygg.kv.get("settings." .. path)
  if override ~= nil then
    return override
  end
  return static(path)
end

local MODES = { "off", "suggest", "auto" }

local function mode()
  local value = effective("evolution.mode")
  if type(value) == "string" then
    return value
  end
  return "off"
end

local function next_mode()
  local current = mode()
  for i = 1, #MODES do
    if MODES[i] == current then
      return MODES[(i % #MODES) + 1]
    end
  end
  return MODES[1]
end

local function strict_gates()
  return effective("gates.require_all_green") == true
end

local function max_per_run()
  local value = effective("evolution.max_per_run")
  if type(value) == "number" then
    return value
  end
  return 0
end

local function gates_word()
  if strict_gates() then
    return TEXTS.beats.gate_strict
  end
  return TEXTS.beats.gate_lenient
end

local function allow_paths_text()
  local paths = CONFIG.evolution.allow_paths
  if type(paths) ~= "table" then
    return TEXTS.beats.paths_none
  end
  return join(paths, "、")
end

-- ============================================================================
-- §进度 进度一律从 KV 现算（不缓存进 local：local 不随存档往返，KV 才是记忆）。
-- ============================================================================
local function num(key, fallback)
  local value = ygg.kv.get(key)
  if type(value) == "number" then
    return value
  end
  return fallback
end

local function text(key, fallback)
  local value = ygg.kv.get(key)
  if type(value) == "string" and value ~= "" then
    return value
  end
  return fallback
end

local function task_title()
  return text("task.title", TEXTS.beats.default_task_title)
end

local function task_desc()
  return text("task.description", TEXTS.beats.default_task_desc)
end

local function plan_count()
  return num("plan.count", 0)
end

local function plan_done()
  return num("plan.done", 0)
end

local function next_index()
  return plan_done() + 1
end

local function slice_id(index)
  return text("slice." .. tostring(index) .. ".id", "s" .. tostring(index))
end

local function slice_title(index)
  return text("slice." .. tostring(index) .. ".title", TEXTS.beats.no_slice)
end

local function done_ids()
  local ids = {}
  for i = 1, plan_done() do
    ids[#ids + 1] = slice_id(i)
  end
  if #ids == 0 then
    return TEXTS.beats.none_done
  end
  return join(ids, "、")
end

local function review_lesson()
  return text("review.lesson", TEXTS.beats.default_lesson)
end

local function verify_state_word()
  return text("verify.last", TEXTS.beats.verify_state_none)
end

-- ============================================================================
-- §指南 现场规则（ygg:agent/playbook）——AGENTS.md 迁移来的规则资源。
-- sections 是**有序数组**（Lua table 遍历序不稳定，顺序由 order 字段承载），
-- 此处建 id→section 索引供 guide.show 直取，并按阶段轮换默认节：每阶段回执
-- 只提示一节，全文按需 guide.show 拉——「规则在现场可查询」而非开局宣读。
-- ============================================================================
local SECTIONS = {}    -- section id → section
local SECTION_IDS = {} -- 有序合法 id 列表（非法 id 回执要列全）
for i = 1, #PLAYBOOK.sections do
  local entry = PLAYBOOK.sections[i]
  SECTIONS[entry.id] = entry
  SECTION_IDS[#SECTION_IDS + 1] = entry.id
end

-- 阶段 → 默认节候选（多候选按阶段内进度轮换；只读 KV 计数，不写状态——
-- 渲染路径必须无副作用，否则同一步重渲染会漂移）。
local STAGE_GUIDE = {
  intake = { "output" },
  decompose = { "integration" },
  implement = { "repository", "lanes", "dispatch" },
  verify = { "gates", "live_evidence" },
  review = { "evidence", "hard_constraints" },
  done = { "pointers" },
}

local function guide_cursor(st)
  if st == STAGE.implement then
    return next_index() -- 每片换一节：repository → lanes → dispatch
  elseif st == STAGE.verify then
    return num("verify.failures", 0) + 1 -- 首查 gates，被打回后转 live_evidence
  elseif st == STAGE.review then
    return plan_count() -- 按计划规模定节（确定性；纯读）
  end
  return 1
end

local function guide_section(st)
  local candidates = STAGE_GUIDE[st]
  local section = nil
  if candidates ~= nil and #candidates > 0 then
    local index = ((guide_cursor(st) - 1) % #candidates) + 1
    section = SECTIONS[candidates[index]]
  end
  if section == nil then
    -- 资源缺节（数据漂移的防御面）：回落首节——schema minItems 1 已挡住空表。
    section = PLAYBOOK.sections[1]
  end
  return section
end

local MODALITY_MARK = {
  ["must"] = TEXTS.beats.modality_must,
  ["must-not"] = TEXTS.beats.modality_must_not,
  ["should"] = TEXTS.beats.modality_should,
}

local GATE_MARK = {
  required = TEXTS.beats.gate_required,
  optional = TEXTS.beats.gate_optional,
  pending = TEXTS.beats.gate_pending,
}

-- 可选值的两种收口（值缺省就整段不出现，不留空模板痕迹）：
-- suffix = 拼进上一行；narrate_optional = 自成一行且仅在非空时 narrate。
local function suffix(template, value)
  if type(value) ~= "string" or value == "" then
    return ""
  end
  return fill(template, { value = value })
end

local function narrate_optional(template, value)
  if type(value) ~= "string" or value == "" then
    return
  end
  ygg.narrate(fill(template, { value = value }))
end

-- 一节全文的叙事面：标题摘要 → 原子规则 → 适用阶段 → 布局 → 命令 → 指针 →
-- 备注。逐条 narrate（不拼大段字符串）：回执行数与规则数一一对应，便于
-- Agent 按行定位。
local function narrate_section(section)
  ygg.narrate(fill(TEXTS.beats.guide_head, {
    title = section.title,
    summary = section.summary,
    count = tostring(#section.rules),
    source = tostring(section.source),
  }))
  for i = 1, #section.rules do
    local rule = section.rules[i]
    ygg.narrate(fill(TEXTS.beats.guide_rule, {
      mark = MODALITY_MARK[rule.modality] or "",
      text = rule.text,
      basis = suffix(TEXTS.beats.guide_basis, rule.basis),
    }))
  end
  if section.applies_to ~= nil then
    ygg.narrate(fill(TEXTS.beats.guide_stages, { stages = join(section.applies_to, "、") }))
  end
  if section.layout ~= nil then
    for i = 1, #section.layout do
      local entry = section.layout[i]
      local flag = ""
      if entry.status == "planned" then
        flag = TEXTS.beats.guide_planned
      end
      ygg.narrate(fill(TEXTS.beats.guide_layout, {
        path = entry.path,
        role = entry.role,
        flag = flag,
      }))
    end
  end
  if section.commands ~= nil then
    for i = 1, #section.commands do
      local command = section.commands[i]
      local gate = ""
      if command.gate ~= nil then
        gate = fill(TEXTS.beats.guide_gate, {
          gate = GATE_MARK[command.gate] or tostring(command.gate),
        })
      end
      ygg.narrate(fill(TEXTS.beats.guide_command, {
        label = command.label,
        gate = gate,
        run = command.run,
      }))
      narrate_optional(TEXTS.beats.guide_command_precondition, command.precondition)
      narrate_optional(TEXTS.beats.guide_command_note, command.note)
    end
  end
  if section.pointers ~= nil then
    for i = 1, #section.pointers do
      local pointer = section.pointers[i]
      ygg.narrate(fill(TEXTS.beats.guide_pointer, {
        goal = pointer.goal,
        where = pointer.where,
        note = suffix(TEXTS.beats.guide_pointer_note, pointer.note),
      }))
    end
  end
  if type(section.notes) == "string" and section.notes ~= "" then
    ygg.narrate(fill(TEXTS.beats.guide_notes, { notes = section.notes }))
  end
  ygg.narrate(fill(TEXTS.beats.guide_tail, {
    id = section.id,
    total = tostring(#SECTION_IDS),
  }))
end

-- 返回 false = 非法 id（合法清单已 narrate）。两种结果都不改变任何状态。
local function narrate_guide(id)
  local section = SECTIONS[id]
  if section == nil then
    ygg.narrate(fill(TEXTS.beats.guide_unknown, {
      id = tostring(id),
      total = tostring(#SECTION_IDS),
      ids = join(SECTION_IDS, "、"),
    }))
    return false
  end
  narrate_section(section)
  return true
end

-- 阶段回执里的一行现场规则（只一行：全文留给 guide.show）。
local function narrate_stage_guide(st)
  local section = guide_section(st)
  if section == nil then
    return
  end
  ygg.narrate(fill(TEXTS.beats.guide_hint, {
    title = section.title,
    summary = section.summary,
    count = tostring(#section.rules),
  }))
end

-- 菜单入口：文案与载荷同源（与 MENU 其它项同一口径）。
local function guide_item(st)
  local section = guide_section(st)
  if section == nil then
    return nil
  end
  return {
    text = fill(TEXTS.options.guide, {
      title = section.title,
      count = tostring(#section.rules),
    }),
    action = "guide.show",
    payload = section.id,
  }
end

-- ============================================================================
-- §菜单 选项文本 ↔ 动作载荷的**单一来源**：同一张 MENU 表既产出 pendingChoice
-- 的选项文案，又决定选中序号对应的动作与默认载荷（杜绝文案/动作漂移）。
-- 规则：每态第一条 = 主前进动作（全 1 贪心走查可通关）；guide.show 紧随主前进
-- 动作（verify 态排在其失败分支之后），config.set 常驻且置末。
-- ============================================================================
local MENU = {}

local function config_evolution_item()
  local next_value = next_mode()
  return {
    text = fill(TEXTS.options.config_evolution, { mode = mode(), next = next_value }),
    action = "config.set",
    payload = "evolution.mode=" .. next_value,
  }
end

local function config_gates_item()
  local current = strict_gates()
  local next_value = not current
  return {
    text = fill(TEXTS.options.config_gates, {
      state = tostring(current),
      next = tostring(next_value),
    }),
    action = "config.set",
    payload = "gates.require_all_green=" .. tostring(next_value),
  }
end

MENU[STAGE.intake] = function()
  return {
    {
      text = TEXTS.options.intake,
      action = "task.intake",
      payload = TEXTS.beats.default_task_title .. "|" .. TEXTS.beats.default_task_desc,
    },
    guide_item(STAGE.intake),
    config_evolution_item(),
    config_gates_item(),
  }
end

MENU[STAGE.decompose] = function()
  return {
    {
      text = fill(TEXTS.options.plan_submit, { max = tostring(TEXTS.tuning.max_slices) }),
      action = "plan.submit",
      payload = TEXTS.beats.default_plan,
    },
    guide_item(STAGE.decompose),
    config_evolution_item(),
    config_gates_item(),
  }
end

MENU[STAGE.implement] = function()
  local id = slice_id(next_index())
  return {
    {
      text = fill(TEXTS.options.slice_report, { id = id }),
      action = "slice.report",
      payload = id .. "|" .. TEXTS.beats.default_summary .. "|" .. TEXTS.beats.default_artifacts,
    },
    guide_item(STAGE.implement),
    config_evolution_item(),
    config_gates_item(),
  }
end

MENU[STAGE.verify] = function()
  return {
    -- 第一条 = 主前进（全绿 → review）；第二条 = 如实报错（可能打回 implement）。
    { text = TEXTS.options.verify_green, action = "verify.submit", payload = "true" },
    {
      text = TEXTS.options.verify_fail,
      action = "verify.submit",
      payload = "all_green=false;report=" .. TEXTS.beats.default_gate_report,
    },
    guide_item(STAGE.verify),
    config_gates_item(),
    config_evolution_item(),
  }
end

MENU[STAGE.review] = function()
  return {
    {
      text = TEXTS.options.review_note,
      action = "review.note",
      payload = TEXTS.beats.default_lesson .. "|" .. tostring(TEXTS.tuning.default_score),
    },
    guide_item(STAGE.review),
    config_evolution_item(),
    config_gates_item(),
  }
end

local function menu(st)
  local builder = MENU[st]
  if builder == nil then
    return nil -- done（局末）无待选动作
  end
  return builder()
end

local function option_texts(items)
  local out = {}
  for i = 1, #items do
    out[i] = items[i].text
  end
  return out
end

-- ============================================================================
-- §渲染 每个状态回执 = 处境（scenes）+ 指导（beats，第二人称）+ 可选动作（choice）。
-- 渲染器只读 KV、只 narrate/choice——不写状态、不 emit（choice 必须是最后输出）。
-- ============================================================================
local RENDER = {}

local function evolution_receipt()
  local current = mode()
  local used = num("evolution.used", 0)
  local cap = max_per_run()
  local paths = allow_paths_text()
  if current == "auto" then
    if ygg.kv.get("evolution.requested") == true then
      return fill(TEXTS.beats.evolution_auto, {
        paths = paths,
        used = tostring(used),
        cap = tostring(cap),
      })
    end
    if used >= cap then
      return fill(TEXTS.beats.evolution_capped, { paths = paths, cap = tostring(cap) })
    end
    return TEXTS.beats.evolution_auto_pending
  elseif current == "suggest" then
    return fill(TEXTS.beats.evolution_suggest, { paths = paths })
  end
  return TEXTS.beats.evolution_off
end

RENDER[STAGE.intake] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.intake, {
    stage = current,
    mode = mode(),
    gates = gates_word(),
  }))
  ygg.narrate(fill(TEXTS.beats.intake_guide, {
    min = tostring(TEXTS.tuning.min_slices),
    max = tostring(TEXTS.tuning.max_slices),
  }))
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.intake, option_texts(menu(current)))
end

RENDER[STAGE.decompose] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.decompose, {
    title = task_title(),
    description = task_desc(),
    min = tostring(TEXTS.tuning.min_slices),
    max = tostring(TEXTS.tuning.max_slices),
    mode = mode(),
  }))
  ygg.narrate(fill(TEXTS.beats.decompose_guide, {
    min = tostring(TEXTS.tuning.min_slices),
    max = tostring(TEXTS.tuning.max_slices),
  }))
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.decompose, option_texts(menu(current)))
end

RENDER[STAGE.implement] = function()
  local current = stage()
  local done = plan_done()
  local index = next_index()
  ygg.narrate(fill(TEXTS.scenes.implement, {
    next = tostring(index),
    count = tostring(plan_count()),
    slice = slice_title(index),
    done = tostring(done),
    done_ids = done_ids(),
    mode = mode(),
  }))
  ygg.narrate(fill(TEXTS.beats.implement_guide, {
    slice_id = slice_id(index),
    mode = mode(),
  }))
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.implement, option_texts(menu(current)))
end

RENDER[STAGE.verify] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.verify, {
    count = tostring(plan_count()),
    done_ids = done_ids(),
    verify_state = verify_state_word(),
    failures = tostring(num("verify.failures", 0)),
    gates = gates_word(),
  }))
  ygg.narrate(fill(TEXTS.beats.verify_guide, { gates = gates_word() }))
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.verify, option_texts(menu(current)))
end

RENDER[STAGE.review] = function()
  local current = stage()
  ygg.narrate(fill(TEXTS.scenes.review, {
    title = task_title(),
    count = tostring(plan_count()),
    failures = tostring(num("verify.failures", 0)),
    mode = mode(),
  }))
  -- 演进回执：按当前 effective 配置分支（auto 已修订 / auto 额度用尽 /
  -- auto 本局未触发 / suggest 建议包 / off 仅报告）——这就是「配置控制
  -- 状态回执」的观测面，不做提示词式的笼统描述。
  ygg.narrate(evolution_receipt())
  -- 红线在现场：局末报告同样属于入库面，只允许键名（秘密只在 .env / .env.local）。
  ygg.narrate(TEXTS.beats.secret_redline)
  ygg.narrate(fill(TEXTS.beats.review_guide, { score = tostring(TEXTS.tuning.default_score) }))
  narrate_stage_guide(current)
  ygg.choice(TEXTS.prompts.review, option_texts(menu(current)))
end

-- 局末：叙事行以【结局 前缀收口（无头走查 ENDING_PREFIX 判据），且不再 choice
-- （pendingChoice 为空 = 该局没有下一个动作）。
RENDER[STAGE.done] = function()
  ygg.narrate(fill(TEXTS.endings.done, {
    title = task_title(),
    count = tostring(plan_count()),
    failures = tostring(num("verify.failures", 0)),
    score = tostring(num("review.score", TEXTS.tuning.default_score)),
    mode = mode(),
    lesson = review_lesson(),
  }))
end

local function render()
  local current = stage()
  local renderer = RENDER[current]
  if renderer == nil then
    ygg.narrate(fill(TEXTS.beats.unknown_stage, { stage = tostring(current) }))
    renderer = RENDER[STAGE.intake]
  end
  renderer()
end

-- ============================================================================
-- §推进 转场 + 渲染的唯二出口（顺序即语义，见文件头「时序纪律」）。
-- ============================================================================
local function advance(target)
  if target ~= stage() then
    -- fsm 转场先入 FIFO（stage.advance 由 stdlib fsm handler 消费）。
    ygg.emit("stage.advance", target)
  end
  -- 渲染后入 FIFO：排到此处时 fsm 已转场完毕，渲染读到新态。
  ygg.narrative["goto"]("node:render")
end

local function allowed_list(current)
  local items = menu(current)
  if items == nil then
    return TEXTS.beats.allowed_none
  end
  -- 去重：同一动作可能有两种载荷选项（如 verify.submit 全绿 / 失败），
  -- 回执只说「允许哪些动作」，不重复列同名项。
  local names = {}
  for i = 1, #items do
    if not contains(names, items[i].action) then
      names[#names + 1] = items[i].action
    end
  end
  return join(names, "、")
end

-- 拒绝回执的两个面（收尾同一：narrate + 原地重渲染，主状态不变）：
--   reject         = 越阶动作（当前阶段不允许这个动作）
--   reject_payload = 动作允许但载荷/参数不接受
-- 注意 render() 以 choice 收尾 → handler 以终结语义正常结束（无 emit 前置）。
local function refuse(template, action, detail)
  local current = stage()
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

-- ============================================================================
-- §动作解析 wire format（内核 payload 只有四个标量，复合载荷压成单字符串）。
-- 每个解析器返回 (值…, problem)：problem 非 nil 时由调用方走 reject_payload
--（动作本身在阶段内合法，只是载荷不接受——两种拒绝回执不能混）。
-- ============================================================================
local function parse_intake(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "task.intake" })
  end
  local raw = trim(payload)
  if raw == "" then
    return TEXTS.beats.default_task_title, TEXTS.beats.default_task_desc, nil
  end
  local title, description = cut(raw, "|")
  title = trim(title)
  if title == "" then
    title = TEXTS.beats.default_task_title
  end
  description = trim(description or "")
  if description == "" then
    description = TEXTS.beats.default_task_desc
  end
  return title, description, nil
end

local function valid_slice_id(id)
  if id == "" then
    return false
  end
  for _, banned in ipairs({ "|", ";", "=" }) do
    if id:find(banned, 1, true) then
      return false
    end
  end
  return true
end

local function parse_plan(payload)
  if type(payload) ~= "string" then
    return nil, fill(TEXTS.beats.bad_payload, { action = "plan.submit" })
  end
  local raw = trim(payload)
  if raw == "" then
    raw = TEXTS.beats.default_plan
  end
  local slices = {}
  for _, segment in ipairs(split(raw, ";")) do
    local piece = trim(segment)
    if piece ~= "" then
      local head, tail = cut(piece, "=")
      local id, title
      if tail == nil or trim(tail) == "" then
        -- 只给标题：自动编号（序号 = 已收片数 + 1，只算非空段）。
        id = "s" .. tostring(#slices + 1)
        title = piece
      else
        id = trim(head)
        title = trim(tail)
      end
      if not valid_slice_id(id) then
        return nil, fill(TEXTS.beats.bad_slice_id, { id = id })
      end
      for _, other in ipairs(slices) do
        if other.id == id then
          return nil, fill(TEXTS.beats.dup_slice_id, { id = id })
        end
      end
      if title == "" then
        return nil, fill(TEXTS.beats.bad_slice_title, { id = id })
      end
      slices[#slices + 1] = { id = id, title = title }
    end
  end
  local min_slices = TEXTS.tuning.min_slices
  local max_slices = TEXTS.tuning.max_slices
  if #slices < min_slices or #slices > max_slices then
    return nil, fill(TEXTS.beats.bad_slice_count, {
      count = tostring(#slices),
      min = tostring(min_slices),
      max = tostring(max_slices),
    })
  end
  return slices, nil
end

local function parse_slice_report(payload)
  if type(payload) ~= "string" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "slice.report" })
  end
  local head, rest = cut(trim(payload), "|")
  local id = trim(head)
  if id == "" then
    return nil, nil, nil, fill(TEXTS.beats.bad_payload, { action = "slice.report" })
  end
  local summary = TEXTS.beats.default_summary
  local artifacts = TEXTS.beats.default_artifacts
  if rest ~= nil then
    local body, tail = cut(rest, "|")
    if trim(body) ~= "" then
      summary = trim(body)
    end
    if tail ~= nil and trim(tail) ~= "" then
      artifacts = trim(tail)
    end
  end
  return id, summary, artifacts, nil
end

local function parse_verify(payload)
  if type(payload) == "boolean" then
    return payload, "", nil
  end
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "verify.submit" })
  end
  local raw = trim(payload)
  if raw == "true" then
    return true, "", nil
  end
  if raw == "false" then
    return false, "", nil
  end
  local all_green = nil
  local report = ""
  for _, segment in ipairs(split(raw, ";")) do
    local piece = trim(segment)
    if piece ~= "" then
      local key, value = cut(piece, "=")
      key = trim(key)
      value = trim(value or "")
      if key == "all_green" then
        all_green = (value == "true")
      elseif key == "report" then
        report = value
      end
    end
  end
  if all_green == nil then
    return nil, nil, fill(TEXTS.beats.bad_verify_payload, { got = raw })
  end
  return all_green, report, nil
end

local function parse_review(payload)
  if type(payload) ~= "string" then
    return nil, nil, fill(TEXTS.beats.bad_payload, { action = "review.note" })
  end
  local lesson, score_text = cut(trim(payload), "|")
  lesson = trim(lesson)
  if lesson == "" then
    lesson = TEXTS.beats.default_lesson
  end
  local score = TEXTS.tuning.default_score
  local candidate = trim(score_text or "")
  if candidate ~= "" then
    local parsed = tonumber(candidate)
    if parsed == nil then
      return nil, nil, fill(TEXTS.beats.bad_score, { got = candidate })
    end
    score = math.floor(parsed)
    if score < 1 or score > 5 then
      return nil, nil, fill(TEXTS.beats.bad_score, { got = candidate })
    end
  end
  return lesson, score, nil
end

-- ============================================================================
-- §配置写入 config.set 的校验 + 落 KV；返回 problem（nil = 成功）。
-- 只认 settings.* 命名空间下的白名单键（其余可读拒绝，不改任何状态）。
-- ============================================================================
local function config_scope(key)
  if key == "evolution.mode" then
    return TEXTS.beats.scope_evolution_mode
  elseif key == "evolution.max_per_run" then
    return TEXTS.beats.scope_evolution_cap
  elseif key == "gates.require_all_green" then
    return TEXTS.beats.scope_gates
  end
  return TEXTS.beats.scope_host
end

local function apply_config(key, value)
  if key == "evolution.mode" then
    if not contains(MODES, value) then
      return fill(TEXTS.beats.bad_config_value, {
        key = key,
        value = value,
        allowed = join(MODES, " / "),
      })
    end
    ygg.kv.set("settings.evolution.mode", value)
    return nil
  elseif key == "evolution.max_per_run" then
    local parsed = tonumber(value)
    if parsed == nil then
      return fill(TEXTS.beats.bad_config_value, {
        key = key,
        value = value,
        allowed = "0–8 的整数",
      })
    end
    parsed = math.floor(parsed)
    if parsed < 0 or parsed > 8 then
      return fill(TEXTS.beats.bad_config_value, {
        key = key,
        value = value,
        allowed = "0–8 的整数",
      })
    end
    ygg.kv.set("settings.evolution.max_per_run", parsed)
    return nil
  elseif key == "gates.require_all_green" then
    local parsed = to_bool(value)
    if parsed == nil then
      return fill(TEXTS.beats.bad_config_value, {
        key = key,
        value = value,
        allowed = "true / false",
      })
    end
    ygg.kv.set("settings.gates.require_all_green", parsed)
    return nil
  elseif key == "autosave" or key == "hot_reload" then
    local parsed = to_bool(value)
    if parsed == nil then
      return fill(TEXTS.beats.bad_config_value, {
        key = key,
        value = value,
        allowed = "true / false",
      })
    end
    ygg.kv.set("settings." .. key, parsed)
    return nil
  end
  return fill(TEXTS.beats.bad_config_key, {
    key = key,
    allowed = TEXTS.beats.config_keys,
  })
end

-- 进入复盘的唯一出口：按**进入时刻**的演进模式登记本局自动修订请求。
-- （requested 是状态、mode 是配置——回执据此稳定，不随后续 config.set 抖动）。
-- 真实 revise 由宿主执行；内核无 IO，不能在此声称 Mod 文件已被改写。
local function enter_review()
  local used = num("evolution.used", 0)
  local cap = max_per_run()
  if mode() == "auto" and used < cap then
    ygg.kv.set("evolution.used", used + 1)
    ygg.kv.set("evolution.requested", true)
  end
  advance(STAGE.review)
end

-- ============================================================================
-- §动作 handler 每个 handler 的模式固定：阶段守卫 → 载荷解析 → 副作用 →
-- narrate → advance（拒绝路径 = reject → 原地重渲染）。
-- ============================================================================
ygg.on("task.intake", function(payload)
  local current = stage()
  if current ~= STAGE.intake then
    return reject("task.intake", fill(TEXTS.beats.stage_expected, { expected = STAGE.intake }))
  end
  local title, description, problem = parse_intake(payload)
  if problem ~= nil then
    return reject_payload("task.intake", problem)
  end
  ygg.kv.set("task.title", title)
  ygg.kv.set("task.description", description)
  ygg.narrate(fill(TEXTS.beats.intake_ok, { title = title }))
  advance(STAGE.decompose)
end)

ygg.on("plan.submit", function(payload)
  local current = stage()
  if current ~= STAGE.decompose then
    return reject("plan.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.decompose }))
  end
  local slices, problem = parse_plan(payload)
  if problem ~= nil then
    return reject_payload("plan.submit", problem)
  end
  for i = 1, #slices do
    ygg.kv.set("slice." .. tostring(i) .. ".id", slices[i].id)
    ygg.kv.set("slice." .. tostring(i) .. ".title", slices[i].title)
  end
  ygg.kv.set("plan.count", #slices)
  ygg.kv.set("plan.done", 0)
  local ids = {}
  for i = 1, #slices do
    ids[i] = slices[i].id
  end
  ygg.narrate(fill(TEXTS.beats.plan_ok, {
    ids = join(ids, "、"),
    count = tostring(#slices),
  }))
  advance(STAGE.implement)
end)

ygg.on("slice.report", function(payload)
  local current = stage()
  if current ~= STAGE.implement then
    return reject("slice.report", fill(TEXTS.beats.stage_expected, { expected = STAGE.implement }))
  end
  local id, summary, artifacts, problem = parse_slice_report(payload)
  if problem ~= nil then
    return reject_payload("slice.report", problem)
  end
  local index = next_index()
  local expected = slice_id(index)
  if id ~= expected then
    -- 顺序门：乱序上报不推进进度（切片是计划的有序前缀，不是集合）。
    return reject_payload("slice.report", fill(TEXTS.beats.slice_order, {
      expected = expected,
      got = id,
      index = tostring(index),
    }))
  end
  ygg.kv.set("slice." .. tostring(index) .. ".summary", summary)
  ygg.kv.set("slice." .. tostring(index) .. ".artifacts", artifacts)
  ygg.kv.set("plan.done", index)
  ygg.kv.set("slice.last", id)
  ygg.narrate(fill(TEXTS.beats.slice_ok, {
    id = id,
    summary = summary,
    done = tostring(index),
    count = tostring(plan_count()),
  }))
  if index >= plan_count() then
    ygg.narrate(fill(TEXTS.beats.slice_final, { count = tostring(plan_count()) }))
    advance(STAGE.verify)
  else
    advance(STAGE.implement) -- 同态推进：fsm 无自环边，只重渲染
  end
end)

ygg.on("verify.submit", function(payload)
  local current = stage()
  if current ~= STAGE.verify then
    return reject("verify.submit", fill(TEXTS.beats.stage_expected, { expected = STAGE.verify }))
  end
  local all_green, report, problem = parse_verify(payload)
  if problem ~= nil then
    return reject_payload("verify.submit", problem)
  end
  if all_green then
    ygg.kv.set("verify.last", TEXTS.beats.verify_green_record)
    ygg.narrate(fill(TEXTS.beats.verify_ok, { count = tostring(plan_count()) }))
    enter_review()
    return
  end
  local failures = num("verify.failures", 0) + 1
  ygg.kv.set("verify.failures", failures)
  if report ~= "" then
    ygg.kv.set("verify.last", report)
  else
    ygg.kv.set("verify.last", TEXTS.beats.verify_failed_record)
  end
  if strict_gates() then
    -- 打回：退回最后一片重做（plan.done 回退一格），失败计数保留。
    local count = plan_count()
    local redo = count
    if redo < 1 then
      redo = 1
    end
    ygg.kv.set("plan.done", redo - 1)
    ygg.narrate(fill(TEXTS.beats.verify_fail, {
      failures = tostring(failures),
      slice_id = slice_id(redo),
    }))
    advance(STAGE.implement)
  else
    -- 宽放：gates.require_all_green=false → 失败也放行进复盘（配置控制流转）。
    ygg.narrate(fill(TEXTS.beats.gate_bypass, {
      failures = tostring(failures),
      gates = gates_word(),
    }))
    enter_review()
  end
end)

ygg.on("review.note", function(payload)
  local current = stage()
  if current ~= STAGE.review then
    return reject("review.note", fill(TEXTS.beats.stage_expected, { expected = STAGE.review }))
  end
  local lesson, score, problem = parse_review(payload)
  if problem ~= nil then
    return reject_payload("review.note", problem)
  end
  ygg.kv.set("review.lesson", lesson)
  ygg.kv.set("review.score", score)
  ygg.kv.set("ending", STAGE.done) -- 局末标记（测试/宿主断言面）
  ygg.narrate(fill(TEXTS.beats.review_ok, { score = tostring(score), lesson = lesson }))
  advance(STAGE.done)
end)

ygg.on("config.set", function(payload)
  local current = stage()
  if type(payload) ~= "string" then
    return reject("config.set", fill(TEXTS.beats.bad_payload, { action = "config.set" }))
  end
  local key, value = cut(trim(payload), "=")
  key = trim(key)
  value = trim(value or "")
  -- 两种写法都收：`settings.evolution.mode` 与 `evolution.mode`（归一为前者）。
  if key:sub(1, 9) == "settings." then
    key = key:sub(10)
  end
  local problem = apply_config(key, value)
  if problem ~= nil then
    return reject_payload("config.set", problem)
  end
  ygg.narrate(fill(TEXTS.beats.config_changed, {
    key = key,
    value = tostring(effective(key)),
    scope = config_scope(key),
    stage = current,
  }))
  render() -- 常驻动作：主状态不变，回执重渲染当前阶段（choice 收尾）
end)

-- ============================================================================
-- §指南动作 guide.show（常驻动作，任意主状态可用）：载荷 = section id（如
-- "credentials"）；空载荷取**本阶段默认节**（与菜单入口同一来源，不漂移）。
-- 只 narrate 该节全文，不改任何进度状态；非法 id 回执合法清单后原样重渲染。
-- ============================================================================
ygg.on("guide.show", function(payload)
  local current = stage()
  local id = trim(payload)
  if id == "" then
    local section = guide_section(current)
    if section ~= nil then
      id = section.id
    end
  end
  narrate_guide(id) -- 非法 id 时该函数自己 narrate 合法清单（不改变状态）
  render()
end)

-- ============================================================================
-- §选择路由 pendingChoice 的回应面：序号 → MENU 的对应项（动作 + 默认载荷）→
-- 派发同名动作事件。动作事件的一切校验仍在各自的 handler 里（单一处理路径），
-- 因此「选项驱动」与「Agent 直接派发事件」走的是同一套守卫与副作用。
-- ============================================================================
ygg.on("ygg.choice_selected", function(index)
  local current = stage()
  local items = menu(current)
  if items == nil then
    ygg.narrate(fill(TEXTS.beats.unknown_choice, { stage = current }))
    render() -- 局末：只把结局叙事再放一遍（无后续动作）
    return
  end
  local picked = tonumber(index)
  if picked == nil or items[picked] == nil then
    ygg.narrate(fill(TEXTS.beats.bad_index, {
      stage = current,
      index = tostring(index),
      count = tostring(#items),
    }))
    render()
    return
  end
  ygg.emit(items[picked].action, items[picked].payload)
end)

ygg.on("event.start", function()
  ygg.narrate(fill(TEXTS.scenes.prologue, {
    engine = tostring(ygg.engine_version),
    stage = stage(),
    mode = mode(),
  }))
  ygg.narrate(fill(TEXTS.beats.prologue_guide, { mode = mode(), gates = gates_word() }))
  -- 红线前置：配置与成长产物都入库，只存非敏感配置。
  ygg.narrate(TEXTS.beats.secret_redline)
  render()
end)

-- 渲染节点：唯一入口（advance 以 ygg.narrative.goto 派生事件回到这里）。
ygg.narrative.node("node:render", function()
  render()
end)

-- ============================================================================
-- §状态机 单机单事件：payload = 目标态，边 = 五态流转图的合法转移（when 按目标
-- 态匹配 = 显式白名单）。非法目标 = 静默 no-op（守卫在 handler 里先拦，fsm 是
-- 第二道声明面）；状态落 KV `fsm.stage`，随存档往返。
-- ============================================================================
ygg.fsm.define({
  name = "stage",
  initial = INITIAL,
  event = "stage.advance",
  transitions = {
    {
      from = STAGE.intake,
      to = STAGE.decompose,
      when = function(target)
        return target == STAGE.decompose
      end,
    },
    {
      from = STAGE.decompose,
      to = STAGE.implement,
      when = function(target)
        return target == STAGE.implement
      end,
    },
    {
      from = STAGE.implement,
      to = STAGE.verify,
      when = function(target)
        return target == STAGE.verify
      end,
    },
    {
      from = STAGE.verify,
      to = STAGE.review,
      when = function(target)
        return target == STAGE.review
      end,
    },
    {
      -- 门禁失败打回（同机第二条出边，靠 when 与上一条区分目标态）。
      from = STAGE.verify,
      to = STAGE.implement,
      when = function(target)
        return target == STAGE.implement
      end,
    },
    {
      from = STAGE.review,
      to = STAGE.done,
      when = function(target)
        return target == STAGE.done
      end,
    },
  },
})
