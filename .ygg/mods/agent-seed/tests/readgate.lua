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
