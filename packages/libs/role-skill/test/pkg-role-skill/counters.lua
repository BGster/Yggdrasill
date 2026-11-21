-- counters：用例段自 pkg-role-skill.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, eq_seq, is_true, sorted_keys, expect_error, codec, new_world, fresh, events_named, record_of, pkg_sources, source_text =
    C.S, C.run, C.eq, C.eq_seq, C.is_true, C.sorted_keys, C.expect_error, C.codec, C.new_world, C.fresh, C.events_named, C.record_of, C.pkg_sources, C.source_text
run("pkg.role-skill.kv-string-shape", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  is_true(role.grant(1, "warrior"), "grant 成功")

  local raw = world.kv["role.actor.1"]
  eq(type(raw), "string", "记录的载荷必须是字符串（KV 只收标量）")
  local payload = record_of(world, 1)
  eq_seq(sorted_keys(payload), { "counters", "role", "skills" }, "记录键集恰好三个")
  eq(payload.role, "warrior", "role 字段")
  eq_seq(sorted_keys(payload.skills), {}, "新记录没有技能槽位")
  eq_seq(sorted_keys(payload.counters), {}, "新记录没有计数")

  local index = codec.decode(world.kv["role.index"])
  eq(type(index), "table", "索引载荷也是字符串（解码后是表）")
  eq(index[1], 1, "索引第 1 项是 id 1")
  eq(#index, 1, "索引长度 1")
end)

-- ── 用例 2：授予幂等（I3）─────────────────────────────────────────────────────

run("pkg.role-skill.grant-idempotent", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", {})

  is_true(role.grant(1, "warrior"), "首次 grant")
  local writes = world.writes_now()
  is_true(role.grant(1, "warrior"), "再次 grant 仍返回 true（返回的是状态）")
  eq(world.writes_now(), writes, "幂等路径零写")
  eq(#events_named(world, "role.granted"), 1, "role.granted 只发一次")
  eq(#codec.decode(world.kv["role.index"]), 1, "索引只插入一次")
end)

-- ── 用例 3：撤销语义（未持有 false / 技能集不动）──────────────────────────────

run("pkg.role-skill.revoke-absent-false", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", {})

  eq(role.revoke(1, "warrior"), false, "未持有时 revoke 返回 false")
  eq(#world.events, 0, "未持有 ⇒ 零事件")

  is_true(role.grant(1, "warrior"), "grant")
  is_true(role.revoke(1, "warrior"), "持有时 revoke 返回 true")
  eq(role.has(1, "warrior"), false, "撤销后 has 为 false")
  eq(#events_named(world, "role.revoked"), 1, "role.revoked 发过一次")
  eq(role.revoke(1, "warrior"), false, "再撤销返回 false（记录保留、只清引用面）")
  is_true(record_of(world, 1) ~= nil, "记录未被删除（KV 无删除面）")
end)

-- ── 用例 4：未定义的角色 ─────────────────────────────────────────────────────

run("pkg.role-skill.grant-unknown-role-errors", function()
  local world = new_world()
  local role = fresh(world)
  expect_error("YGG-PKG-ROLE-UNKNOWN", function()
    role.grant(1, "nobody")
  end, "未定义角色")
  expect_error("YGG-PKG-ROLE-DEF", function()
    role.define("", {})
  end, "空角色名")
end)

-- ── 用例 5：引用面闸门（I7：只看 tag，不写不发）───────────────────────────────

run("pkg.role-skill.requires-tags-gate", function()
  local world = new_world()
  local role = fresh(world)
  role.define("mage", { requires_tags = { "awakened" } })

  local before = sorted_keys(world.kv)
  eq(role.grant(2, "mage"), false, "tag 不满足 ⇒ false")
  eq_seq(sorted_keys(world.kv), before, "被拒时 KV 键集合不变")
  eq(#world.events, 0, "被拒时零事件")

  world.add_tag(2, "awakened")
  is_true(role.grant(2, "mage"), "满足后可授予")
  eq(role.has(2, "mage"), true, "has 反映状态")
end)

-- ── 用例 6：引用建立即事件（升级不是建立）────────────────────────────────────

run("pkg.role-skill.skill-gain-once", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("fireball", { attrs_spec = { magic = 5 }, charges = 3 })

  is_true(role.grant_skill(1, "fireball"), "首次 grant_skill")
  eq(#events_named(world, "role.skill_gained"), 1, "首次发一次事件")
  local slot = record_of(world, 1).skills.fireball
  eq(slot.rank, 1, "缺省 rank = 1")
  eq(slot.charges, 3, "按声明充满")

  is_true(role.grant_skill(1, "fireball", 4), "带新 rank 再授一次")
  eq(#events_named(world, "role.skill_gained"), 1, "事件仍 1 次（升级不是建立）")
  eq(record_of(world, 1).skills.fireball.rank, 4, "rank 已更新")

  is_true(role.grant_skill(1, "fireball"), "缺省 rank 再授")
  eq(record_of(world, 1).skills.fireball.rank, 4, "缺省保持原值")
end)

-- ── 用例 7：PP 语义（零不减负 / 无声明不参与）──────────────────────────────────

run("pkg.role-skill.charge-floor", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("flash", { charges = 2 })
  role.grant_skill(1, "flash")

  eq(role.charge(1, "flash"), 1, "2 → 1")
  eq(role.charge(1, "flash"), 0, "1 → 0")
  eq(role.charge(1, "flash"), 0, "0 不再减负")

  role.skill_define("passive", {})
  role.grant_skill(1, "passive")
  local writes = world.writes_now()
  eq(role.charge(1, "passive"), 0, "未声明 charges ⇒ 0")
  eq(world.writes_now(), writes, "未声明 charges ⇒ 不写")
  eq(role.charge(1, "nope"), 0, "无槽位 ⇒ 0")
end)

-- ── 用例 8：充能上限取声明值 ─────────────────────────────────────────────────

run("pkg.role-skill.recharge-cap", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("flash", { charges = 3 })
  role.grant_skill(1, "flash")

  role.charge(1, "flash")
  eq(role.recharge(1, "flash"), 3, "无参 ⇒ 充满")
  role.charge(1, "flash")
  role.charge(1, "flash")
  role.charge(1, "flash")
  eq(role.recharge(1, "flash", 1), 1, "给 1 就加 1")
  eq(role.recharge(1, "flash", 99), 3, "夹到声明上限")

  role.skill_define("passive", {})
  role.grant_skill(1, "passive")
  eq(role.recharge(1, "passive", 5), 0, "未声明 charges ⇒ 0")
end)

-- ── 用例 9：合成而非写回（I2，本包最重要的一条）──────────────────────────────

run("pkg.role-skill.compose-no-writeback", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  role.skill_define("power", { attrs_spec = { attack = 2 } })
  role.skill_define("fury", { attrs_spec = { attack = 4 } })
  world.set_attr(1, "attack", 10)

  is_true(role.grant(1, "warrior"), "grant")
  is_true(role.grant_skill(1, "power"), "grant_skill power")
  is_true(role.grant_skill(1, "fury"), "grant_skill fury")

  eq(role.effective(1, "attack"), 19, "10 + 3 + 2 + 4")
  eq(world.entity.get(1, "attack"), 10, "基础值未被写回（I2）")

  is_true(role.revoke_skill(1, "fury"), "卸下 fury")
  eq(role.effective(1, "attack"), 15, "10 + 3 + 2（卸下即还原，无需撤销历史）")
  eq(world.entity.get(1, "attack"), 10, "基础值仍未变")
end)

-- ── 用例 10：自定义合成规则收到的加成顺序 ─────────────────────────────────────

run("pkg.role-skill.compose-custom-order", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  role.skill_define("zeta", { attrs_spec = { attack = 7 } })
  role.skill_define("alpha", { attrs_spec = { attack = 5 } })
  role.skill_define("mid", { attrs_spec = { attack = 6 } })
  world.set_attr(1, "attack", 10)

  role.grant(1, "warrior")
  -- 授予顺序刻意打乱：合成顺序由**名字**决定，不由授予顺序（否则两个游戏的同一套
  -- 技能会因装配顺序不同而算出不同加成）。
  role.grant_skill(1, "zeta")
  role.grant_skill(1, "alpha")
  role.grant_skill(1, "mid")

  local seen = nil
  is_true(role.compose(function(base, bonuses)
    local parts = { tostring(base) }
    for i = 1, #bonuses do
      parts[#parts + 1] = tostring(bonuses[i])
    end
    seen = table.concat(parts, ",")
    return base
  end), "compose 装自定义规则")

  eq(role.effective(1, "attack"), 10, "自定义规则原样返回 base")
  eq(seen, "10,3,5,6,7", "顺序 = base、角色加成、技能名升序（alpha 5 / mid 6 / zeta 7）")
  expect_error("YGG-PKG-ROLE-COMPOSE", function()
    role.compose(42)
  end, "非函数合成规则")
end)

-- ── 用例 11：属性缺失记 0（不是 nil，不报错）──────────────────────────────────

run("pkg.role-skill.effective-missing-attr-zero", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  role.skill_define("power", { attrs_spec = { magic = 5 } })
  role.skill_define("fury", { attrs_spec = { magic = 2 } })
  role.grant(1, "warrior")
  role.grant_skill(1, "power")
  role.grant_skill(1, "fury")

  eq(world.entity.get(1, "magic"), nil, "布置：实体上没有 magic")
  eq(role.effective(1, "magic"), 7, "缺失属性 base 记 0，加成照加（5 + 2）")
  eq(role.effective(1, "luck"), 0, "无基础无加成 ⇒ 0")
  eq(role.effective(99, "attack"), 0, "无记录的实体 ⇒ 0")
end)

-- ── 用例 12：计数器只增（订阅面 + 选择器两种形态）────────────────────────────

run("pkg.role-skill.counters-increment", function()
  local world = new_world()
  local role = fresh(world, {
    tick = function(payload)
      return payload.who
    end,
    turn = "entity",
  })
  role.define("warrior", {})
  role.grant(1, "warrior")
  role.grant(2, "warrior")

  is_true(type(world.handlers.turn) == "function", "turn 已订阅")
  is_true(type(world.handlers.tick) == "function", "tick 已订阅")

  for _ = 1, 3 do
    world.handlers.turn({ entity = 1 })
  end
  eq(role.count(1, "turn"), 3, "三次 turn ⇒ 3")
  eq(role.count(2, "turn"), 0, "实体 2 未被计数")
  eq(role.count(1, "never"), 0, "未订阅的事件名 ⇒ 0")

  -- 解析不出 id（payload 里没有那个字段）⇒ 不写
  local writes = world.writes_now()
  world.handlers.turn({ other = 1 })
  eq(world.writes_now(), writes, "id 为 nil ⇒ 不写")
  eq(role.count(1, "turn"), 3, "计数不变")

  -- 函数选择器读的是 payload 里的自定义字段
  world.handlers.tick({ who = 2 })
  eq(role.count(2, "tick"), 1, "函数选择器解析出的 id")

  -- 没有记录的实体：统计事件不凭空建记录
  world.handlers.turn({ entity = 99 })
  eq(world.kv["role.actor.99"], nil, "没有记录就不建记录")
end)

-- ── 用例 13：显式清零（带 event / 不带 event）────────────────────────────────

run("pkg.role-skill.counters-reset", function()
  local world = new_world()
  local role = fresh(world, { turn = "entity", tick = "entity" })
  role.define("warrior", {})
  role.grant(1, "warrior")
  world.handlers.turn({ entity = 1 })
  world.handlers.turn({ entity = 1 })
  world.handlers.tick({ entity = 1 })

  is_true(role.reset_counters(1, "turn"), "带 event 清零")
  eq(role.count(1, "turn"), 0, "turn 归零")
  eq(role.count(1, "tick"), 1, "别的计数不动")

  -- 已经是 0：返回 true 且不写
  local writes = world.writes_now()
  is_true(role.reset_counters(1, "turn"), "零值再清仍 true")
  eq(world.writes_now(), writes, "零值再清不写")

  is_true(role.reset_counters(1), "不带 event 清全部")
  eq(role.count(1, "tick"), 0, "tick 也归零")
  eq(#sorted_keys(record_of(world, 1).counters), 0, "counters 清空")

  -- 无记录：视为已清零，不写
  local before = sorted_keys(world.kv)
  is_true(role.reset_counters(99), "无记录返回 true")
  eq_seq(sorted_keys(world.kv), before, "无记录不写")
end)

-- ── 用例 14：静态面（零时间源、零属性写回面）──────────────────────────────────

--- 包的源文件清单：来自 manifest 的 packages 段（内容寻址，天然排除 test/**）。


end
