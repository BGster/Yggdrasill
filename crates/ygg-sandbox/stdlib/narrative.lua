-- ygg.narrative：声明式叙事流路由（S10 stdlib）。
--
-- 语义（锁定设计）：
-- - 叙事节点为**装载期声明**的纯数据面：`node(id, body)` 把函数体登记进
--   私有 upvalue 表 `nodes`（模块私有，Mod 面无法绕过路由直接触达他人
--   节点表）；body 必须是函数、id 必须是非空字符串，重复定义拒绝；
-- - `goto` 以派生事件承载：调用方 `ygg.narrative["goto"](id)` 发出
--   `ygg.emit("ygg.narrative.goto", id)`，经 per-transition FIFO 回到路由
--   handler 执行对应 body——叙事跳转 = 事件，天然吃燃料计量与 FIFO 级联序
--   （确定性纪律：无任何隐藏控制流旁路）；
-- - 路由 handler 在 stdlib 装载期经 ygg.on 注册（先于 Mod chunk，注册序
--   即派发序：stdlib 路由先于 Mod 自有的同名事件 handler）；
--   未知节点 = 可读报错（不静默）。
--
-- Lua 5.4 保留字注记：`goto` 是保留字，点号访问 `ygg.narrative.goto` 是
-- 语法错误——本库以 `ygg.narrative["goto"]` 括号形态定义与调用（事件名
-- `"ygg.narrative.goto"` 不受影响，仅 Lua 侧标识符访问路径受限）。

ygg.narrative = ygg.narrative or {}

--- 私有节点表（upvalue：Mod 面不可达）。
local nodes = {}

--- 装载期声明一个叙事节点（id 非空字符串；body 必须为函数；重复定义拒绝）。
function ygg.narrative.node(id, body)
  if type(id) ~= "string" or id == "" then
    error("ygg.narrative.node: 'id' must be a non-empty string, got " .. type(id), 2)
  end
  if type(body) ~= "function" then
    error("ygg.narrative.node: node '" .. id .. "' body must be a function, got " .. type(body), 2)
  end
  if nodes[id] ~= nil then
    error("ygg.narrative.node: node '" .. id .. "' is already defined", 2)
  end
  nodes[id] = body
end

--- 叙事跳转：发出路由派生事件（在 handler 执行期内调用——派生事件经
-- per-transition FIFO 回路由 handler；装载期调用会进收集缓冲、不被回放）。
ygg.narrative["goto"] = function(id)
  if type(id) ~= "string" or id == "" then
    error("ygg.narrative['goto']: 'id' must be a non-empty string, got " .. type(id), 2)
  end
  ygg.emit("ygg.narrative.goto", id)
end

-- 路由 handler：解析 id 对应的 body 并执行；未知节点可读报错（不静默）。
ygg.on("ygg.narrative.goto", function(id)
  local body = nodes[id]
  if body == nil then
    error("ygg.narrative['goto']: unknown node '" .. tostring(id) .. "'", 2)
  end
  body()
end)
