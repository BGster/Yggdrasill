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

-- ── 用例 1-7：encode（确定性字节 / 键序 / 数字 / 转义 / 拒绝面）────────────────

run("pkg.codec.encode-deterministic-bytes", function()
  local world = new_world()
  local _, codec = fresh(world)

  -- 同值两次必须**逐字节**相等。
  local value = {
    name = "creature",
    hp = 100,
    alive = true,
    tags = { "enemy", "boss" },
    nested = { a = 1, b = { 2, 3 } },
  }
  eq(codec.encode(value), codec.encode(value), "同值两次")

  -- 插入序扰动：100 键对象用两种插入序建两张表，字节必须一样。
  -- （`pairs` 序依赖哈希种子，不排序就做不到这一条。）
  local forward = {}
  for i = 1, 100 do
    forward[("k%03d"):format(i)] = i
  end
  local backward = {}
  for i = 100, 1, -1 do
    backward[("k%03d"):format(i)] = i
  end
  eq(codec.encode(forward), codec.encode(backward), "插入序扰动不影响字节")
  eq(#codec.encode(forward) > 0, true, "编码结果非空")
end)

run("pkg.codec.encode-key-order", function()
  local world = new_world()
  local _, codec = fresh(world)
  -- 字面量对拍，而不是「能解析」：“能解析”对任何键序都成立，测不到 I1。
  eq(codec.encode({ b = 1, a = 2 }), '{"a":2,"b":1}', "键序为字节序")
  eq(codec.encode({ z = 1, A = 2, _x = 3 }), '{"A":2,"_x":3,"z":1}', "大小写与下划线的字节序")
  eq(codec.encode({}), "{}", "空表编码为对象")
  eq(codec.encode({ 1, 2, 3 }), "[1,2,3]", "序列编码为数组")
end)

run("pkg.codec.encode-number-form", function()
  local world = new_world()
  local _, codec = fresh(world)

  eq(codec.encode(1), "1", "整数直接写")
  eq(codec.encode(1.0), "1.0", "浮点幺保持浮点身份")
  eq(codec.encode(0.1), "0.1", "浮点最短往返（不是 0.10000000000000001）")
  eq(codec.encode(1e300), "1e300", "大指数去 + 与 E")
  eq(codec.encode(-0.0), "0.0", "负零归一到零（与内核 normalize_f64 同口径）")
  is_true(codec.encode(2 ^ 63):find("e") ~= nil, "2^63 需指数形式")
  eq(codec.encode(100.0), "100.0", "量级在区间内优先十进制")

  -- 每个再 decode 回同值**同 math.type**（编解码对偶：`1` 与 `1.0` 两条路都不串形）。
  for _, pair in ipairs({ { 1, "integer" }, { 1.0, "float" }, { 0.1, "float" }, { 1e300, "float" }, { -0.0, "float" }, { 100.0, "float" } }) do
    local original = pair[1]
    local round = codec.decode(codec.encode(original))
    eq(round, original, "往返值 " .. tostring(original))
    eq(math.type(round), pair[2], "往返 math.type " .. tostring(original))
  end
end)

run("pkg.codec.encode-number-nonfinite", function()
  local world = new_world()
  local _, codec = fresh(world)
  -- NaN 与 ±inf 不是 JSON 值；静默写 null 会把「无穷大」读成「没有值」。
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode(0 / 0)
  end, "NaN 被拒")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode(math.huge)
  end, "+inf 被拒")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode(-math.huge)
  end, "-inf 被拒")
end)

run("pkg.codec.encode-string-escapes", function()
  local world = new_world()
  local _, codec = fresh(world)

  local source = '"' .. "\\" .. "\n\r\t" .. ("%c"):format(1)
  local expected = '"' .. '\\"' .. "\\\\" .. "\\n\\r\\t" .. "\\u0001" .. '"'
  eq(codec.encode(source), expected, "引号/反斜杠/换行/回车/制表/控制字符")
  eq(codec.decode(codec.encode(source)), source, "转义往返")

  -- 非 ASCII 原样（UTF-8 直通，不转 \\uXXXX）。
  eq(codec.encode("中文"), '"中文"', "非 ASCII 不转义")
  eq(codec.decode(codec.encode("中文")), "中文", "非 ASCII 往返")
  -- 手写的 `\\uXXXX` 要能读回来（含代理对）：这层不是为了自己用，是为了能读
  -- 别人生成的 JSON。
  eq(codec.decode('"\\u4e2d\\u6587"'), "中文", "\\u 转义读回")
  eq(codec.decode('"\\ud83d\\ude00"'), "\240\159\152\128", "代理对读回（😀）")
end)

run("pkg.codec.encode-rejects-holes", function()
  local world = new_world()
  local _, codec = fresh(world)

  -- `{1, nil, 3}` 在 Lua 里 `#` 是未定义的（可能 1 也可能 3），一律当洞。
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode({ 1, nil, 3 })
  end, "序列中间有 nil")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode({ [1] = 1, [3] = 3 })
  end, "显式洞")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode({ a = 1, [1] = 2 })
  end, "数组键与字符串键混用")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode({ [0] = 1 })
  end, "下标 0")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode({ [1.5] = 1 })
  end, "非整数键")
end)

run("pkg.codec.encode-rejects-non-values", function()
  local world = new_world()
  local _, codec = fresh(world)

  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode({ f = function()
      return 1
    end })
  end, "函数值")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode({ co = coroutine.create(function()
      return 1
    end) })
  end, "thread 值")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode(function()
      return 1
    end)
  end, "顶层函数")

  -- 对照组：带元表的表**可以**编码（元表不进字节，只编内容）。
  eq(codec.encode(setmetatable({ a = 1 }, { __index = function()
    return 9
  end })), '{"a":1}', "元表不进字节")

  -- 循环引用要报错而不是栈溢出。
  local loop = {}
  loop.self = loop
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.encode(loop)
  end, "循环引用")
end)

-- ── 用例 8：decode 往返 + 位置化报错 ────────────────────────────────────────

run("pkg.codec.decode-roundtrip", function()
  local world = new_world()
  local _, codec = fresh(world)

  -- 深 fixture：嵌套 + 三种标量 + 转义串 + 空表。
  local fixture = {
    name = "creature",
    hp = 100,
    ratio = 0.5,
    alive = true,
    tags = { "enemy", "boss" },
    nested = { deep = { 1, 2, { 3, "四" } }, empty = {} },
    escaped = '"' .. "\\" .. "\n\t",
  }
  is_true(same_deep(fixture, codec.decode(codec.encode(fixture))), "深 fixture 往返")

  -- 三类坏 JSON（题面点名的）：尾逗号 / 缺引号 / 截断——必须报错并带字节位置。
  for _, bad in ipairs({ '{"a":1,}', '{"a":}', '{"a":1' }) do
    local ok, err = pcall(codec.decode, bad)
    eq(ok, false, "坏 JSON 必须报错：" .. bad)
    is_true(
      tostring(err):find("YGG-PKG-CODEC-TYPE", 1, true) ~= nil,
      "错误码不对：" .. tostring(err)
    )
    is_true(tostring(err):find("at byte", 1, true) ~= nil, "错误必须带位置：" .. tostring(err))
  end

  -- 其它形态：每一类都是真实生成的 JSON 会遇到的形状。
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode("[1,]")
  end, "数组尾逗号")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode('"abc')
  end, "字符串未闭合")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode("tru")
  end, "字面量被截断")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode('{"a":1} x')
  end, "顶层值之后的残留字节")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode('{"a" 1}')
  end, "键后缺冒号")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode('"a' .. ("%c"):format(1) .. '"')
  end, "未转义的控制字符")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode(12345)
  end, "非字符串输入")

  -- null 的对偶语义：对象里 = 键不存在；数组里 = 报错（丢不得）；顶层 = nil。
  local with_null = codec.decode('{"a":null,"b":1}')
  is_nil(with_null.a, "对象 null 丢弃")
  eq(with_null.b, 1, "null 旁边的值不受影响")
  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.decode("[null]")
  end, "数组里的 null")
  is_nil(codec.decode("null"), "顶层 null = nil")

  -- 空白容忍（四种合法空白）。
  is_true(
    same_deep(codec.decode('  { "a" :\t[ 1 , 2 ]\r\n}  '), { a = { 1, 2 } }),
    "空白不影响语义"
  )
end)
