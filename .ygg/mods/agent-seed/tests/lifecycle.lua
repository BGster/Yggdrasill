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

test("未完成批次时退休不可达：走完选择序列也不出现 [ENDING", function()
  fresh()
  -- 固定走局序列（等价 playtest 的 choices）。
  for _, idx in ipairs({ 3, 4, 5, 1, 2, 1 }) do
    click_index(idx)
  end
  -- 必须查**行首**：`run.retire` 的说明文案里会引用 `[ENDING · run retired]` 这句话本身，
  -- 用 anywhere 匹配会把文案引用误当成真结束行。
  for _, line in ipairs(narrated()) do
    assert_true(
      line:sub(1, #"[ENDING") ~= "[ENDING",
      "未完成的局不得出现结束行：" .. line
    )
  end
end)

test("run.retire 值域与退役收口：子页装配 / 退休结束行 / 清除", function()
  drive_to_release()

  -- ① 值域：除 `true` / `""` 之外一律拒，且不落键。
  reset()
  fire("config.set", "run.retire=false")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "非法值必须被拒：" .. table.concat(narrated(), " | ")
  )
  assert_nil(kv("settings.run.retire"), "不得落键")

  -- ② 先声明发布（任何 review.note 的前置）。
  fire("release.skip", "纯测试批次，无 develop→master 合并")

  -- ③ 经由子页装配：纯展示入口，写入仍是 config.set。
  click("config — ")
  local sub = render_options()
  assert_eq(#sub, 5, "配置子页必须是 5 项")
  assert_true(
    sub[4]:sub(1, #"config.set — run.retire") == "config.set — run.retire",
    "子页第 4 项必须是 run.retire：" .. sub[4]
  )
  click("config.set — run.retire")
  assert_eq(kv("settings.run.retire"), "true", "已上膛")
  assert_eq(#render_options(), 5, "点完仍停在配置子页")
  click("back — ")

  -- ④ 退休收口。
  reset()
  click("review.note — ")
  local joined = table.concat(narrated(), "\n")
  assert_true(
    joined:find("[ENDING · run retired]", 1, true) ~= nil,
    "上膛后的 review.note 必须退休：" .. joined
  )
  assert_true(joined:find("[BATCH", 1, true) == nil, "退休收口永不带批次结算行：" .. joined)
  assert_eq(kv("ending"), "retired", "ending 键")
  assert_eq(kv("fsm.stage"), "retired", "FSM 阶段")
  assert_nil(kv("run.batch"), "退休路径不写 run.batch")
  assert_eq(
    kv("review.evolution.summary"),
    "candidates screened against the evolution chapter",
    "evolution 摘要被归档"
  )

  -- ⑤ 退休后仍可清除，并叙述自己的 beat。
  reset()
  fire("config.set", "run.retire=")
  assert_eq(count_marker("[RUN RETIRE CLEARED]"), 1, "清除必须叙述")
  assert_eq(kv("settings.run.retire"), "", "键置为空串")
end)

test("批次循环回到协调幕：空队列永不关闭、永不记绿、永不结算", function()
  fresh()
  -- 固定走局序列（等价 playtest 的 choices）：boot → charter.set → run.begin → 两次被拒的收口。
  for _, idx in ipairs({ 1, 2, 1, 1 }) do
    click_index(idx)
  end

  for _, line in ipairs(narrated()) do
    assert_true(
      line:sub(1, #"[COORDINATION DONE]") ~= "[COORDINATION DONE]",
      "空队列不得关闭协调幕：" .. line
    )
    assert_true(
      line:sub(1, #"[GATES GREEN]") ~= "[GATES GREEN]",
      "空队列不得登记全绿：" .. line
    )
    assert_true(line:sub(1, #"[BATCH") ~= "[BATCH", "空队列不得结算批次：" .. line)
  end
end)
