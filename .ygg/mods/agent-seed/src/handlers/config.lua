-- src/handlers/config.lua —— 运行期配置写入与配置二级页开合：config.set|open|close。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local config = require("src.config")
local flow = require("src.flow")
local menus = require("src.menus")
local render_mod = require("src.render")
local reqs = require("src.requirements")
local util = require("src.util")
local wire = require("src.wire")

local TEXTS

local MODES, PLAY_MODES, effective, executor_raw =
  config.MODES, config.PLAY_MODES, config.effective, config.executor_raw
local refuse, reject, reject_payload =
  flow.refuse, flow.reject, flow.reject_payload
local CONFIG_UI =
  menus.CONFIG_UI
local render =
  render_mod.receipt
local req_index_of, req_id, req_push_history, stage =
  reqs.req_index_of, reqs.req_id, reqs.req_push_history, reqs.stage
local contains, cut, fill, join, to_bool, trim =
  util.contains, util.cut, util.fill, util.join, util.to_bool, util.trim
local probe_payload_ok =
  wire.probe_payload_ok

-- 执行者账本的单点写入（`executor.<rN>.<stage>`）。
-- 自声明；只校形状与「换人必给原因」，不验证 id 真伪。
local function executor_set(key, value)
  local req_text, stage_name = wire.parse_executor_key(key)
  if req_text == nil or not wire.executor_stage_ok(stage_name) then
    return reject_payload("config.set", fill(TEXTS.beats.executor_bad_key, {
      key = tostring(key),
      stages = join(wire.EXECUTOR_STAGES, " / "),
    }))
  end
  local index = req_index_of(req_text)
  if index == nil then
    return reject_payload("config.set", fill(TEXTS.beats.executor_unknown_req, { id = req_text }))
  end
  local agent, reason, id_problem = wire.parse_executor_value(value)
  if id_problem ~= nil then
    return reject_payload("config.set", id_problem)
  end
  local slot = "settings.executor." .. req_text .. "." .. stage_name
  local current = wire.executor_id_of(executor_raw(req_text, stage_name))
  if agent == "" then
    ygg.kv.set(slot, "")
    ygg.narrate(fill(TEXTS.beats.executor_cleared, { id = req_id(index), stage = stage_name }))
    return render()
  end
  if current == agent then
    ygg.narrate(fill(TEXTS.beats.executor_same, {
      id = req_id(index),
      stage = stage_name,
      agent = agent,
    }))
    return render()
  end
  -- 跨阶段：plan 看 analysis；development 看 plan（plan 缺失时回看 analysis）。
  local prev_stage = nil
  if stage_name == "plan" then
    prev_stage = "analysis"
  elseif stage_name == "development" then
    prev_stage = "plan"
  end
  local prev = ""
  if prev_stage ~= nil then
    prev = wire.executor_id_of(executor_raw(req_text, prev_stage))
    if prev == "" and stage_name == "development" then
      prev = wire.executor_id_of(executor_raw(req_text, "analysis"))
    end
  end
  local handoff_from = current ~= "" and current or prev
  local needs_reason = handoff_from ~= "" and handoff_from ~= agent
  if needs_reason and reason == "" then
    return refuse(TEXTS.beats.req_wait_rejected, "config.set", fill(TEXTS.beats.executor_reason_required, {
      id = req_id(index),
      stage = stage_name,
      prev = handoff_from,
      agent = agent,
    }))
  end
  ygg.kv.set(slot, wire.format_executor(agent, reason))
  -- 只有「真的换了人」才留痕；跨阶段同 id 是正常连续（首次绑定该 stage），不写 hist。
  if handoff_from ~= "" and handoff_from ~= agent then
    req_push_history(index, fill(TEXTS.beats.executor_hist, {
      stage = stage_name,
      from = handoff_from,
      to = agent,
      reason = reason ~= "" and reason or TEXTS.beats.executor_no_reason,
    }))
  end
  ygg.narrate(fill(TEXTS.beats.executor_bound, {
    id = req_id(index),
    stage = stage_name,
    agent = agent,
  }))
  return render()
end



function M.install(deps)
  TEXTS = deps.texts
  ygg.on("config.set", function(payload)
    local current = stage()
    if type(payload) ~= "string" then
      return reject("config.set", fill(TEXTS.beats.bad_payload, { action = "config.set" }))
    end
    local key, value = cut(trim(payload), "=")
    key = trim(key)
    value = trim(value or "")
    if key:sub(1, 9) == "settings." then
      key = key:sub(10)
    end
    local problem = nil
    if key == "play.mode" then
      -- 切模式是「决策权移交」，不是业务判断：主 Agent 在用户当次授权下可自行切
      -- （人只有对话、没有菜单时，锁住就等于永久锁死 goal）。代价是必须当场留痕。
      if not contains(PLAY_MODES, value) then
        problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = join(PLAY_MODES, " / ") })
      else
        ygg.kv.set("settings.play.mode", value)
        if value == "goal" then
          ygg.narrate(TEXTS.beats.handover_goal)
        else
          ygg.narrate(TEXTS.beats.handover_guide)
        end
        -- 驱动者换了人，二级页的「当前处境」已失效：退出子页回阶段菜单。
        CONFIG_UI.set_page("")
      end
    elseif key == "goal.authorization" then
      -- 自声明：只校形状（单行、trim 后 ≤200），空值 = 显式清除。
      local auth, auth_problem = wire.parse_goal_auth(value)
      if auth_problem ~= nil then
        return reject_payload("config.set", auth_problem)
      end
      ygg.kv.set("settings.goal.authorization", auth)
      if auth == "" then
        ygg.narrate(TEXTS.beats.goal_auth_cleared)
      else
        ygg.narrate(fill(TEXTS.beats.goal_auth_registered, { auth = auth }))
      end
      render()
      return
    elseif key:sub(1, 9) == "executor." then
      return executor_set(key, value)
    elseif key == "evolution.mode" then
      if not contains(MODES, value) then
        problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = join(MODES, " / ") })
      else
        ygg.kv.set("settings.evolution.mode", value)
      end
    elseif key == "evolution.max_per_run" then
      local parsed = tonumber(value)
      if parsed == nil or parsed < 0 or parsed > 8 then
        problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = "0–8 的整数" })
      else
        ygg.kv.set("settings.evolution.max_per_run", math.floor(parsed))
      end
    elseif key == "review.evolution.artifact" then
      -- 登记面：只校验形状（路径），不读文件、不判断内容。
      local artifact = trim(value)
      if artifact ~= "" and not wire.evolution_path_ok(artifact) then
        problem = TEXTS.beats.review_evolution_bad_path
      else
        ygg.kv.set("settings.review.evolution.artifact", artifact)
      end
    elseif key == "review.evolution.candidates" then
      local candidates = trim(value)
      if candidates == "" then
        ygg.kv.set("settings.review.evolution.candidates", "")
      else
        local kept, screened = wire.parse_candidates(candidates)
        if kept == nil then
          problem = TEXTS.beats.review_evolution_bad_candidates
        else
          ygg.kv.set("settings.review.evolution.candidates", tostring(kept) .. "/" .. tostring(screened))
        end
      end
    elseif key == "review.evolution.classes" then
      local classes = trim(value)
      if classes == "" then
        ygg.kv.set("settings.review.evolution.classes", "")
      else
        local parsed, total = wire.parse_classes(classes)
        if parsed == nil or total == 0 then
          problem = TEXTS.beats.review_evolution_bad_classes
        else
          ygg.kv.set("settings.review.evolution.classes", wire.format_classes(parsed))
        end
      end
    elseif key == "gates.require_all_green" or key == "autosave" or key == "hot_reload" then
      local parsed = to_bool(value)
      if parsed == nil then
        problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = "true / false" })
      else
        ygg.kv.set("settings." .. key, parsed)
      end
    else
      problem = fill(TEXTS.beats.bad_config_key, { key = key, allowed = TEXTS.beats.config_keys })
    end
    if problem ~= nil then
      return reject_payload("config.set", problem)
    end
    ygg.narrate(fill(TEXTS.beats.config_changed, {
      key = key,
      value = tostring(effective(key)),
      stage = current,
    }))
    render()
  end)
  ygg.on("config.open", function(payload)
    if not probe_payload_ok(payload) then
      return reject_payload("config.open", fill(TEXTS.beats.probe_rejected, {
        action = "config.open",
        got = tostring(payload),
        usage = TEXTS.beats.config_open_usage,
      }))
    end
    CONFIG_UI.set_page("config", stage())
    ygg.narrate(TEXTS.beats.config_opened)
    render()
  end)
  ygg.on("config.close", function(payload)
    if not probe_payload_ok(payload) then
      return reject_payload("config.close", fill(TEXTS.beats.probe_rejected, {
        action = "config.close",
        got = tostring(payload),
        usage = TEXTS.beats.config_open_usage,
      }))
    end
    CONFIG_UI.set_page("")
    ygg.narrate(fill(TEXTS.beats.config_closed, { stage = stage() }))
    render()
  end)
end


return M
