-- `pkg.dialogue.dispatch` —— `emit` 的分发面：回退链、深度帽、失败事件。
--
-- # 两条执行路（I1 通道封闭）
--
-- handler 的执行只有两条：`builtin:<name>`（mod 注册的 Lua 函数，走 `REGISTRY`）与
-- `agent`（经 `ygg.tool.call` 的既有工具桥）。包内不出现第二个出口：网络、子进程、
-- 文件面调用一概没有——d12 用静态命中数（会返回反例的形态）钉这一条。
-- 命中面清单写在 `dialogue.md` §7（文档里带字面量，源码里不带，免得扫描命中自己）。
--
-- # 深度语义：只数**同步嵌套**
--
-- `CHAIN.depth` 在一次同步调用链里递增：`dialogue.emit` 同步调 handler，handler
-- 再调 `dialogue.emit` 才是嵌套。`ygg.emit` 入队的派生事件由 driver 排空
-- （`crates/ygg-core/src/transition.rs` 的 FIFO 排空），**不共享**本链，深度重新
-- 从 1 起算。宿主也**没有** `max_cascade_depth`（`grep -rni cascade --include=*.rs
-- crates/` 命中 0），所以本包的帽是这条链的唯一守卫（回写 D8）。
--
-- # 回执形状（I2 / d3）
--
-- `{ handled, by, depth }` 三个字段，恰好这三个：多一个字段就要有读者，
-- 而读者还没出现（`reason` 进 `dialogue.failed` 的载荷，不进回执）。

local S = require("pkg.dialogue.store")
local H = require("pkg.dialogue.handlers")
local memory = require("pkg.dialogue.memory")

local D = {}

--- 模块局部调用链栈（`install` 不重置它：它不是状态，是当前调用深度）。
local CHAIN = { depth = 0 }

--- `frame(type, source, target, payload?)`：消息帧（纯函数，不读不写 KV）。
function D.frame(type_name, source, target, payload)
  return {
    v = 1,
    type = type_name,
    source = source,
    target = target,
    payload = payload or {},
  }
end

--- `agent` handler 的工具实参：**扁平标量**（宿主 `tool_args_from_lua` 只收标量，
--- 嵌套表直接拒）。所以帧不整体下传——`type/source/target/text_key/credential_key`
--- 五个键就是 agent 侧能看到的全部（回写 D11）。
local function agent_args(spec, frame)
  local credential = spec.opts.credential_key
  local text_key = frame.payload.text_key
  return {
    type = frame.type,
    source = tostring(frame.source),
    target = tostring(frame.target),
    text_key = (type(text_key) == "string") and text_key or "",
    credential_key = (type(credential) == "string") and credential or "",
  }
end

--- 跑一个 handler → `{ handled, reason }`。失败一律回退，不静默吞（I2）。
local function run_spec(h, spec, frame)
  if spec.kind == "agent" then
    local tool = spec.opts.tool or "dialogue.agent"
    local ok, res = pcall(h.tool.call, tool, agent_args(spec, frame))
    if not ok then
      return { handled = false, reason = tostring(res) }
    end
    if type(res) ~= "table" then
      return { handled = false, reason = "tool-result-not-a-table" }
    end
    if res.unmounted == true then
      return { handled = false, reason = "unmounted" }
    end
    if res.ok == false then
      return { handled = false, reason = tostring(res.error or "tool-failed") }
    end
    return { handled = true }
  end

  local fn = H.fn_of(frame.type, spec.name)
  if fn == nil then
    -- 有 spec 无函数 = 未绑定（重载后忘记注册的显式形态，D3）。
    return { handled = false, reason = "unbound" }
  end
  local ok, result = pcall(fn, frame)
  if not ok then
    return { handled = false, reason = tostring(result) }
  end
  if result == true or (type(result) == "table" and result.handled == true) then
    return { handled = true }
  end
  return { handled = false, reason = "declined" }
end

--- 按注册序走一遍 handler 列表；第一个 handled 的终止链（I7）。
local function walk(h, type_name, frame)
  local specs = H.list(h, type_name)
  local reason = "none"
  for i = 1, #specs do
    local outcome = run_spec(h, specs[i], frame)
    if outcome.handled then
      memory.remember(h, specs[i], frame)
      return { handled = true, by = specs[i].name }
    end
    if outcome.reason ~= nil then
      reason = outcome.reason
    end
  end
  h.emit("dialogue.failed", type_name .. "|" .. reason)
  return { handled = false, by = nil }
end

--- `emit(type, source, target, payload?)` → `{ handled, by, depth }`。
function D.emit(h, type_name, source, target, payload)
  if not S.type_name_ok(type_name) then
    S.fail(S.E.TYPE, "type 必须匹配 ^[a-z][a-z0-9_-]*$，得到 " .. tostring(type_name))
  end
  if not S.ref_ok(source) then
    S.fail(S.E.TYPE, "source 必须是实体 ref（正整数，0 = 系统），得到 " .. tostring(source))
  end
  if not S.ref_ok(target) then
    S.fail(S.E.TYPE, "target 必须是实体 ref（正整数，0 = 系统），得到 " .. tostring(target))
  end
  if payload ~= nil and type(payload) ~= "table" then
    S.fail(S.E.TYPE, "payload 只收 table 或 nil，得到 " .. type(payload))
  end

  local depth = CHAIN.depth + 1
  if depth > S.CONFIG.max_depth then
    h.emit("dialogue.depth_capped", type_name .. "|" .. tostring(depth))
    return { handled = false, by = nil, depth = depth }
  end

  local frame = D.frame(type_name, source, target, payload)
  CHAIN.depth = depth
  local ok, result = pcall(walk, h, type_name, frame)
  -- pcall 保护：handler 抛错也要把深度还回去，否则一次异常就把后续链永久压到帽上。
  CHAIN.depth = depth - 1
  if not ok then
    error(result, 0)
  end
  result.depth = depth
  return result
end

return D
