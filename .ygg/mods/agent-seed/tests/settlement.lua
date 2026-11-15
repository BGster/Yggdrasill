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

test("发布声明门：未声明不给 review.note；坏形状零写入；声明后批结算（skip + confirm 两路）", function()
  drive_to_release()

  -- ① 未声明：菜单有 confirm / skip，但没有 review.note。
  local has = {}
  for _, text in ipairs(render_options()) do
    for _, prefix in ipairs({ "release.confirm — ", "release.skip — ", "review.note — " }) do
      if text:sub(1, #prefix) == prefix then
        has[prefix] = true
      end
    end
  end
  assert_true(has["release.confirm — "] and has["release.skip — "], "confirm/skip are offered")
  assert_true(not has["review.note — "], "an undeclared release withholds review.note")

  -- ② 坏形状：拒绝且零写入。
  reset()
  fire("release.skip", "")
  assert_true(narrated_contains("[REJECTED"), "an empty payload is not a reason")
  reset()
  fire("release.confirm", "master@zzzz|v2026.11.12.1")
  assert_true(narrated_contains("[REJECTED"), "a non-hex sha is refused")
  reset()
  fire("release.confirm", "master@abc1234|not-a-tag")
  assert_true(narrated_contains("[REJECTED"), "a malformed tag is refused")
  assert_nil(kv("run.release.master_sha"), "refusals write nothing")
  assert_nil(kv("run.release.skipped_reason"), "refusals write nothing")

  -- ③ 带理由 skip → 已声明 → review.note 领跑 → 批 1 结算回协调。
  reset()
  fire("release.skip", "本批无需发布：纯测试批次")
  assert_true(narrated_contains("[RELEASE SKIPPED]"), "skip narrates its beat")
  assert_eq(kv("run.release.skipped_reason"), "本批无需发布：纯测试批次", "the reason lands")
  assert_true(
    render_options()[1]:sub(1, #"review.note — ") == "review.note — ",
    "a declared release leads with the batch closer"
  )
  reset()
  click("review.note — ")
  assert_true(narrated_contains("[BATCH 1 SETTLED]"), "the close narrates its batch line")
  assert_eq(kv("run.phase"), "", "back to coordinate")
  assert_eq(kv("run.batch"), 2, "the batch counter advances")

  -- ④ 批 2：维护义务必须先解除，否则 coordinate.submit 被硬阻塞。
  reset()
  fire("config.set", "docs.maintenance.done=1")
  assert_true(narrated_contains("[DOCS MAINTENANCE CLEARED]"), "the obligation is cleared")
  fire("requirement.add", "second batch requirement|desc|semantics-b2")
  fire("requirement.dispatch", "0")
  submit_req("r2", 1, "plan", "semantics-b2-p1")
  click("gate.approve — ")
  submit_req("r2", 1, "development", "semantics-b2-p2")
  click("proposal.accept — ")
  click("gate.approve — ")
  fire("integration.confirm", "r2|develop@beef124")
  click("coordinate.submit — ")
  click("verify.submit all_green=true — ")
  fire("settlement.evolution", "|batch two screened|c=0/0|sessions=0/0/none|todo=0/cleared")
  assert_eq(kv("run.phase"), "release", "batch 2 reaches the release act")

  -- ⑤ confirm 路径：写 sha 与 tag，再结算批 2。
  reset()
  fire("release.confirm", "master@1a2b3c4|v2026.11.12.2")
  assert_true(narrated_contains("[RELEASE CONFIRMED]"), "confirm narrates its beat")
  assert_eq(kv("run.release.master_sha"), "1a2b3c4", "the merge sha lands")
  assert_eq(kv("run.release.tag"), "v2026.11.12.2", "the batch tag lands")
  reset()
  click("review.note — ")
  assert_true(narrated_contains("[BATCH 2 SETTLED]"), "the batch-2 close narrates its line")
end)

test("批结算登记维护义务并硬阻塞下一次批边界（解除后放行）", function()
  drive_to_release()
  fire("release.skip", "纯测试批次，无 develop→master 合并")

  reset()
  click("review.note — ")
  assert_true(narrated_contains("[BATCH 1 SETTLED]"), "review.note settles the batch")
  assert_true(narrated_contains("[DOCS MAINTENANCE DUE]"), "the settle registers the obligation")
  assert_true(not narrated_contains("[ENDING"), "a batch settle never carries the ending prefix")
  assert_eq(kv("run.batch"), 2, "the batch counter advances")
  assert_eq(kv("run.phase"), "", "back to coordinate")
  assert_eq(kv("docs.maintenance.due"), "1", "the obligation is pinned to batch 1")
  assert_eq(kv("req.count"), 1, "maintenance derives no requirement row")

  -- 义务未解除 → 批边界被拒，并给出解除命令。
  reset()
  click("coordinate.submit — ")
  assert_true(narrated_contains("docs-maintenance/due"), "the boundary names the blocking reason")
  assert_true(narrated_contains("docs.maintenance.done=1"), "and prints the clearing command")
  assert_eq(kv("run.phase"), "", "the phase does not move while blocked")

  -- 解除 → 放行。
  reset()
  fire("config.set", "docs.maintenance.done=1")
  assert_true(narrated_contains("[DOCS MAINTENANCE CLEARED]"), "the obligation is cleared")
  assert_eq(kv("docs.maintenance.due"), "", "the obligation empties")
  click("coordinate.submit — ")
  assert_eq(kv("run.phase"), "settle", "the boundary is released")
end)

test("维护义务只认等值批次：旧批次号与未知键都被拒", function()
  drive_to_release()
  fire("release.skip", "纯测试批次")
  click("review.note — ")
  assert_eq(kv("docs.maintenance.due"), "1", "the settle pins the obligation to batch 1")

  reset()
  fire("config.set", "docs.maintenance.done=0")
  assert_true(narrated_contains("[REJECTED"), "a stale batch number cannot clear a newer obligation")
  assert_eq(kv("docs.maintenance.due"), "1", "the obligation survives the stale clear")

  reset()
  fire("config.set", "docs.maintenance.other=1")
  assert_true(narrated_contains("[REJECTED"), "an unknown docs key is refused")
  assert_eq(kv("docs.maintenance.due"), "1", "the obligation survives the unknown key")

  reset()
  fire("config.set", "docs.maintenance.done=1")
  assert_true(narrated_contains("[DOCS MAINTENANCE CLEARED]"), "the equal batch clears it")
  assert_eq(kv("docs.maintenance.due"), "", "the obligation empties")
end)

test("一次事务完成四项：evolution + sessions + todo，同回合 [SETTLE READY]", function()
  drive_to_settle()

  reset()
  click("verify.submit all_green=true — ")
  assert_true(narrated_contains("[GATES GREEN]"), "verify records the evidence")

  -- 畸形可选段：拒绝，且不写任何东西（moved 停在 reset 基线 0）。
  reset()
  fire("settlement.evolution", "|summary|c=0/0|sessions=1/3/nowhere")
  assert_true(narrated_contains("sessions="), "the refusal names the bad segment")
  assert_eq(kv("settlement.sessions.moved"), 0, "the refusal writes nothing")

  -- 合并回合：一条载荷同时结清 evolution / sessions / todo。
  reset()
  fire(
    "settlement.evolution",
    "|candidates screened against the evolution chapter|c=0/0|" ..
      "sessions=1/0/~/.pi/agent/sessions-trash/semantics|todo=0/re-checked the todo list"
  )
  assert_true(narrated_contains("[SETTLE READY]"), "the four-settled effect fires in the same turn")
  assert_eq(kv("settlement.sessions.destination"), "~/.pi/agent/sessions-trash/semantics", "destination")
  assert_eq(kv("settlement.sessions.moved"), 1, "moved")
  assert_eq(kv("settlement.sessions.remaining"), 0, "remaining")
  assert_eq(kv("settlement.todo.remaining"), 0, "todo remaining")
  assert_eq(kv("settlement.todo.verified_by"), "re-checked the todo list", "todo provenance")
  assert_eq(
    kv("settlement.evolution.summary"),
    "candidates screened against the evolution chapter",
    "the evolution summary lands"
  )
  assert_eq(kv("run.phase"), "release", "four settled → the release act")
end)

test("集成确认：SHA 形状门 / 只作用于 integrating 行 / reject 回 ready / done 只读", function()
  drive_to_dev_pending("semantics-p2")
  drive_to_accept()
  drive_to_integrating()

  -- ① 证据形状：7–40 位 hex（可带 develop@ 前缀）。
  for _, evidence in ipairs({ "develop@zzzz", "develop@abc", "abc", "develop@" }) do
    reset()
    fire("integration.confirm", "r1|" .. evidence)
    assert_true(narrated_contains("[REJECTED"), "bad evidence must be refused: " .. evidence)
    assert_eq(kv("req.1.status"), "open", "status stays open")
    assert_eq(kv("req.1.integrating"), true, "the row is still integrating")
  end

  -- ② 只作用于 integrating 行：r2 还是 draft。
  fire("requirement.add", "bystander requirement|desc|semantics-bystander")
  reset()
  fire("integration.confirm", "r2|develop@abc1234")
  assert_true(narrated_contains("[REJECTED"), "a non-integrating row is refused")

  -- ③ confirm：done + sha 证据 + 泳道释放。
  reset()
  fire("integration.confirm", "r1|develop@abc1234")
  assert_true(narrated_contains("[INTEGRATION CONFIRMED]"), "confirm narrates its beat")
  assert_eq(kv("req.1.status"), "done", "the true state moves")
  assert_eq(kv("req.1.develop_sha"), "abc1234", "the sha evidence is kept")
  assert_eq(kv("req.1.lane"), 0, "the lane is released")
  assert_eq(kv("req.1.closed_reason"), "", "no bypass reason")

  -- ④ reject 路径：r2 走到 integrating 后 reject → ready。
  fire("requirement.dispatch", "0")
  submit_req("r2", 1, "plan", "semantics-bystander-p1")
  click("gate.approve — ")
  submit_req("r2", 1, "development", "semantics-bystander-p2")
  click("proposal.accept — ")
  click("gate.approve — ")
  assert_eq(kv("req.2.lane"), 1, "r2 binds its lane")
  reset()
  fire("integration.reject", "r2|rework the diff")
  assert_true(narrated_contains("[INTEGRATION REJECTED]"), "reject narrates its beat")
  assert_eq(kv("req.2.status"), "open", "reject keeps the true state open")
  assert_eq(kv("req.2.integrating"), false, "the integrating flag clears")
  assert_eq(kv("req.2.plan_ok"), true, "plan_ok survives an integration reject")
  assert_eq(kv("req.2.lane"), 0, "the lane is released")
  assert_eq(phase_of("r2"), "ready", "the row is back in ready")

  -- ⑤ done 行只读：再 confirm 必须被拒。
  reset()
  fire("integration.confirm", "r1|develop@abc1234")
  assert_true(narrated_contains("[REJECTED"), "a done row only accepts read-back")
end)
