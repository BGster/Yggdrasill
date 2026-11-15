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

test("空值清除签名：值置空 + 索引摘除，重复清除幂等", function()
  fresh()
  charter_and_begin()

  fire("config.set", "read.acts/coordinate=9f2c1a")
  fire("config.set", "read.coordinate/dispatch=c0ffee")
  assert_eq(kv("settings.read_index"), "acts/coordinate,coordinate/dispatch", "both docs indexed")

  reset()
  fire("config.set", "read.acts/coordinate=")
  assert_true(narrated_contains("[SIGN] acts/coordinate cleared"), "must narrate the clear")
  assert_eq(kv("settings.read.acts/coordinate"), "", "the value is emptied, not deleted")
  assert_eq(kv("settings.read_index"), "coordinate/dispatch", "the index drops the cleared doc")

  -- 幂等：再清一次既不拒绝也不改索引。
  reset()
  fire("config.set", "read.acts/coordinate=")
  assert_true(not narrated_contains("REJECTED"), "a repeated clear must not be rejected")
  assert_eq(kv("settings.read_index"), "coordinate/dispatch", "a repeated clear leaves the index alone")
end)

test("作用域边界销票：run.begin / amend / proposal.reject 都清空签名", function()
  fresh()

  -- ① run.begin（立局幕 → 协调幕）
  fire("config.set", "read.acts/charter=aaaaaa")
  assert_eq(kv("settings.read.acts/charter"), "aaaaaa|charter|0", "signed in the charter act")
  charter_and_begin()
  assert_eq(kv("settings.read.acts/charter"), "", "run.begin wipes signatures")
  assert_eq(kv("settings.read_index"), "", "the index is cleared too")

  -- 一条在册需求（协调幕；不动 phase）
  fire("requirement.add", "synthetic requirement|synthetic description|semantics-r1")
  fire("requirement.dispatch", "0")

  -- ② amendment 推进 rev → 销票
  fire("config.set", "read.acts/coordinate=9f2c1a")
  fire("config.set", "read.coordinate/dispatch=c0ffee")
  assert_eq(kv("settings.read_index"), "acts/coordinate,coordinate/dispatch", "two docs signed")
  reset()
  fire("requirement.amend", "r1||amended description")
  assert_true(not narrated_contains("[REJECTED"), "amend must succeed")
  assert_eq(kv("settings.read.acts/coordinate"), "", "amend wipes the act doc")
  assert_eq(kv("settings.read.coordinate/dispatch"), "", "amend wipes the stage doc")
  assert_eq(kv("settings.read_index"), "", "amend clears the index")

  -- ③ proposal.reject 打回（plan 分支不碰 rev，靠 handler 的显式 clear）
  fire("config.set", "read.acts/coordinate=9f2c1a")
  assert_eq(kv("settings.read_index"), "acts/coordinate", "re-signed before submitting")
  submit(2, "plan", "semantics-read-plan")
  reset()
  fire("proposal.reject", "p1|needs rework")
  assert_true(narrated_contains("[PROPOSAL REJECTED] p1 (r1)"), "the receipt must name the proposal")
  assert_eq(kv("settings.read.acts/coordinate"), "", "a reject wipes signatures")
  assert_eq(kv("settings.read_index"), "", "the index is cleared")
end)

test("actor 侧键随签名同生共死（清除 / 换人 / 边界销票）", function()
  fresh()
  as_actor("test-agent")
  charter_and_begin()

  fire("config.set", "read.acts/coordinate=9f2c1a")
  assert_eq(kv("settings.read.acts/coordinate"), "9f2c1a|coordinate|0", "value shape")
  assert_eq(kv("settings.read_actor.acts/coordinate"), "test-agent", "side key records the signer")

  -- ① 单件清除：两个键一起变空。
  fire("config.set", "read.acts/coordinate=")
  assert_eq(kv("settings.read.acts/coordinate"), "", "the value is emptied")
  assert_eq(kv("settings.read_actor.acts/coordinate"), "", "the side key dies with it")

  -- ② 换人重签 = 侧键 upsert（不是两个签名叠加）。
  as_actor("other-agent")
  fire("config.set", "read.acts/coordinate=9f2c1a")
  assert_eq(kv("settings.read_actor.acts/coordinate"), "other-agent", "upsert, not a second entry")
  as_actor("test-agent")

  -- ③ 作用域边界（rev 推进 → req_bump_rev 销票）：两个键一起清。
  fire("requirement.add", "side-key requirement|synthetic description|semantics-side")
  fire("requirement.dispatch", "0")
  fire("requirement.amend", "r1||amended to bump the revision")
  assert_eq(kv("settings.read.acts/coordinate"), "", "the boundary wipes the value")
  assert_eq(kv("settings.read_actor.acts/coordinate"), "", "and the side key")
  assert_eq(kv("settings.read_index"), "", "and the index")
end)
