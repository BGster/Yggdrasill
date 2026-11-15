-- readgate：读签门禁（迁移自 Rust 对照套件 32–52 例，逐条对齐）。

test("签名记录 ver|scene|seq 并登记索引（settings. 前缀等价、重签覆盖不重复）", function()
  fresh()
  charter_and_begin()

  reset()
  fire("config.set", "read.acts/coordinate=9f2c1a")
  assert_true(narrated_contains("[SIGNED] acts/coordinate@9f2c1a"), "must narrate the signature")
  assert_eq(kv("settings.read.acts/coordinate"), "9f2c1a|coordinate|0", "value shape is ver|scene|seq")
  assert_eq(kv("settings.read_index"), "acts/coordinate", "the index registers the doc")

  -- `settings.` 前缀写法等价；同键重签覆盖 ver，索引不重复登记。
  fire("config.set", "settings.read.acts/coordinate=aa11bb")
  assert_eq(kv("settings.read.acts/coordinate"), "aa11bb|coordinate|0", "settings. prefix is equivalent")
  assert_eq(kv("settings.read_index"), "acts/coordinate", "re-signing must not duplicate the index")

  -- 第二件：索引追加，两件互不覆盖。
  fire("config.set", "read.coordinate/dispatch=c0ffee")
  assert_eq(kv("settings.read.coordinate/dispatch"), "c0ffee|coordinate|0", "second doc keeps its own key")
  assert_eq(kv("settings.read_index"), "acts/coordinate,coordinate/dispatch", "index appends in order")
end)

test("签名拒绝：坏 doc-id / 超长 / 坏 ver / 索引键不可写，且零写入", function()
  fresh()
  charter_and_begin()

  local bad_ids = {
    "read.acts=9f2c1a", -- 缺 <doc-id> 段（无斜杠）
    "read.acts/sub/dir=9f2c1a", -- 斜杠过多（只认一段）
    "read.acts/charter.md=9f2c1a", -- 点号：effective() 会下钻错位
    "read.Acts/Coordinate=9f2c1a", -- 大写
    "read./coordinate=9f2c1a", -- 幕段空
    "read.acts/" .. string.rep("a", 49) .. "=9f2c1a", -- doc-id 超长
    "read." .. string.rep("a", 33) .. "/coordinate=9f2c1a", -- 幕段超长
  }
  for _, payload in ipairs(bad_ids) do
    reset()
    fire("config.set", payload)
    assert_true(narrated_contains("[REJECTED"), "must refuse: " .. payload)
  end
  assert_nil(kv("settings.read_index"), "refusals must not touch the index")
  assert_nil(kv("settings.read.acts"), "refusals must not leave stray keys")

  for _, payload in ipairs({
    "read.acts/coordinate=xyz", -- 非 hex
    "read.acts/coordinate=9F2C1A", -- 大写 hex 不收
    "read.acts/coordinate=abc", -- 短于 4
  }) do
    reset()
    fire("config.set", payload)
    assert_true(narrated_contains("[REJECTED"), "must refuse: " .. payload)
  end
  assert_nil(kv("settings.read.acts/coordinate"), "bad ver must write nothing")

  -- 索引键不在用户面（无 read. 前缀 → bad_config_key），只能由 mod 自维护。
  reset()
  fire("config.set", "read_index=acts/coordinate")
  assert_true(narrated_contains("[REJECTED"), "the index key is not user-writable")
  assert_nil(kv("settings.read_index"), "index stays untouched")
end)
