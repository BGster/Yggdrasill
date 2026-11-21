-- import：用例段自 pkg-codec.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, check, eq, is_true, is_nil, expect_error, run, same_deep, new_world, fresh, basic_spec, sorted_keys =
    C.S, C.check, C.eq, C.is_true, C.is_nil, C.expect_error, C.run, C.same_deep, C.new_world, C.fresh, C.basic_spec, C.sorted_keys
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

end
