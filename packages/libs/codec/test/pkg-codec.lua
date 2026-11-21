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

  -- 宽表（契约基准集的 `kv` 段就是 2 000 键，见 §5）：键序不因排序算法实现而变。
  -- 逆序插入逼出最坏插入序，输出里的键必须**严格升序**出现。
  local WIDTH = 1000
  local wide = {}
  for i = WIDTH, 1, -1 do
    wide[("k%04d"):format(i)] = i
  end
  local text = codec.encode(wide)
  local cursor = 1
  local seen = 0
  local previous = nil
  while true do
    local start, finish = text:find('"k%d%d%d%d"', cursor)
    if start == nil then
      break
    end
    local key = text:sub(start + 1, finish - 1)
    if previous ~= nil then
      is_true(previous < key, "宽表键序必须严格升序：" .. previous .. " → " .. key)
    end
    previous = key
    seen = seen + 1
    cursor = finish + 1
  end
  eq(seen, WIDTH, "宽表：全部键都必须出现在输出里")
  local forward = {}
  for i = 1, WIDTH do
    forward[("k%04d"):format(i)] = i
  end
  eq(codec.encode(forward), text, "宽表：正序插入与逆序插入字节相同")
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

-- ── 用例 10-13：export 形状 / ids 模式 / kv 点名 / checksum 互校 ─────────────

run("pkg.codec.export-shape", function()
  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())
  local first = entity.spawn("creature", { hp = 7 }, { "enemy" })
  local second = entity.spawn("creature")

  local bundle = codec.export({ entities = true, ids = "index", mod = "probe" })

  local keys = sorted_keys(bundle)
  eq(#keys, 5, "bundle 顶层键数")
  eq(keys[1], "checksum", "bundle 键 1")
  eq(keys[2], "codec_version", "bundle 键 2")
  eq(keys[3], "entities", "bundle 键 3")
  eq(keys[4], "exported_from", "bundle 键 4")
  eq(keys[5], "kv", "bundle 键 5")
  eq(bundle.codec_version, "1", "codec_version")
  eq(bundle.exported_from, "probe", "导出源 = 调用方传入（宿主无 mod 名面，D10）")
  eq(bundle.checksum, "", "export 不算校验和（那是 checksum/write 的事）")

  eq(#bundle.entities, 2, "两条记录")
  eq(bundle.entities[1].id, first, "记录按 id 升序")
  eq(bundle.entities[2].id, second, "记录按 id 升序")

  local record_keys = sorted_keys(bundle.entities[1])
  eq(#record_keys, 4, "记录键数")
  eq(record_keys[1], "attrs", "记录键 1")
  eq(record_keys[2], "id", "记录键 2")
  eq(record_keys[3], "schema", "记录键 3")
  eq(record_keys[4], "tags", "记录键 4")
  eq(bundle.entities[1].schema, "creature", "记录带 schema 名")
  eq(bundle.entities[1].attrs.hp, 7, "显式值入档")
  eq(bundle.entities[2].attrs.hp, 100, "缺省值也入档（读回才能重建）")
  eq(#bundle.entities[1].tags, 1, "tag 数")
  eq(bundle.entities[1].tags[1], "enemy", "tag 名")
end)

run("pkg.codec.export-ids-mode", function()
  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())
  entity.spawn("creature")
  local target = entity.spawn("creature")
  entity.spawn("creature")

  local all = codec.export({ entities = true, ids = "index" })
  eq(#all.entities, entity.count(), 'ids="index" 的条数 = entity.count()')
  eq(#all.entities, 3, "三条")

  local one = codec.export({ entities = true, ids = { target } })
  eq(#one.entities, 1, "显式 id 列表只导那几条")
  eq(one.entities[1].id, target, "导的就是那一条")

  local none = codec.export({ entities = false })
  eq(#none.entities, 0, "entities=false 不导实体")
  eq(next(none.entities), nil, "实体段为空表")

  expect_error("YGG-PKG-CODEC-TYPE", function()
    codec.export({ entities = true, ids = 42 })
  end, "ids 类型非法")
end)

run("pkg.codec.export-kv-keys", function()
  local world = new_world()
  local _, codec = fresh(world)
  world.kv["quest.1"] = "main"
  world.kv["dlg.2"] = 7
  world.kv["other.3"] = true

  local named = codec.export({ keys = { "quest.1", "dlg.2" } })
  eq(#sorted_keys(named.kv), 2, "只导点名的两个键（不遍历）")
  eq(named.kv["quest.1"], "main", "kv 值 1")
  eq(named.kv["dlg.2"], 7, "kv 值 2")
  is_nil(named.kv["other.3"], "未点名的键不得入档")

  -- 不传 keys：kv 段为空表，**不**遍历（宿主也没有遍历面）。
  local bare = codec.export({})
  eq(next(bare.kv), nil, "不传 keys 则 kv 为空")
  -- 点名了但不存在的键：跳过，不写 nil（JSON 无 null 槽位）。
  local missing = codec.export({ keys = { "nope.9" } })
  eq(next(missing.kv), nil, "不存在的键被跳过")
end)

run("pkg.codec.write-integrity", function()
  local bundle = {
    codec_version = "1",
    exported_from = "probe",
    entities = {},
    kv = {},
    checksum = "",
  }

  -- ① 宿主不回 sha256（只 path/bytes）→ 不报错，返回我方值。
  do
    local world = new_world()
    local _, codec = fresh(world)
    local expected = codec.checksum(bundle)
    world.fs_sha_mode = nil
    local out = codec.write("a.json", bundle)
    eq(out.sha256, expected, "宿主不回值 ⇒ 用包内自算")
    eq(out.bytes, #world.files["a.json"], "bytes 是实际写入长度")
  end

  -- ② 宿主回相同的值 → 不报错。
  do
    local world = new_world()
    local _, codec = fresh(world)
    world.fs_sha_mode = "echo"
    world.fs_sha_value = codec.checksum(bundle)
    local out = codec.write("b.json", bundle)
    eq(out.sha256, world.fs_sha_value, "两套实现一致时不报警")
  end

  -- ③ 回执字节数对不上（截断 / 半写）→ YGG-PKG-CODEC-CHECKSUM。
  --    （S16 后不再比宿主 sha256：它算写出文本，包内 digest 算 `checksum=""` 的编码，
  --    两份字节永不相同；内容面由 `read` 的强制 checksum 兜底。）
  do
    local world = new_world()
    local _, codec = fresh(world)
    world.host.fs.write = function(path, content)
      return { path = path, bytes = #content + 1 }
    end
    expect_error("YGG-PKG-CODEC-CHECKSUM", function()
      codec.write("c.json", bundle)
    end, "写入字节数不符必须报错")
  end

  -- checksum 不修改入参（纯读面）。
  local before = {}
  for key, value in pairs(bundle) do
    before[key] = value
  end
  local world = new_world()
  local _, codec = fresh(world)
  codec.checksum(bundle)
  eq(bundle.checksum, before.checksum, "checksum 不得改写入参")
end)

-- ── 用例 20：往返（版本截面归 C5，import 就位后补齐）───────────────────────

run("pkg.codec.import-version-and-roundtrip", function()
  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())
  entity.spawn("creature", { hp = 5 }, { "boss" })
  entity.spawn("creature")

  local report = codec.roundtrip("save.json")
  is_true(report.equal, "往返字节必须逐字相等")
  eq(report.bytes_in, report.bytes_out, "往返字节数相等")
  is_true(report.bytes_in > 0, "字节数非零")

  -- 读回就是导出物（读盘 + 校验和都过了一道）。
  local loaded = codec.read("save.json")
  eq(#loaded.entities, 2, "读回两条记录")
  eq(loaded.entities[1].attrs.hp, 5, "读回属性值")

  -- 缺失文件与坏 checksum 是两个不同的码。
  expect_error("YGG-PKG-CODEC-MISSING", function()
    codec.read("nope.json")
  end, "缺失文件")
  local tampered = world.files["save.json"]:gsub("\"hp\":5", "\"hp\":6")
  world.files["tampered.json"] = tampered
  expect_error("YGG-PKG-CODEC-CHECKSUM", function()
    codec.read("tampered.json")
  end, "篡改后 checksum 不符")

  -- 版本截面：未知 codec_version 必须拒（契约 §1：只拒，不猜）。
  local bundle = codec.export({ entities = true, ids = "index" })
  bundle.checksum = codec.checksum(bundle)
  bundle.codec_version = "9"
  expect_error("YGG-PKG-CODEC-VERSION", function()
    codec.import(bundle, { on_conflict = "skip" })
  end, "未知版本")
end)

-- ── 用例 14-19：import 的两阶段与三种冲突策略 ─────────────────────────────────

--- 造一份「从 A 局导出」的 bundle（n 条记录，属性值各不相同）并**封签**。
---
--- 封签是模拟「从盘上读回来的档」：越过信任边界的 bundle 一定带 checksum。
--- 注意：这会切换当前模块实例的宿主，调用方随后必须自己 `fresh()` 一次新世界。
local function exported_bundle(count)
  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())
  for i = 1, count do
    entity.spawn("creature", { hp = i * 10, title = "t" .. i }, { "enemy" })
  end
  local bundle = codec.export({ entities = true, ids = "index", mod = "probe" })
  bundle.checksum = codec.checksum(bundle)
  return bundle
end

run("pkg.codec.import-conflict-required", function()
  local bundle = exported_bundle(2)
  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())

  -- 策略必选：缺省不是 skip，而是**报错**（契约 I3：退路默认值会让一次误调用
  -- 静默丢掉一整份档的差异）。
  expect_error("YGG-PKG-CODEC-CONFLICT", function()
    codec.import(bundle, {})
  end, "缺 on_conflict")
  expect_error("YGG-PKG-CODEC-CONFLICT", function()
    codec.import(bundle, { on_conflict = "merge" })
  end, "非法策略值")
  expect_error("YGG-PKG-CODEC-CONFLICT", function()
    codec.import(bundle)
  end, "缺 opts")

  -- 显式策略才通过。
  local report = codec.import(bundle, { on_conflict = "skip" })
  eq(report.created, 2, "显式 skip 通过")
end)

run("pkg.codec.import-idempotent", function()
  local bundle = exported_bundle(3)

  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())

  local first = codec.import(bundle, { on_conflict = "overwrite" })
  eq(first.created, 3, "首次导入建 3 条")
  eq(entity.count(), 3, "实体数")
  local snapshot = {}
  for i = 1, 3 do
    local id = entity.ids()[i]
    snapshot[id] = entity.attrs(id)
  end

  -- 第二次导入：一条也不新建，属性快照不变（契约 I2 的直接断言）。
  local second = codec.import(bundle, { on_conflict = "overwrite" })
  eq(second.created, 0, "二次导入 created 必须为 0")
  eq(second.overwritten, 3, "二次导入全部走覆盖")
  eq(entity.count(), 3, "实体数不变")
  for i = 1, 3 do
    local id = entity.ids()[i]
    is_true(same_deep(snapshot[id], entity.attrs(id)), "实体 " .. id .. " 的属性快照不变")
  end
end)

run("pkg.codec.import-skip-zero-writes", function()
  local bundle = exported_bundle(2)

  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())
  entity.spawn("creature", { hp = 1 })
  entity.spawn("creature", { hp = 2 })

  local writes_before = world.kv_writes_now()
  local report = codec.import(bundle, { on_conflict = "skip" })
  eq(report.created, 0, "skip 不新建")
  eq(report.skipped, 2, "skip 跳过两条")
  eq(world.kv_writes_now(), writes_before, "skip 必须是零写")
  eq(entity.count(), 2, "实体数不变")
end)

run("pkg.codec.import-preserves-ids", function()
  local bundle = exported_bundle(4)
  local original = {}
  for i = 1, #bundle.entities do
    original[i] = bundle.entities[i].id
  end

  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())

  local report = codec.import(bundle, { on_conflict = "skip" })
  eq(report.created, 4, "全部新建")
  eq(next(report.renamed), nil, "空局升序全量导入不得发生重命名")
  local ids = entity.ids()
  eq(#ids, #original, "id 数相同")
  for i = 1, #original do
    eq(ids[i], original[i], "第 " .. i .. " 个 id 与源档相同")
  end
  eq(report.refs, 0, "无重命名则无悬垂引用")
end)

run("pkg.codec.import-rename-map", function()
  -- 从 5 条的局里只导最后一条（id = 5）——导入到一个空局，内核只会给它 1。
  local full = exported_bundle(5)
  local subset = {
    codec_version = full.codec_version,
    exported_from = full.exported_from,
    entities = { full.entities[5] },
    kv = {},
    checksum = "",
  }
  -- 让这条记录带一个指向自己旧 id 的 ref（I6：引用不重写，只报数）。
  subset.entities[1].attrs.target = 5

  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())
  subset.checksum = codec.checksum(subset)

  local report = codec.import(subset, { on_conflict = "rename" })
  eq(report.created, 1, "只建一条")
  eq(report.renamed[5], 1, "旧 id 5 → 新 id 1")
  eq(entity.count(), 1, "实体数 1")
  -- 引用**原值原样**写入（不重映射到 1）。
  eq(entity.get(1, "target"), 5, "ref 属性仍是旧 id")
  eq(report.refs, 1, "返回体报 1 个 ref 指向被重命名的 id")
  eq(entity.exists(5), false, "旧 id 在本局不存在")
end)

run("pkg.codec.import-checksum-first", function()
  local bundle = exported_bundle(2)
  -- 篡改一条记录的值，**不动** checksum 字段：这就是「坏档」的真实形状。
  bundle.entities[1].attrs.hp = 999

  local world = new_world()
  local entity, codec = fresh(world)
  entity.define("creature", basic_spec())

  local writes_before = world.kv_writes_now()
  expect_error("YGG-PKG-CODEC-CHECKSUM", function()
    codec.import(bundle, { on_conflict = "overwrite" })
  end, "篡改后导入被拒")
  eq(world.kv_writes_now(), writes_before, "校验和先行：零部分导入")
  eq(entity.count(), 0, "一条也不得落地")

  -- 两态是**显式**的：封签的 bundle `verified = true`，未封签的（`export` 直出）
  -- `verified = false`——后者不是「校验通过」，是「没有可校的声明」。
  local sealed = exported_bundle(1)
  local w1 = new_world()
  local e1, c1 = fresh(w1)
  e1.define("creature", basic_spec())
  eq(c1.import(sealed, { on_conflict = "skip" }).verified, true, "封签的 bundle 校验过")

  local unsealed = c1.export({ entities = true, ids = "index" })
  eq(unsealed.checksum, "", "export 产出的 bundle 未封签")
  local w2 = new_world()
  local e2, c2 = fresh(w2)
  e2.define("creature", basic_spec())
  local report = c2.import(unsealed, { on_conflict = "skip" })
  eq(report.verified, false, "未封签的 bundle 不得声称校验过")
  eq(report.created, 1, "未封签也能导入（内存态倒手，空局建一条）")
end)
