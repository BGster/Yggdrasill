-- lifecycle：立局与六幕的骨架断言（迁移自 Rust 对照套件第 1–5、25 例）。
-- 迁移期只搬「不需要复杂前置状态」的骨架断言；完整分支随各域迁入时补齐。

test("event.start：开局落在 charter 微状态", function()
  -- 必须走 fresh()：用例共享一个 Lua 状态，前一个文件可能已经把局推进过。
  fresh()
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

test("click 链路：章程→开工走完，run.phase 归零", function()
  fresh()
  click("charter.set — ")
  assert_eq(kv("task.title"), "任务标题", "charter.set must write the default trio")
  click("run.begin — ")
  assert_eq(kv("run.phase"), "", "run.begin leaves the charter micro-state")
end)

test("章程门：开局在 charter、菜单诚实、三件套可修订、run.begin 锁定", function()
  fresh()
  assert_eq(kv("run.phase"), "charter", "boot lands in the charter micro-state")

  local options = render_options()
  assert_true(
    options[1]:sub(1, #"charter.set — ") == "charter.set — ",
    "charter.set must be the first charter item"
  )
  for _, text in ipairs(options) do
    assert_true(
      text:sub(1, #"run.begin — ") ~= "run.begin — ",
      "an incomplete charter must not put run.begin on the menu"
    )
  end

  -- run.begin 前置：三件套齐备。可读拒绝，相位不动。
  reset()
  fire("run.begin", "0")
  assert_true(narrated_contains("[REJECTED"), "run.begin must be refused before the trio")
  assert_eq(kv("run.phase"), "charter", "a refused run.begin must not move the phase")

  -- charter.set 写三件套，且立局内可反复修订（后写覆盖）。
  reset()
  fire("charter.set", "semantics run|prove the v2 state machine|suite green")
  assert_eq(kv("task.title"), "semantics run", "title")
  assert_eq(kv("task.goal"), "prove the v2 state machine", "goal")
  assert_eq(kv("task.success"), "suite green", "success")

  local found = false
  for _, text in ipairs(render_options()) do
    if text:sub(1, #"run.begin — ") == "run.begin — " then
      found = true
    end
  end
  assert_true(found, "a complete charter must put run.begin on the menu")

  fire("charter.set", "semantics run v2|prove the loop|suite green")
  assert_eq(kv("task.title"), "semantics run v2", "last write wins")

  -- run.begin → 协调幕：run.phase = ""（常驻主态）。
  reset()
  fire("run.begin", "0")
  assert_eq(kv("run.phase"), "", "run.begin clears the charter micro-state")
  assert_true(narrated_contains("[RUN BEGUN]"), "it must narrate the open")
end)
