-- sha256-encode：用例段自 pkg-codec.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, same_deep, new_world, fresh, basic_spec, sorted_keys =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.same_deep, C.new_world, C.fresh, C.basic_spec, C.sorted_keys
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

end
