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

--- 升序键列表（测试里允许排序库：这里跑的是普通 Lua，不是沙箱）。
local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
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

-- ── 用例 9：sha256 已知答案（D11）─────────────────────────────────────────────
--
-- 向量取自独立实现（`python3 -c "hashlib.sha256"` 与本机 `shasum -a 256` 对拍，
-- 见 development 回执的原始输出）。覆盖空串、单字节、55/56/64/65 四个填充边界、
-- 与经典的 448-bit 双块向量——填充与长度尾写错时这五条必有一条红。

run("pkg.codec.sha256-vectors", function()
  local world = new_world()
  local _, codec = fresh(world)

  eq(
    codec.sha256(""),
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "空串"
  )
  eq(
    codec.sha256("abc"),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    "abc"
  )
  eq(
    codec.sha256(("a"):rep(55)),
    "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
    "55 字节（填充边界前一格）"
  )
  eq(
    codec.sha256(("a"):rep(56)),
    "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
    "56 字节（需跨块）"
  )
  eq(
    codec.sha256(("a"):rep(64)),
    "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
    "64 字节（整块无余量）"
  )
  eq(
    codec.sha256(("a"):rep(65)),
    "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
    "65 字节（整块 + 1）"
  )
  eq(
    codec.sha256("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
    "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    "448-bit 双块经典向量"
  )

  -- 输入域：非字符串不得被隐式 tostring（那会把 `table: 0x...` 的地址混进字节）。
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.sha256({})
  end, "非字符串输入被拒")
  -- 输出形状：64 位小写 hex（不是 0x 前缀、不是大写）。
  eq(#codec.sha256(""), 64, "输出长度")
  eq(codec.sha256("") == codec.sha256(""):lower(), true, "输出小写")
end)
