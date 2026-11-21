-- `pkg.codec.*` 自测（20 条行为用例；perf 在 `pkg-codec-perf.lua`，file 级断言不计数）。
--
-- # 载体：真包 + mock 宿主
--
-- 这里装的**不是** codec 的替身：`pkg.entity` 与 `pkg.codec` 都是真模块，只是宿主面
-- 换成 mock（KV / EntityStore / emit / fs 四个面，语义照抄 `crates/ygg-core` 与
-- `crates/ygg-sandbox/src/host_api.rs`）。于是 codec 走的是**真的** entity API——
-- 两包之间的接口缝（codec 不读 `pkg.entity.*` 任何键）被这套用例顺带钉住。
--
-- # 隔离口径
--
-- 每条用例 `fresh()`：清两个包的 `package.loaded`（模块级 deps / memo 归零）+ 新 mock。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function check(name, ok, detail)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. (detail and (": " .. tostring(detail)) or ""))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected or math.type(actual) ~= math.type(expected) then
    error(
      ("%s: expected %s (%s), got %s (%s)"):format(
        label or "eq",
        tostring(expected),
        tostring(math.type(expected)),
        tostring(actual),
        tostring(math.type(actual))
      ),
      2
    )
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy, got " .. tostring(value), 2)
  end
end

local function is_nil(value, label)
  if value ~= nil then
    error((label or "is_nil") .. ": expected nil, got " .. tostring(value), 2)
  end
end

local function expect_error(code, fn, label)
  local ok, err = pcall(fn)
  if ok then
    error((label or "expect_error") .. ": expected " .. code .. ", but the call succeeded", 2)
  end
  if not tostring(err):find(code, 1, true) then
    error(("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)), 2)
  end
end

local function run(name, fn)
  local ok, err = pcall(fn)
  check(name, ok, ok and nil or err)
end

--- 深相等（含 `math.type`：`1` 与 `1.0` 在序列化面不是同一个值）。
local function same_deep(a, b)
  if type(a) ~= type(b) then
    return false
  end
  if type(a) ~= "table" then
    return a == b and math.type(a) == math.type(b)
  end
  local count = 0
  for key, value in pairs(a) do
    count = count + 1
    if not same_deep(value, b[key]) then
      return false
    end
  end
  for _ in pairs(b) do
    count = count - 1
  end
  return count == 0
end

-- ── mock 宿主面（语义对齐内核；见文件头注）─────────────────────────────────────

local function new_world()
  local kv = {}
  local kv_writes = 0
  local store = { next_id = 1, attrs = {}, tags = {} }
  local events = {}
  local files = {}
  local world = {
    kv = kv,
    store = store,
    events = events,
    files = files,
    kv_writes = 0,
    fs_sha_mode = nil, -- nil = 不回 sha256；"echo" = 回 fs_sha_value；"wrong" = 回错值
    fs_sha_value = nil,
  }

  world.host = {
    kv = {
      get = function(key)
        return kv[key]
      end,
      set = function(key, value)
        kv_writes = kv_writes + 1
        kv[key] = value
      end,
    },
    entity = {
      spawn = function()
        local id = store.next_id
        store.next_id = id + 1
        store.attrs[id] = {}
        store.tags[id] = {}
        return id
      end,
      get_attribute = function(id, name)
        local bag = store.attrs[id]
        return bag and bag[name] or nil
      end,
      set_attribute = function(id, name, value)
        local bag = store.attrs[id]
        if bag == nil then
          error("entity " .. tostring(id) .. " not found", 0)
        end
        bag[name] = value
      end,
      add_tag = function(id, tag)
        local set = store.tags[id]
        if set == nil then
          error("entity " .. tostring(id) .. " not found", 0)
        end
        if set[tag] then
          return false
        end
        set[tag] = true
        return true
      end,
      remove_tag = function(id, tag)
        local set = store.tags[id]
        if set == nil then
          error("entity " .. tostring(id) .. " not found", 0)
        end
        if not set[tag] then
          return false
        end
        set[tag] = nil
        return true
      end,
      has_tag = function(id, tag)
        local set = store.tags[id]
        return set ~= nil and set[tag] == true
      end,
      -- S9 控制面两键的替身：存在性 = store 里还有这条记录。
      exists = function(id)
        return store.attrs[id] ~= nil
      end,
      destroy = function(id)
        if store.attrs[id] == nil then
          return false
        end
        store.attrs[id] = nil
        store.tags[id] = nil
        return true
      end,
    },
    emit = function(name, payload)
      events[#events + 1] = { name = name, payload = payload }
    end,
    fs = {
      write = function(path, content)
        files[path] = content
        local out = { path = path, bytes = #content }
        if world.fs_sha_mode == "echo" then
          out.sha256 = world.fs_sha_value
        elseif world.fs_sha_mode == "wrong" then
          out.sha256 = ("0"):rep(64)
        end
        return out
      end,
      read = function(path)
        return files[path]
      end,
    },
  }

  world.kv_writes_now = function()
    return kv_writes
  end
  world.key_count = function()
    local n = 0
    for _ in pairs(kv) do
      n = n + 1
    end
    return n
  end
  return world
end

--- 装真包（entity 先、codec 后：依赖序）并返回两个句柄。
local function fresh(world)
  package.loaded["pkg.entity"] = nil
  package.loaded["pkg.codec"] = nil
  local entity = require("pkg.entity")
  entity.install({ host = world.host })
  local codec = require("pkg.codec")
  codec.install({ host = world.host })
  return entity, codec
end

--- 标准 fixture：4 属性（含必填+缺省 / 可选 / 布尔 / 引用）+ 2 tag。
local function basic_spec()
  return {
    attrs = {
      hp = { type = "number", required = true, default = 100 },
      title = { type = "string" },
      alive_flag = { type = "boolean", default = true },
      target = { type = "ref" },
    },
    tags = { enemy = {}, boss = {} },
  }
end

--- 升序键列表（测试里允许排序库：这里跑的是普通 Lua，不是沙箱）。
local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

-- ── 用例 9：sha256 已知答案（D11）─────────────────────────────────────────────
--
-- 向量取自独立实现（`python3 -c "hashlib.sha256"` 与本机 `shasum -a 256` 对拍，
-- 见 development 回执的原始输出）。覆盖空串、单字节、55/56/64/65 四个填充边界、
-- 与经典的 448-bit 双块向量——填充与长度尾写错时这五条必有一条红。


-- === S-25 批十三测试件连拆装配：用例段搬入兄弟件（test/codec/<theme>.lua），执行次序 = 装配次序 ===
local C = {
  S = S,
  check = check,
  eq = eq,
  is_true = is_true,
  is_nil = is_nil,
  expect_error = expect_error,
  run = run,
  same_deep = same_deep,
  new_world = new_world,
  fresh = fresh,
  basic_spec = basic_spec,
  sorted_keys = sorted_keys
}

dofile(PKG_ROOT .. "/codec/test/pkg-codec/sha256-encode.lua")(C)
dofile(PKG_ROOT .. "/codec/test/pkg-codec/decode-export-write.lua")(C)
dofile(PKG_ROOT .. "/codec/test/pkg-codec/import.lua")(C)
