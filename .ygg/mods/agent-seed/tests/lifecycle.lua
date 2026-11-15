-- lifecycle：立局与六幕的骨架断言（迁移自 Rust 对照套件第 1–5、25 例）。
-- 迁移期只搬「不需要复杂前置状态」的骨架断言；完整分支随各域迁入时补齐。

test("event.start：开局落在 charter 微状态", function()
  fire("event.start", "0")
  assert_eq(kv("run.phase"), "charter", "boot lands in the charter micro-state")
end)

test("run.begin：章程不全时必须可读拒绝且不推进", function()
  fresh()
  reset()
  fire("run.begin", "0")
  assert_eq(kv("run.phase"), "charter", "an incomplete charter must not move the phase")
  assert_true(narrated_contains("[REJECTED"), "the refusal must be narrated, not silent")
end)

test("未知事件：不注册的 handler 必须不存在（fail-closed 的外部形态）", function()
  assert_eq(handler("no.such.event"), nil, "unknown events must have no handler")
end)
