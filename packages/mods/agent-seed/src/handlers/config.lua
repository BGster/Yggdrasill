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

local MODES, PLAY_MODES, effective =
  config.MODES, config.PLAY_MODES, config.effective
local reject, reject_payload =
  flow.reject, flow.reject_payload
local CONFIG_UI =
  menus.CONFIG_UI
local render =
  render_mod.receipt
local stage =
  reqs.stage
local contains, cut, fill, join, to_bool, trim =
  util.contains, util.cut, util.fill, util.join, util.to_bool, util.trim
local probe_payload_ok =
  wire.probe_payload_ok



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
