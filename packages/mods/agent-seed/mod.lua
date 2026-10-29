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
--   map.show      "" / 0 / "full"（剧情 = 本六态节点图）       任意态（常驻动作，不改主状态）
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
-- play.mode 决定本局**交互模式**（guide 逐步确认 / goal 挂机）、
-- gates.require_all_green 决定门禁失败是否打回；evolution.mode 决定局末复盘的
-- 修订口径（off 仅报告 / suggest 建议包 / auto 白名单内自动修订）。
--
-- 交互模式（play.mode，静态缺省 guide）：每阶段的推进提示词按模式分叉——guide 版
-- 强调「确认后由你选择/提交」，goal 版强调「挂机，宿主按推荐主动作自动推进，无需
-- 确认」；每阶段回执顶部固定一行【运行模式】写明当前模式。模式只影响提示词与宿主
-- 推进节奏，**不改**状态机、动作协议与只读动作（config.set / map.show 行为不变）。

-- ============================================================================
-- §资源 装载期快照（ygg.data 在 host 之后、stdlib 之前安装，Mod 顶层即可读；
-- handler 只消费 local 闭包，资源只读、不入档）。
-- ============================================================================
local TEXTS = ygg.data("ygg:scene/texts")
local CONFIG = ygg.data("ygg:agent/config")

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

-- 交互模式（play.mode）：goal = 挂机（宿主按每步推荐主动作自动推进到结局并结算，
-- 无需逐步确认）；guide = 逐步确认（每一步推进都必须由人选择/提交）。缺省 guide——
-- 安全默认，不改变既有玩法。模式只影响提示词与宿主推进节奏，不改状态机与动作协议。
local PLAY_MODES = { "goal", "guide" }
local DEFAULT_PLAY_MODE = "guide"

local function play_mode()
  local value = effective("play.mode")
  if type(value) == "string" and contains(PLAY_MODES, value) then
    return value
  end
  return DEFAULT_PLAY_MODE
end

local function next_play_mode()
  local current = play_mode()
  for i = 1, #PLAY_MODES do
    if PLAY_MODES[i] == current then
      return PLAY_MODES[(i % #PLAY_MODES) + 1]
    end
  end
  return PLAY_MODES[1]
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
-- §地图 剧情节点图 = 六态工作流的**单源**：节点有序表 + 带动作名的边表。
-- 本 Mod 的「剧情」就是这张状态机图，故 map.show 直接由这两张表渲染 ASCII
-- 图——不额外加资源/Schema（图是代码里的结构事实，不是可编辑的故事内容）。
-- 常驻动作（config.set / map.show）不属于主图，图末一行另注。
-- 渲染只读 KV（fsm.stage / plan.*），不写任何状态（同一步重渲染不漂移）。
-- ============================================================================
local MAP_NODES = {
  STAGE.intake,
  STAGE.decompose,
  STAGE.implement,
  STAGE.verify,
  STAGE.review,
  STAGE.done,
}

-- note = beats 里的注记键名（缺省 = 无注记）；边序即渲染序（先主轴后分支）。
local MAP_EDGES = {
  { from = STAGE.intake, to = STAGE.decompose, action = "task.intake" },
  { from = STAGE.decompose, to = STAGE.implement, action = "plan.submit" },
  {
    from = STAGE.implement,
    to = STAGE.implement,
    action = "slice.report",
    note = "map_note_partial",
  },
  {
    from = STAGE.implement,
    to = STAGE.verify,
    action = "slice.report",
    note = "map_note_all",
  },
  { from = STAGE.verify, to = STAGE.review, action = "verify.submit=true" },
  {
    from = STAGE.verify,
    to = STAGE.implement,
    action = "verify.submit=false",
    note = "map_note_back",
  },
  { from = STAGE.review, to = STAGE.done, action = "review.note", note = "map_note_ending" },
}

-- 标签列宽 = `[最长节点名]`，供多出边的续行做左对齐缩进。
local MAP_LABEL_WIDTH = 0
for i = 1, #MAP_NODES do
  local width = #MAP_NODES[i] + 2
  if width > MAP_LABEL_WIDTH then
    MAP_LABEL_WIDTH = width
  end
end

-- 沙箱不暴露 string 库的静态面（只经字符串元表可达）——自备空格填充。
local function spaces(count)
  local out = ""
  for _ = 1, count do
    out = out .. " "
  end
  return out
end

local function pad_label(label)
  return label .. spaces(MAP_LABEL_WIDTH - #label)
end

local function out_edges(node)
  local out = {}
  for i = 1, #MAP_EDGES do
    if MAP_EDGES[i].from == node then
      out[#out + 1] = MAP_EDGES[i]
    end
  end
  return out
end

-- 全图渲染：表头（当前节点 + 若有计划则附进度）→ 逐节点出边（当前节点行
-- 加 `▶` 标记，多出边用 ├─/└─ 分行）→ 无出边节点收口行 → 常驻动作注记。
local function narrate_map(current)
  local progress = ""
  if plan_count() > 0 then
    progress = fill(TEXTS.beats.map_progress, {
      done = tostring(plan_done()),
      count = tostring(plan_count()),
    })
  end
  ygg.narrate(fill(TEXTS.beats.map_head, { stage = current, progress = progress }))
  local blank = spaces(MAP_LABEL_WIDTH)
  for i = 1, #MAP_NODES do
    local node = MAP_NODES[i]
    local mark = spaces(2)
    if node == current then
      mark = TEXTS.beats.map_mark
    end
    local label = pad_label("[" .. node .. "]")
    local edges = out_edges(node)
    if #edges == 0 then
      ygg.narrate(fill(TEXTS.beats.map_row_leaf, {
        mark = mark,
        node = label,
        branch = TEXTS.beats.map_branch_single,
        text = TEXTS.beats.map_terminal,
      }))
    end
    for j = 1, #edges do
      local edge = edges[j]
      local branch = TEXTS.beats.map_branch_single
      if #edges > 1 then
        if j == #edges then
          branch = TEXTS.beats.map_branch_last
        else
          branch = TEXTS.beats.map_branch_first
        end
      end
      local note = ""
      if edge.note ~= nil then
        note = TEXTS.beats[edge.note]
      end
      local lead = spaces(2)
      if j == 1 then
        lead = mark
      end
      local head = blank
      if j == 1 then
        head = label
      end
      ygg.narrate(fill(TEXTS.beats.map_row, {
        mark = lead,
        node = head,
        branch = branch,
        action = edge.action,
        target = "[" .. edge.to .. "]",
        note = note,
      }))
    end
  end
  ygg.narrate(TEXTS.beats.map_foot)
end

-- 菜单入口：与 config.set 同为常驻动作，但只读（载荷 "full" = 全图）。
local function map_item(st)
  return {
    text = fill(TEXTS.options.map, { stage = st }),
    action = "map.show",
    payload = "full",
  }
end

-- ============================================================================
-- §菜单 选项文本 ↔ 动作载荷的**单一来源**：同一张 MENU 表既产出 pendingChoice
-- 的选项文案，又决定选中序号对应的动作与默认载荷（杜绝文案/动作漂移）。
-- 规则：每态第一条 = 主前进动作（全 1 贪心走查可通关）；map.show 紧随主前进
-- 动作（verify 态排在其失败分支之后），config.set 常驻且置末。
-- 顺序即契约：只读动作排在 config.set 之前，改配置永远是最后一档。
-- ============================================================================
local MENU = {}

-- play.mode 的常驻开关：放**第一个** config.set（最全局的一档），位置在 map.show
-- 之后——只读动作仍排在改配置之前（菜单顺序契约不变）。
local function config_play_item()
  local next_value = next_play_mode()
  return {
    text = fill(TEXTS.options.config_play, { mode = play_mode(), next = next_value }),
    action = "config.set",
    payload = "play.mode=" .. next_value,
  }
end

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
    map_item(STAGE.intake),
    config_play_item(),
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
    map_item(STAGE.decompose),
    config_play_item(),
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
    map_item(STAGE.implement),
    config_play_item(),
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
    map_item(STAGE.verify),
    config_play_item(),
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
    map_item(STAGE.review),
    config_play_item(),
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

-- 每阶段的推进提示词按 play.mode 分叉：guide 版强调「确认后由你选择/提交」，goal
-- 版强调「挂机，宿主按推荐主动作自动推进，无需确认」。goal 键缺失时回落 guide 版
-- ——数据漂移不静默丢提示。
local function stage_prompt(st)
  if play_mode() == "goal" then
    return TEXTS.beats[st .. "_goal"] or TEXTS.beats[st .. "_guide"]
  end
  return TEXTS.beats[st .. "_guide"]
end

-- 提示词占位符的**超集**：guide / goal 两个变体共用一份（fill 只替换模板里出现的
-- 键，多余键无副作用）。全部值只读 KV / 资源现算，渲染路径仍无副作用。
local function prompt_vars()
  return {
    engine = tostring(ygg.engine_version),
    stage = stage(),
    play = play_mode(),
    mode = mode(),
    gates = gates_word(),
    min = tostring(TEXTS.tuning.min_slices),
    max = tostring(TEXTS.tuning.max_slices),
    slice_id = slice_id(next_index()),
    score = tostring(TEXTS.tuning.default_score),
  }
end

-- 每阶段回执顶部固定一行：当前模式 + 该模式的推进纪律（含拒绝回执与局末——
-- 「现在是什么模式、这一步要不要人确认」永远在回执里可见）。
local function narrate_play_mode()
  local hint = TEXTS.beats.play_hint_guide
  if play_mode() == "goal" then
    hint = TEXTS.beats.play_hint_goal
  end
  ygg.narrate(fill(TEXTS.beats.play_receipt, { play = play_mode(), hint = hint }))
end

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
  ygg.narrate(fill(stage_prompt(STAGE.intake), prompt_vars()))
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
  ygg.narrate(fill(stage_prompt(STAGE.decompose), prompt_vars()))
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
  ygg.narrate(fill(stage_prompt(STAGE.implement), prompt_vars()))
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
  ygg.narrate(fill(stage_prompt(STAGE.verify), prompt_vars()))
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
  ygg.narrate(fill(stage_prompt(STAGE.review), prompt_vars()))
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
  if play_mode() == "goal" then
    -- 挂机完成说明：goal 下本局全程无人工确认，结算报告由宿主 `--settle` 生成。
    ygg.narrate(TEXTS.beats.play_done_goal)
  end
end

local function render()
  local current = stage()
  -- 模式行先行：goal/guide 的推进纪律在每个回执里都可见（含拒绝与局末）。
  narrate_play_mode()
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
  if key == "play.mode" then
    return TEXTS.beats.scope_play_mode
  elseif key == "evolution.mode" then
    return TEXTS.beats.scope_evolution_mode
  elseif key == "evolution.max_per_run" then
    return TEXTS.beats.scope_evolution_cap
  elseif key == "gates.require_all_green" then
    return TEXTS.beats.scope_gates
  end
  return TEXTS.beats.scope_host
end

local function apply_config(key, value)
  if key == "play.mode" then
    -- 只认 goal / guide：非法值走可读拒绝，KV 不写（状态与模式都不变）。
    if not contains(PLAY_MODES, value) then
      return fill(TEXTS.beats.bad_config_value, {
        key = key,
        value = value,
        allowed = join(PLAY_MODES, " / "),
      })
    end
    ygg.kv.set("settings.play.mode", value)
    return nil
  elseif key == "evolution.mode" then
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
-- §地图动作 map.show（常驻只读动作，任意主状态可用）：本 Mod 的「剧情」= 六态
-- 工作流，故地图 = 上面 §地图 的节点/边表渲染出的 ASCII 图。
-- 载荷只接空 / 0 / "full"（三者为同一张全图，不做局部视图）；其余载荷回执
-- 用法后原样重渲染——与 config.set 一样**不吞主前进动作**。
-- ============================================================================
local function map_payload_ok(payload)
  if payload == nil then
    return true
  end
  if type(payload) == "number" then
    return payload == 0 -- 内核 --payload 缺省值就是 0
  end
  if type(payload) == "string" then
    local raw = trim(payload)
    return raw == "" or raw == "0" or raw == "full"
  end
  return false
end

ygg.on("map.show", function(payload)
  if map_payload_ok(payload) then
    narrate_map(stage())
  else
    ygg.narrate(fill(TEXTS.beats.map_rejected, {
      got = tostring(payload),
      usage = TEXTS.beats.map_usage,
    }))
  end
  render()
end)

-- ============================================================================
-- §选择路由 pendingChoice 的回应面：序号 → MENU 的第七项（动作 + 默认载荷）→
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
  ygg.narrate(fill(stage_prompt("prologue"), prompt_vars()))
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
