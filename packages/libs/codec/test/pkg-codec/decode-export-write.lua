-- decode-export-write：用例段自 pkg-codec.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, same_deep, new_world, fresh, basic_spec, sorted_keys =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.same_deep, C.new_world, C.fresh, C.basic_spec, C.sorted_keys
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

end
