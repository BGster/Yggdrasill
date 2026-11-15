-- settlement：盘点四项与批边界（迁移自 Rust 对照套件 13–18、79、84–90 例）。

test("退役路径不追加维护需求：武装的 review.note 写 ending 且不动登记簿", function()
  drive_to_release()
  fire("release.skip", "纯测试批次，无 develop→master 合并")
  fire("config.set", "run.retire=true")
  assert_eq(kv("settings.run.retire"), "true", "the arm flag lands in settings")

  reset()
  click("review.note — ")
  assert_true(narrated_contains("[ENDING · run retired]"), "an armed review.note retires the run")
  assert_true(not narrated_contains("[MAINTENANCE REQ"), "the retired path must not append a row")
  assert_true(not narrated_contains("[BATCH"), "a retiring close carries no batch-settle line")
  assert_eq(kv("req.count"), 1, "one requirement in the run")
  assert_nil(kv("run.maintenance.last"), "the retired path leaves the register untouched")
  assert_eq(kv("ending"), "retired", "the ending is written")
end)
