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
