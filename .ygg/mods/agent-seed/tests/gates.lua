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
