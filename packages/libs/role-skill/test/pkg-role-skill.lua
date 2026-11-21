-- `pkg.role-skill.*` 自测。契约 `docs/design/packages/role-skill.md` §7。
--
-- # 为什么自带 mock 与计数
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）：这里就地造一个**语义等价**的
-- 宿主面——`kv` 只收标量、`emit` 记账、`on` 记 handler；`entity` 只暴露本包允许
-- 调用的两个读面（`get` / `has`），**没有**写面：接口缝一旦被越过，代用面直接
-- `nil` 索引报错，比事后核对调用清单更硬。
--
-- 编码面用**真的** `pkg.codec`：KV 载荷字符串就是它产的，mock 一份编码器等于把
-- 「载荷是不是确定性字节」这条一起 mock 掉。
--
-- # 隔离口径
--
-- 每条用例 `fresh()` 一次：清 `pkg.role_skill*` 全部 `package.loaded` 项 + 新建一份
-- mock。用例之间不共享任何状态（含 `role.index` 与订阅表）。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-role-skill.lua`，`--filter pkg-role-skill` 跑到它。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function run(name, body)
  local ok, err = pcall(body)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(err))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error(
      ("%s: expected %s, got %s"):format(tostring(label or "eq"), tostring(expected), tostring(actual)),
      2
    )
  end
end

local function eq_seq(actual, expected, label)
  eq(#actual, #expected, (label or "eq_seq") .. " 长度")
  for i = 1, #expected do
    eq(actual[i], expected[i], (label or "eq_seq") .. "[" .. i .. "]")
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy", 2)
  end
end

local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

local function expect_error(code, body, label)
  local ok, err = pcall(body)
  if ok then
    error((label or "expect_error") .. ": expected " .. code .. ", but the call succeeded", 2)
  end
  if not tostring(err):find(code, 1, true) then
    error(("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)), 2)
  end
end

local codec = require("pkg.codec")
codec.install()

-- ── mock 宿主面（语义对齐内核；见文件头）───────────────────────────────────────

local function new_world()
  local kv = {}
  local reads, writes = 0, 0
  local events = {}
  local handlers = {}
  local attrs = {}
  local tags = {}

  local entity = {
    get = function(id, key)
      local bag = attrs[id]
      if bag == nil then
        return nil
      end
      return bag[key]
    end,
    has = function(id, tag)
      local set = tags[id]
      return set ~= nil and set[tag] == true
    end,
  }

  local world = {
    kv = kv,
    events = events,
    handlers = handlers,
    entity = entity,
  }

  world.host = {
    emit = function(name, payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
    kv = {
      get = function(key)
        reads = reads + 1
        return kv[key]
      end,
      set = function(key, value)
        writes = writes + 1
        kv[key] = value
      end,
    },
    on = function(name, handler)
      handlers[name] = handler
    end,
  }

  -- 测试侧的布置面（不走被测代码）：属性 / 标签只经这两个口子改。
  world.set_attr = function(id, key, value)
    if attrs[id] == nil then
      attrs[id] = {}
    end
    attrs[id][key] = value
  end
  world.add_tag = function(id, tag)
    if tags[id] == nil then
      tags[id] = {}
    end
    tags[id][tag] = true
  end
  -- 事件日志是**脚手架**：真实宿主把派生事件排队、派发后释放。泄漏用例要测的是包的
  -- 增长面，不清日志就变成测「日志数组增长」——那是 mock 的性质，不是包的性质。
  world.clear_events = function()
    for i = #events, 1, -1 do
      events[i] = nil
    end
  end
  world.reads_now = function()
    return reads
  end
  world.writes_now = function()
    return writes
  end
  return world
end

--- 取一份**全新**的包实例并装上 mock（清缓存 = 定义表 / 索引缓存 / 订阅表归零）。
local function fresh(world, events, window_cap)
  package.loaded["pkg.role_skill"] = nil
  for name in pairs(package.loaded) do
    if type(name) == "string" and name:find("^pkg%.role_skill%.") then
      package.loaded[name] = nil
    end
  end
  local role = require("pkg.role_skill")
  role.install({
    codec = codec,
    entity = world.entity,
    events = events,
    host = world.host,
    window_cap = window_cap,
  })
  return role
end

local function events_named(world, name)
  local out = {}
  for i = 1, #world.events do
    if world.events[i].name == name then
      out[#out + 1] = world.events[i]
    end
  end
  return out
end

--- 一次 `grant` 之后把记录解码出来（载荷必须是字符串这件事另有用例钉）。
local function record_of(world, id)
  local raw = world.kv["role.actor." .. id]
  if raw == nil then
    return nil
  end
  return codec.decode(raw)
end

-- ── 用例 1：KV 载荷形状（字符串 + 恰好三个键）────────────────────────────────

local function pkg_sources(id)
  local manifest = rawget(_G, "__MANIFEST")
  if type(manifest) ~= "table" or type(manifest.packages) ~= "table" then
    error("manifest 缺 packages 段（载体 pack.json 未声明包或未重算 manifest）", 0)
  end
  for i = 1, #manifest.packages do
    local package = manifest.packages[i]
    if package.id == id then
      local out = {}
      for j = 1, #package.files do
        out[#out + 1] = package.files[j].path
      end
      return out
    end
  end
  error("manifest 的 packages 段没有包 " .. id, 0)
end

local function source_text(rel)
  local full = PKG_ROOT .. "/" .. rel
  local handle = assert(io.open(full, "rb"), "读不到 " .. full)
  local text = handle:read("a")
  handle:close()
  return text
end


-- === S-25 批十三测试件连拆装配：用例段搬入兄弟件（test/role-skill/<theme>.lua），执行次序 = 装配次序 ===
local C = {
  S = S,
  run = run,
  eq = eq,
  eq_seq = eq_seq,
  is_true = is_true,
  sorted_keys = sorted_keys,
  expect_error = expect_error,
  codec = codec,
  new_world = new_world,
  fresh = fresh,
  events_named = events_named,
  record_of = record_of,
  pkg_sources = pkg_sources,
  source_text = source_text
}

dofile(PKG_ROOT .. "/role-skill/test/pkg-role-skill/counters.lua")(C)
dofile(PKG_ROOT .. "/role-skill/test/pkg-role-skill/capacity-window.lua")(C)
dofile(PKG_ROOT .. "/role-skill/test/pkg-role-skill/perf-compat.lua")(C)
