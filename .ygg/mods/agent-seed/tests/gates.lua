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
