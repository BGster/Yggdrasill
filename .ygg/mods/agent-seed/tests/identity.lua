-- identity：身份与执行者台账（迁移自 Rust 对照套件 27–31、52–53 例）。

test("profile 卡：注册 / upsert / 多键 / 空值清除 / 坏键拒绝", function()
  fresh()
  start_requirement()

  fire("config.set", "profile.agent-1.harness=pi")
  assert_eq(kv("settings.profile.agent-1.harness"), "pi", "the field lands")
  assert_eq(kv("settings.profile_index"), "agent-1", "the index registers the handle")

  -- upsert：同键重写覆盖。
  fire("config.set", "profile.agent-1.harness=codex")
  assert_eq(kv("settings.profile.agent-1.harness"), "codex", "upsert overwrites the field")

  -- 其余保留键 + 自由键。
  fire("config.set", "profile.agent-1.session_id=01a1c961-test")
  fire("config.set", "profile.agent-1.role=plan-executor")
  fire("config.set", "profile.agent-1.assignments=r1:plan,r1:development,r3")
  fire("config.set", "profile.agent-1.registered_by=main")
  assert_eq(kv("settings.profile.agent-1.session_id"), "01a1c961-test", "session_id")
  assert_eq(kv("settings.profile.agent-1.role"), "plan-executor", "role")
  assert_eq(kv("settings.profile.agent-1.assignments"), "r1:plan,r1:development,r3", "assignments")
  assert_eq(kv("settings.profile.agent-1.registered_by"), "main", "registered_by")
  assert_eq(kv("settings.profile_index"), "agent-1", "the index does not duplicate")

  -- `settings.` 前缀写法等价。
  fire("config.set", "settings.profile.agent-1.role=dev")
  assert_eq(kv("settings.profile.agent-1.role"), "dev", "the settings. prefix is equivalent")

  -- 空值 = 逐字段清除。
  fire("config.set", "profile.agent-1.role=")
  assert_eq(kv("settings.profile.agent-1.role"), "", "an empty value clears the field")

  -- 键形坏（缺 <key> 段）→ parse 层拒绝且不落 KV。
  reset()
  fire("config.set", "profile.agent-1=x")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "a malformed profile key is refused"
  )
  assert_nil(kv("settings.profile.agent-1"), "no stray key is written")

  -- 真未知键 → fail-closed。
  reset()
  fire("config.set", "profile_index=agent-x")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "an alien key is refused"
  )
  assert_eq(kv("settings.profile_index"), "agent-1", "the index is untouched")
end)

test("actor 通道 verified：三段身份落到提案与台账", function()
  fresh()
  start_requirement()

  submit_with_identity(1, "plan", "id-verified", "agent-1", "agent-1", "s-plan-1")
  assert_eq(kv("prop.1.stage"), "plan", "the proposal lands")
  assert_eq(kv("prop.1.actor"), "agent-1", "the actor stamp lands on the proposal")
  assert_eq(kv("prop.1.actor_session"), "s-plan-1", "the session stamp lands")
  assert_eq(kv("settings.executor.r1.plan"), "agent-1", "the executor ledger binds in the same call")

  -- 注：原用例后半段是「save → restore 后三段仍存活」，属 driver 层的 save/receipt
  -- 缺口（testing-strategy.md §3 第 3 条），纯 Lua 自检无 restore 语义，故不迁。
end)

test("actor 三态之一：mismatch 只记录不拒，且 plan 提案不进验收菜单", function()
  fresh()
  start_requirement()

  submit_with_identity(1, "plan", "id-mismatch", "agent-1", "agent-2", nil)
  assert_eq(kv("prop.1.actor"), "agent-2", "the host stamp wins the kv slot")
  assert_eq(kv("settings.executor.r1.plan"), "agent-1", "the exec ledger keeps the exec id")
  assert_eq(phase_of("r1"), "plan_gate", "a mismatched channel still raises the gate")

  for _, item in ipairs(render_options()) do
    assert_true(
      item:sub(1, #"proposal.accept — ") ~= "proposal.accept — ",
      "a plan proposal must not be on the acceptance menu: " .. item
    )
  end

  reset()
  fire("proposal.accept", "p1")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "a direct proposal.accept is refused"
  )
  assert_eq(kv("req.1.gate"), "plan", "the gate is untouched")
end)

test("尾链顺序：重复 exec / exec 错位都拒；全链解析并落六键", function()
  fresh()
  start_requirement()

  -- 重复 exec= → 拒绝，零副作用。
  reset()
  fire("proposal.submit", "r1|1|plan|s|artifacts/p.md||x|exec=agent-1|exec=agent-2")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "a duplicated exec= is refused"
  )
  assert_nil(kv("prop.1.req"), "no proposal is created")
  assert_nil(kv("settings.executor.r1.plan"), "no ledger row")

  -- exec= 错位（不在末段）→ 拒绝。
  reset()
  fire("proposal.submit", "r1|1|plan|s|artifacts/p.md|exec=agent-1||x")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "a misplaced exec= is refused"
  )
  assert_nil(kv("prop.1.req"), "no proposal is created")

  -- 全链 exec → actor → actor_session：解析成功并落六键。
  as_actor("agent-1")
  ensure_reads("proposal.submit", "plan", 1)
  reset()
  fire(
    "proposal.submit",
    "r1|1|plan|full-chain summary|artifacts/plan.md|ch=1|id-full|exec=agent-1|actor=agent-1|actor_session=s-full"
  )
  assert_true(not narrated_contains("REJECTED"), "a full chain must parse: " .. table.concat(narrated(), " | "))
  assert_eq(kv("prop.1.stage"), "plan", "stage")
  assert_eq(kv("prop.1.summary"), "full-chain summary", "summary")
  assert_eq(kv("prop.1.changes"), "ch=1", "changes")
  assert_eq(kv("prop.1.ext"), "id-full", "ext")
  assert_eq(kv("prop.1.actor"), "agent-1", "actor")
  assert_eq(kv("prop.1.actor_session"), "s-full", "actor_session")
  assert_eq(kv("settings.executor.r1.plan"), "agent-1", "the ledger binds exec")
  as_actor("selftest")
end)

test("无身份（缺章）：提交被拒、签名被拒；恢复身份后正常", function()
  fresh()
  start_requirement()

  -- ① 真无身份：提案被 read-gate 拒，零副作用。
  as_actor(nil)
  submit_with_identity(1, "plan", "id-unv", "agent-1", nil, nil)
  assert_nil(kv("prop.1.req"), "no proposal without an identity stamp")
  assert_nil(kv("settings.executor.r1.plan"), "no ledger row either")

  -- ② 无身份也签不了名。
  fire("config.set", "read.acts/coordinate=aaaaaa")
  assert_true(narrated_contains("[SIGNED REFUSED]"), "a signature without an actor is refused")
  assert_nil(kv("settings.read.acts/coordinate"), "nothing is written")

  -- ③ 恢复身份后一切照常。
  as_actor("selftest")
  submit_with_identity(1, "plan", "id-plan", "agent-1", "selftest", nil)
  fire("proposal.reject", "r1|replan without identity")
  assert_eq(kv("settings.executor.r1.plan"), "agent-1", "the ledger keeps the exec binding")
  assert_eq(kv("req.1.gate"), "", "the gate is cleared by the reject")
  assert_eq(kv("req.1.job"), "plan", "the job slot is re-hung")

  -- ④ 只有 actor 章、没有 exec：章落 kv，但不建台账（不冒充绑定）。
  fire("requirement.add", "actor-only requirement|synthetic description|id-r2")
  fire("requirement.dispatch", "r2")
  submit_identity_for("r2", 1, "plan", "id-actor-only", nil, "agent-9", nil)
  assert_eq(kv("prop.2.actor"), "agent-9", "an actor-only stamp still lands")
  assert_nil(kv("settings.executor.r2.plan"), "but no ledger row is invented")
end)
