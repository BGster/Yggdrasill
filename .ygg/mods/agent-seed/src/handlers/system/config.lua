-- src/handlers/system/config.lua —— 运行期配置写入与配置二级页开合：config.set|open|close。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。

local M = {}

local clock = require("src.core.clock")
local config = require("src.core.config")
local executor = require("src.domain.executor")
local flow = require("src.runtime.flow")
local menus = require("src.view.menus")
local readset = require("src.domain.readset")
local readsig = require("src.domain.readsig")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local wire = require("src.domain.wire")

local TEXTS

local MODES, PLAY_MODES, effective, goal_authorization, text_of =
  config.MODES, config.PLAY_MODES, config.effective, config.goal_authorization, config.text_of
local refuse, reject, reject_payload =
  flow.refuse, flow.reject, flow.reject_payload
local CONFIG_UI =
  menus.CONFIG_UI
local render =
  render_mod.receipt
local stage, run_phase =
  reqs.stage, reqs.run_phase
local contains, cut, fill, join, split, to_bool, trim =
  util.contains, util.cut, util.fill, util.join, util.split, util.to_bool, util.trim
local probe_payload_ok =
  wire.probe_payload_ok

-- 执行者账本的实现已抽到 `src/domain/executor.lua`：与 `proposal.submit` 的末段 `exec=` 共用同一份
-- 校验与叙事（省回合：登记并入提案提交）。这里只把失败描述映射回 `config.set` 的拒绝口径。
local function executor_set(key, value)
  local problem = executor.bind(key, value)
  if problem ~= nil then
    if problem.kind == "blocked" then
      return refuse(TEXTS.beats.req_wait_rejected, "config.set", problem.detail)
    end
    return reject_payload("config.set", problem.detail)
  end
  return render()
end

-- profile 注册表索引（r3）：沙箱无键枚举，用平铺索引键维护句柄清单（只增不删，
-- registry 只增改、收口随 run 终结）。索引键不在 config.set 用户面（不在分支链 →
-- 直写会被 bad_config_key 拒），只能由本函数维护。投影按 handle 升序（render 排）。
local function profile_index_add(handle)
  local raw = text_of("settings.profile_index", "")
  for _, part in ipairs(split(raw, ",")) do
    if trim(part) == handle then
      return
    end
  end
  ygg.kv.set("settings.profile_index", raw == "" and handle or (raw .. "," .. handle))
end

-- profile 注册表（r3，自报身份卡）：`profile.<handle>.<key>` 逐键直写。注册走
-- config.set（不新增事件）：天然落事件行可审计、同键重写幂等 upsert、不占人类动作门禁。
-- `handle` 键是冗余自报：一致即通过但**不落 KV**（键即 handle，免冗余存储）；
-- 整卡删除不提供（空值逐字段清除，收口随 run 终结）。
local function profile_set(key, value)
  local handle, field = wire.parse_profile_key(key)
  if handle == nil then
    return reject_payload("config.set", TEXTS.beats.profile_bad_handle)
  end
  local clean, problem = wire.validate_profile_value(handle, field, value)
  if problem ~= nil then
    return reject_payload("config.set", problem)
  end
  if field == "handle" then
    ygg.narrate(fill(TEXTS.beats.profile_registered, { handle = handle, field = field }))
    render()
    return
  end
  ygg.kv.set("settings.profile." .. handle .. "." .. field, clean)
  profile_index_add(handle)
  if clean == "" then
    ygg.narrate(fill(TEXTS.beats.profile_cleared, { handle = handle, field = field }))
  else
    ygg.narrate(fill(TEXTS.beats.profile_registered, { handle = handle, field = field }))
  end
  render()
end

-- 「幕前必读」签名（r3 销票制）：`read.<act>/<doc-id>=<ver>`。
-- 值由 readsig 组装成 `ver|<签名幕>|<seq>`（seq 预留，此刻恒 0，自增由 r4 落地）；
-- 作用域边界由 readsig.clear() 主动销票（run.begin / coordinate.submit / 进发布幕 /
-- 回协调幕 / rev 变更 / proposal.reject），所以「键非空」= 本作用域已签，
-- 消费者（r4）不需要时钟、不需要比较序号。
-- 覆盖面 k/n 只在 r2 的 readPlan 已烘进 data/config.json 时才算；**缺表即降级**
-- （回执只给件数），绝不因此拒签：签名是自声明，不是门禁。
local function read_set(key, value)
  local doc = wire.parse_read_key(key)
  if doc == nil then
    return reject_payload("config.set", fill(TEXTS.beats.read_bad_doc, { doc = key }))
  end
  local scene = run_phase()
  local ver = trim(value)
  if ver == "" then
    -- 空值 = 显式清除该件（与 run.retire / goal.authorization 同构）。
    readsig.forget(doc)
    ygg.narrate(fill(TEXTS.beats.read_cleared, { doc = doc, scene = scene }))
    render()
    return
  end
  if not wire.read_ver_ok(ver) then
    return reject_payload("config.set", fill(TEXTS.beats.read_bad_ver, { ver = ver }))
  end
  readsig.sign(doc, ver, scene)
  -- r4：签名时刻快照（与 readsig 值段的 seq 互为印证；值段 seq 为 0 时消费者回落读它）。
  clock.mark_sign(doc)
  local kept, total = readsig.coverage(scene, effective("readPlan"))
  if total == nil then
    ygg.narrate(fill(TEXTS.beats.read_signed_unplotted, {
      doc = doc,
      ver = ver,
      k = tostring(kept),
      scene = scene,
    }))
  else
    ygg.narrate(fill(TEXTS.beats.read_signed, {
      doc = doc,
      ver = ver,
      k = tostring(kept),
      n = tostring(total),
      scene = scene,
    }))
  end
  render()
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
          if goal_authorization() == "" then
            ygg.narrate(TEXTS.beats.goal_auth_pending)
          end
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
    elseif key == "run.retire" then
      -- r9 退役武装（与 goal.authorization 同构：run 级自声明，空值 = 显式解除）。
      -- 武装本身不过票据闸也不产生通过效果——终局由武装后的 review.note 人类票据完成。
      if value ~= "true" and value ~= "" then
        problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = 'true / "" (empty clears)' })
      else
        ygg.kv.set("settings.run.retire", value)
        if value == "true" then
          ygg.narrate(TEXTS.beats.run_retire_armed)
        else
          ygg.narrate(TEXTS.beats.run_retire_cleared)
        end
      end
    elseif key:sub(1, 9) == "executor." then
      return executor_set(key, value)
    elseif key:sub(1, 8) == "profile." then
      return profile_set(key, value)
    elseif key:sub(1, 5) == "read." then
      return read_set(key, value)
    elseif key == "readGate.window" then
      -- 遗忘窗口（r4 四维之一）：整数，缺省 80，越界夹紧到 [0, MAX]。
      -- 调窗是**自解除的第二个杆杆**（第一杆是重读重签）：四维拒台词里逐条报当前值与抬高命令。
      -- 回执走通用 config_changed（`effective("readGate.window")` 此刻已是夹紧后的值）——不新开文案键。
      local parsed = tonumber(value)
      if parsed == nil then
        problem = fill(TEXTS.beats.bad_config_value, {
          key = key,
          value = value,
          allowed = "integer 0-" .. tostring(readset.MAX_WINDOW),
        })
      else
        parsed = math.floor(parsed)
        if parsed < 0 then
          parsed = 0
        end
        if parsed > readset.MAX_WINDOW then
          parsed = readset.MAX_WINDOW
        end
        ygg.kv.set("settings.readGate.window", parsed)
      end
    elseif key == "tools.root" then
      -- harness tools 注册根（r3 / D-MA26）：目录路径，形状校验（无 `..`/`|`/控制字符，≤240）。
      -- 只校形状不读文件（Mod 无 IO）；空值 = 显式清除（机证回退人证）。
      if not wire.tools_root_ok(value) then
        return reject_payload("config.set", TEXTS.beats.tools_root_bad)
      end
      ygg.kv.set("settings.tools.root", trim(value))
      if trim(value) == "" then
        ygg.narrate(TEXTS.beats.tools_root_cleared)
      else
        ygg.narrate(fill(TEXTS.beats.tools_root_registered, { root = trim(value) }))
      end
      render()
      return
    elseif key:sub(1, 6) == "tools." then
      -- tools.<name> 分两类（r3 / D-MA26）：
      --   保留参数键 tools.session / tools.sessions_root = 自由值（会话 id / 会话库路径），
      --     供结算 cross-check 拼 argv（mod 沙箱读不到进程环境，参数只能来自 KV）。
      --   脚本覆盖键 tools.<name>=<relpath> = 覆盖缺省 `<name>.sh`（`.sh` 结尾）。
      local name = wire.parse_tools_key(key)
      if name == nil then
        return reject_payload("config.set", fill(TEXTS.beats.bad_config_key, { key = key, allowed = TEXTS.beats.config_keys }))
      end
      if name == "session" or name == "sessions_root" then
        -- 自由值：禁 `|`/控制字符，≤240；空 = 清除。session 是标识符，sessions_root 是路径。
        if not wire.tools_root_ok(value) then
          return reject_payload("config.set", TEXTS.beats.tools_param_bad)
        end
        ygg.kv.set("settings.tools." .. name, trim(value))
        ygg.narrate(fill(TEXTS.beats.tools_param_registered, { name = name, value = trim(value) }))
        render()
        return
      end
      if not wire.tools_relpath_ok(value) then
        return reject_payload("config.set", TEXTS.beats.tools_relpath_bad)
      end
      ygg.kv.set("settings.tools." .. name, trim(value))
      ygg.narrate(fill(TEXTS.beats.tools_name_registered, { name = name, path = trim(value) }))
      render()
      return
    elseif key == "evolution.mode" then
      if not contains(MODES, value) then
        problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = join(MODES, " / ") })
      else
        ygg.kv.set("settings.evolution.mode", value)
      end
    elseif key == "evolution.max_per_run" then
      local parsed = tonumber(value)
      if parsed == nil or parsed < 0 or parsed > 8 then
        problem = fill(TEXTS.beats.bad_config_value, { key = key, value = value, allowed = "integer 0-8" })
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
    elseif key == "review.evolution.summary" then
      -- 预登记 summary：去空白；空值 = 显式清除，非空原样写入。
      ygg.kv.set("settings.review.evolution.summary", trim(value))
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
    -- 子页键用微状态口径（与 CONFIG_UI.open 的比对侧一致，T4 缺陷 4：真状态 "live"
    -- 与微状态比对永不相等 → 子页打不开、run.retire 武装路径不可用）。
    CONFIG_UI.set_page("config", run_phase())
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
