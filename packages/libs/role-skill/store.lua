-- `packages/libs/role-skill/store.lua` —— 四个 KV 键的唯一读写口 + 依赖解析。
--
-- 契约：`docs/design/packages/role-skill.md` §2（数据形状）/ I1（薄层）。
--
-- # KV 只收标量
--
-- 宿主 `ygg.kv` 只收四个标量（`crates/ygg-core/src/entity.rs` 的 `Value`）；
-- `host_api::value_from_lua` 对其它类型直接报 `unsupported payload type`。所以四个
-- 键的载荷一律是 `codec.encode` 的**字符串**，读回时 `codec.decode`。
--
-- # 索引是升序稠密数组，不是集合
--
-- `{ [id] = true }` 存成 JSON 会变成对象键，读回来所有整数键都成了字符串
-- （`"1" ~= 1`），成员判定随即失效。数组保留整数，插入时保持升序 ⇒ 字节稳定。
--
-- # 记录不可删
--
-- KV 没有 delete、也没有遍历面（`docs/design/kernel/state-surface.md` §2.8.1 是同一
-- 事实）。所以 `revoke` / `revoke_skill` 只清**引用面**，既不删记录，也不从
-- `role.index` 摘除——这不是省事，是唯一能做的事；写成「删除」的那版代码会在第一次
-- 调用时就找不到 API。
--
-- # 全部 KV 流量收口在这里
--
-- 其它模块只能经 `read` / `write` 摸 KV。换编码口径只改这一处；「一次 `effective`
-- 恰好几次 `kv.get`」也才能被用例按同一个入口数出来（性能面）。
--
-- # 不缓存
--
-- 跨 dispatch 的 Lua 侧缓存是回放隐患（`state-surface.md` §2.8.0）：同一份存档在
-- 两个进程里带不同的缓存就会产出不同结果。首实现一律直读，实测达不到预算再加。

local M = {}

--- 错误码面（包内唯一一份；`pkg.role_skill.ERRORS` 从这里复制）。
M.E = {
  DEF = "YGG-PKG-ROLE-DEF",
  UNKNOWN = "YGG-PKG-ROLE-UNKNOWN",
  COMPOSE = "YGG-PKG-ROLE-COMPOSE",
  DEPS = "YGG-PKG-ROLE-DEPS",
  KV = "YGG-PKG-ROLE-KV",
  INSTALL = "YGG-PKG-ROLE-INSTALL",
  ATTR = "YGG-PKG-ROLE-ATTR",
}

--- 报错：`<码>: <细节>`（level 0 = 不夹带调用点，用例按码做子串断言）。
function M.fail(code, detail)
  error(code .. ": " .. detail, 0)
end

--- KV 键名（全在这里，前缀 `role.` 由本契约 §2 声明，D10）。
M.KEY = {
  DEFS = "role.defs",
  SKILLS = "role.skills",
  INDEX = "role.index",
  WINDOW = "role.window.",
}

--- 实体记录键：`role.actor.<id>`。
function M.actor_key(id)
  return "role.actor." .. tostring(id)
end

--- 近期流键：`role.window.<id>`（独立键族，契约 D12）。
function M.window_key(id)
  return M.KEY.WINDOW .. tostring(id)
end

--- 窗口 ring 的缺省容量（契约 §2 / §4 `window_cap`）。
M.DEFAULT_WINDOW_CAP = 100

local installed = nil

--- 装好的窗口容量；未指定 / 未安装 ⇒ 缺省值（纯读，不碰宿主面）。
function M.window_cap()
  if installed ~= nil and installed.window_cap ~= nil then
    return installed.window_cap
  end
  return M.DEFAULT_WINDOW_CAP
end

--- 装配：记 `{ host?, entity?, codec?, events?, window_cap? }`（不预读、不预写；契约 I8）。
---
--- `host` 与 `ygg` 两个键都收：批次里的四个包（entity / codec / quest / dialogue）
--- 用 `host`，这条统一拼法不该因为一个包的契约草稿换个名字就分裂。
function M.install(deps)
  if deps ~= nil and type(deps) ~= "table" then
    M.fail(M.E.INSTALL, "install 只收 table 或 nil，得到 " .. type(deps))
  end
  local cap = type(deps) == "table" and deps.window_cap or nil
  if cap ~= nil then
    if type(cap) ~= "number" or cap ~= (cap // 1) or cap < 1 then
      M.fail(M.E.INSTALL, "window_cap 必须是正整数，得到 " .. tostring(cap))
    end
    cap = cap | 0
  end
  installed = {
    host = type(deps) == "table" and (deps.host or deps.ygg) or nil,
    entity = type(deps) == "table" and deps.entity or nil,
    codec = type(deps) == "table" and deps.codec or nil,
    window_cap = cap,
  }
  return M
end

--- 宿主面（`deps.host` 优先，其次 `ygg` 全局）。
function M.host()
  local current = installed and installed.host or nil
  if current == nil then
    current = ygg
  end
  if current == nil then
    M.fail(M.E.DEPS, "ygg 不可达：沙箱外调用需 `install({ host = ... })`")
  end
  return current
end

-- ── 依赖包解析（缺失即硬失败，不降级去猜别的存储）────────────────────────────

local function resolve(kind, name, injected, required)
  local module = injected
  if module == nil then
    local ok, loaded = pcall(require, name)
    if ok then
      module = loaded
    end
  end
  if type(module) ~= "table" then
    M.fail(M.E.DEPS, "缺 " .. kind .. "：install({ " .. kind .. " = ... }) 或可 require 的 " .. name)
  end
  for i = 1, #required do
    if type(module[required[i]]) ~= "function" then
      M.fail(M.E.DEPS, kind .. " 面必须导出 " .. required[i] .. "(...)")
    end
  end
  return module
end

--- 属性袋真源（`entity.get` / `entity.has` 是**唯一**允许的读面，契约 §2 的接口缝）。
function M.entity()
  return resolve("entity", "pkg.entity", installed and installed.entity or nil, { "get", "has" })
end

--- 确定性编码（四个 KV 键的载荷口径）。
function M.codec()
  return resolve("codec", "pkg.codec", installed and installed.codec or nil, { "encode", "decode" })
end

-- ── 四个键的读写（唯一的 KV 流量入口）──────────────────────────────────────

--- 读一个键：`nil` → `default`；非字符串载荷（不是本包写的）即 `-KV`。
function M.read(key, default)
  local text = M.host().kv.get(key)
  if text == nil then
    return default
  end
  if type(text) ~= "string" then
    M.fail(M.E.KV, "键 " .. key .. " 的载荷必须是 codec 编码的字符串，得到 " .. type(text))
  end
  return M.codec().decode(text)
end

--- 写一个键：值一律先 `codec.encode`（表写不进 KV，见文件头）。
function M.write(key, value)
  M.host().kv.set(key, M.codec().encode(value))
end

-- ── 派发事件（派生事件的排队入口也收口在这里）──────────────────────────────

--- 排队一条派生事件。事件名是**固定集合**（见 `pkg.role_skill.EVENTS`）：名字不随
--- 角色名 / 技能名变化，订阅方不必猜。
function M.emit(name, payload)
  local host = M.host()
  if type(host.emit) ~= "function" then
    M.fail(M.E.DEPS, "宿主面缺 emit(name, payload)")
  end
  host.emit(name, payload)
end

return M
