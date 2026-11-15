-- gates：两道人工门（迁移自 Rust 对照套件 6–11、19–20、22 例）。

test("计划门 reject：回 planning、plan_ok=false、job 重挂、提案可再交", function()
  drive_to_plan_gate()
  assert_eq(phase_of("r1"), "plan_gate", "the plan gate is the current phase")

  click("gate.reject — ")
  assert_eq(kv("req.1.gate"), "", "the gate slot is cleared")
  assert_eq(kv("req.1.plan_ok"), false, "a plan reject keeps plan_ok false")
  assert_eq(kv("req.1.job"), "plan", "the plan job returns without a dispatch turn")
  assert_eq(phase_of("r1"), "planning", "the row is back in planning")

  -- 门位空出后可再次交付 plan 提案。
  submit(1, "plan", "semantics-p1-replan")
  assert_eq(kv("req.1.gate"), "plan", "a second plan proposal raises the gate again")
  assert_eq(kv("req.1.gate_rev"), 1, "gate_rev follows the revision")
end)

test("旁路关闭：split/merge 写 closed_reason、被旁路行不可 amend、done amend 派生", function()
  fresh()
  charter_and_begin()

  -- 一条开发前草稿（无 job 在飞，可 split）。
  fire("requirement.add", "split me|synthetic description|semantics-split")

  -- ① split：父变 done + closed_reason=split。
  reset()
  fire("requirement.split", "r1|first child|second child")
  assert_true(narrated_contains("[REQ SPLIT]"), "split narrates its beat")
  assert_eq(kv("req.1.status"), "done", "the parent is done")
  assert_eq(kv("req.1.closed_reason"), "split", "closed with a reason")
  assert_eq(phase_of("r1"), "split", "the display name is split")

  -- ② 被旁路的行不可 amend。
  reset()
  fire("requirement.amend", "r1|reopen attempt")
  -- 注意：这条拒绝走 `[BLOCKED:TODO]`（前置未满足），不是 `[REJECTED:*]`。
  assert_true(
    narrated_contains("[BLOCKED") or narrated_contains("[REJECTED"),
    "a bypassed row is not amendable"
  )

  -- ③ merge 两个 open 行：源变 done + merged，新行出现。
  fire("requirement.merge", "r2,r3|merged requirement|merged desc")
  assert_eq(kv("req.2.status"), "done", "source 1 done")
  assert_eq(kv("req.2.closed_reason"), "merged", "source 1 merged")
  assert_eq(kv("req.3.status"), "done", "source 2 done")
  assert_eq(kv("req.3.closed_reason"), "merged", "source 2 merged")
  assert_eq(phase_of("r4"), "draft", "the merged row is a fresh draft")

  -- ④ 走完 r4，再 amend 这条 done 行 → 派生后继。
  fire("requirement.dispatch", "r4")
  submit_req("r4", 1, "plan", "semantics-derive-p1")
  click("gate.approve — ")
  submit_req("r4", 1, "development", "semantics-derive-p2")
  click("proposal.accept — ")
  click("gate.approve — ")
  fire("integration.confirm", "r4|develop@feed777")
  assert_eq(kv("req.4.status"), "done", "r4 is done")
  assert_eq(kv("req.4.closed_reason"), "", "no bypass reason")

  reset()
  fire("requirement.amend", "r4|follow-up title")
  assert_true(narrated_contains("[REQ DERIVED]"), "amending a done row derives a successor")
  assert_eq(kv("req.4.status"), "done", "the original stays done")
  assert_eq(kv("req.5.origin"), "r4", "the successor records its origin")
  assert_eq(phase_of("r5"), "draft", "the successor is a draft")
end)

test("门禁独立性：别人的待决提案与在跑 job 不阻塞本条门的推进", function()
  drive_to_plan_gate()

  -- r2：加 → 派发 → 抬高自己的 plan gate。
  fire("requirement.add", "second requirement|synthetic description|semantics-r2")
  fire("requirement.dispatch", "0")
  submit_req("r2", 1, "plan", "semantics-r2-p1")
  assert_eq(kv("prop.2.status"), "pending", "r2's proposal is pending")
  assert_eq(kv("req.2.job"), "", "r2's job slot is free")
  assert_eq(kv("req.2.gate"), "plan", "r2 raises its own plan gate")

  -- r3：加 → 派发（plan job 在跑，尚无提案）。
  fire("requirement.add", "third requirement|synthetic description|semantics-r3")
  fire("requirement.dispatch", "0")
  assert_eq(kv("req.3.job"), "plan", "r3's plan job is running")

  -- 批准 r1：同回合自动派发 r1 的 development。
  reset()
  click("gate.approve — ")
  assert_true(narrated_contains("[AUTO-DISPATCH]"), "approving r1 auto-dispatches development")
  assert_eq(kv("req.1.plan_ok"), true, "r1 plan_ok")
  assert_eq(kv("req.1.job"), "development", "r1 development job starts")
  assert_eq(kv("req.1.lane"), 1, "r1 binds a lane")

  -- 邻居不受影响。
  assert_eq(kv("prop.2.status"), "pending", "r2's proposal stays pending")
  assert_eq(kv("req.2.gate"), "plan", "r2 keeps its own gate")
  assert_eq(phase_of("r2"), "plan_gate", "r2 is still on its plan gate")
  assert_eq(kv("req.3.job"), "plan", "r3's job keeps running")
end)

test("验收门 reject：保住 plan_ok、释放泳道、回 ready 并重新派发 development", function()
  drive_to_dev_pending("semantics-p2")
  drive_to_accept()
  assert_eq(kv("req.1.status"), "open", "the true state stays open")
  assert_eq(kv("req.1.gate_rev"), 1, "gate_rev pins the revision")

  -- guide 门：pass-type 动作没有菜单票据时一律被拒，状态不动（单发直派路）。
  reset()
  fire_direct("gate.approve", "r1")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "a direct approve without a ticket must be refused"
  )
  assert_eq(kv("req.1.gate"), "accept", "the gate stays up")
  assert_eq(kv("req.1.status"), "open", "the state does not move")

  -- 正常 reject：reject 不受票据门限制。
  reset()
  click("gate.reject — ")
  assert_eq(kv("req.1.gate"), "", "the gate slot clears")
  assert_eq(kv("req.1.plan_ok"), true, "an accept reject keeps plan_ok")
  assert_eq(kv("req.1.lane"), 0, "the lane is released")
  assert_eq(phase_of("r1"), "ready", "the row is back in ready")

  fire("requirement.dispatch", "0")
  assert_eq(kv("req.1.job"), "development", "development is re-dispatched")
  assert_eq(kv("req.1.lane"), 1, "a lane is bound again")
end)

test("验收门 amend：rev+1 / plan_ok 失效 / gate 绑定旧 rev / 旧批准必须 stale", function()
  drive_to_dev_pending("semantics-p2")
  drive_to_accept()
  assert_eq(phase_of("r1"), "accept_gate", "the row sits on the accept gate")

  fire("requirement.amend", "r1|amended title")
  assert_eq(kv("req.1.rev"), 2, "amend bumps the revision")
  assert_eq(kv("req.1.plan_ok"), false, "plan_ok is invalidated")
  assert_eq(kv("req.1.gate"), "accept", "the gate itself is not taken down")
  assert_eq(kv("req.1.gate_rev"), 1, "but it stays bound to the old revision")

  -- 旧批准不得生效：stale 拒绝，状态与门位都不动。
  reset()
  click("gate.approve — ")
  assert_true(narrated_contains("stale"), "an approval bound to the old rev must be refused as stale")
  assert_eq(kv("req.1.status"), "open", "status untouched")
  assert_eq(kv("req.1.gate"), "accept", "the gate stays up")
  assert_eq(kv("req.1.gate_rev"), 1, "gate_rev untouched")

  reset()
  click("gate.reject — ")
  assert_eq(kv("req.1.gate"), "", "reject clears the gate")
  assert_eq(kv("req.1.plan_ok"), false, "plan_ok stays false after the amend")
  assert_eq(phase_of("r1"), "draft", "the row falls back to draft")

  fire("requirement.dispatch", "0")
  assert_eq(kv("req.1.job"), "plan", "a plan job is re-hung")
end)

test("开发提案 reject：未 amend 保 plan_ok；amend 后落 draft", function()
  -- 路径 1：未 amend —— plan_ok 保住，回到 ready 可重新派发。
  drive_to_dev_pending("semantics-p3")
  fire("proposal.reject", "r1|incomplete artifacts")
  assert_eq(kv("req.1.status"), "open", "status stays open")
  assert_eq(kv("req.1.plan_ok"), true, "plan_ok survives a plain dev reject")
  assert_eq(kv("req.1.lane"), 0, "the lane is released")
  assert_eq(phase_of("r1"), "ready", "back to ready")
  fire("requirement.dispatch", "0")
  assert_eq(kv("req.1.job"), "development", "development is re-dispatched")
  assert_eq(kv("req.1.lane"), 1, "a lane is bound again")

  -- 路径 2：先 amend（rev 推进使 plan_ok 失效），再 reject —— 落 draft，需要新计划。
  drive_to_dev_pending("semantics-p4")
  fire("requirement.amend", "r1|add an acceptance criterion")
  assert_eq(kv("req.1.rev"), 2, "amend bumps the revision")
  assert_eq(kv("req.1.plan_ok"), false, "amend invalidates plan_ok")
  fire("proposal.reject", "r1|redo on the new rev")
  assert_eq(kv("req.1.status"), "open", "status stays open")
  assert_eq(kv("req.1.plan_ok"), false, "plan_ok stays false")
  assert_eq(phase_of("r1"), "draft", "an amended reject falls to draft")
  fire("requirement.dispatch", "0")
  assert_eq(kv("req.1.job"), "plan", "a plan job is re-hung")
end)
