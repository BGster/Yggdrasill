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

test("verify 失败：回协调幕、失败计数 +1、结算槽与发布声明清空", function()
  drive_to_settle()
  reset()
  click("verify.submit all_green=false — ")
  assert_true(narrated_contains("[GATES FAILED]"), "the honest failure must narrate its beat")
  assert_eq(kv("run.phase"), "", "the batch falls back to coordinate")
  assert_nil(kv("fsm.stage"), "live is the FSM initial state: not materialized")
  assert_eq(kv("verify.passed"), false, "verify is not passed")
  assert_eq(kv("verify.failures"), 1, "the failure counter advances")
  assert_eq(kv("settlement.evolution.summary"), "", "the settlement slot is cleared")
  assert_eq(kv("run.release.master_sha"), "", "the release declaration is cleared")

  -- 回协调幕后可继续接活。
  fire("requirement.add", "rework requirement|desc|semantics-rework")
  assert_eq(kv("req.2.status"), "open", "the queue accepts new work for the next round")
end)
