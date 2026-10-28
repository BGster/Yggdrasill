-- ygg.fsm：确定性有限状态机（S10 stdlib）。
--
-- 语义（锁定设计）：
-- - 状态持久化在 ygg.kv 的键 `"fsm." .. name` 下（即席状态写：状态是内核
--   KV 状态的一部分，随存档快照往返，读档后状态机原样续接）；
-- - handler 经 ygg.on(spec.event) 注册（define 只能在装载期调用——seal 后
--   ygg.on 以可读报错拒绝）；
-- - 转换按声明序尝试（ipairs 严格序；禁 pairs——确定性纪律：状态哈希与
--   派发序绝不消费无序迭代）；
-- - 声明序首个「from 匹配且条件成立」的边胜出（无 when = 恒真；when 为
--   可选的 function(payload) -> boolean，truthy 即通过——B1 条件边：
--   同一 from 的多出边单机可表达）；无匹配 = 静默 no-op（不写 KV、
--   不报错——状态机对无关事件透明）；
-- - 首次事件时状态未初始化则先落 initial 再参与匹配（B2：键缺席写
--   initial，幂等——键存在则不动；未触发的机器仍不落盘）；仅在实际
--   转移时另写目标态；无匹配不报错（静默 no-op，但 initial 已落盘——
--   「状态机已被触发过」在 KV 面可见）。

ygg.fsm = ygg.fsm or {}

--- 声明一台状态机并注册其事件 handler。
-- spec = { name = string, initial = string, event = string,
--          transitions = { { from = string, to = string,
--                            when = function(payload) -> boolean? }, ... } }
function ygg.fsm.define(spec)
  if type(spec) ~= "table" then
    error("ygg.fsm.define: spec must be a table, got " .. type(spec), 2)
  end
  if type(spec.name) ~= "string" or spec.name == "" then
    error("ygg.fsm.define: field 'name' must be a non-empty string", 2)
  end
  if type(spec.initial) ~= "string" or spec.initial == "" then
    error("ygg.fsm.define: field 'initial' must be a non-empty string", 2)
  end
  if type(spec.event) ~= "string" or spec.event == "" then
    error("ygg.fsm.define: field 'event' must be a non-empty string", 2)
  end
  if type(spec.transitions) ~= "table" then
    error("ygg.fsm.define: field 'transitions' must be a table (array of {from, to})", 2)
  end
  for i, t in ipairs(spec.transitions) do
    if type(t) ~= "table" then
      error("ygg.fsm.define: transitions[" .. i .. "] must be a table", 2)
    end
    if type(t.from) ~= "string" or t.from == "" then
      error("ygg.fsm.define: transitions[" .. i .. "] must have a non-empty string 'from'", 2)
    end
    if type(t.to) ~= "string" or t.to == "" then
      error("ygg.fsm.define: transitions[" .. i .. "] must have a non-empty string 'to'", 2)
    end
    if t.when ~= nil and type(t.when) ~= "function" then
      error("ygg.fsm.define: transitions[" .. i .. "].when must be a function or nil, got "
        .. type(t.when), 2)
    end
  end

  local key = "fsm." .. spec.name
  local initial = spec.initial
  local transitions = spec.transitions

  ygg.on(spec.event, function(payload)
    local state = ygg.kv.get(key)
    if state == nil then
      -- 首次触发且键缺席：先落 initial 再参与匹配（B2 键缺席可见化；
      -- 幂等——键存在则不动，不重复写）。
      ygg.kv.set(key, initial)
      state = initial
    end
    -- 声明序尝试（ipairs 严格序；首个 from 匹配且 when 成立的边胜出；
    -- 无 when = 恒真；无匹配 = 静默 no-op）。
    for _, t in ipairs(transitions) do
      if t.from == state and (t.when == nil or t.when(payload)) then
        ygg.kv.set(key, t.to)
        return
      end
    end
  end)
end
