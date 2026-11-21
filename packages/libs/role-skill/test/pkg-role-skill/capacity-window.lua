-- capacity-window：用例段自 pkg-role-skill.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, eq_seq, is_true, sorted_keys, expect_error, codec, new_world, fresh, events_named, record_of, pkg_sources, source_text =
    C.S, C.run, C.eq, C.eq_seq, C.is_true, C.sorted_keys, C.expect_error, C.codec, C.new_world, C.fresh, C.events_named, C.record_of, C.pkg_sources, C.source_text
run("pkg.role-skill.static-no-clock", function()
  local paths = pkg_sources("role-skill")
  is_true(#paths >= 7, "manifest 里 role-skill 源文件 ≥ 7，实得 " .. #paths)
  local needles = {
    "ygg.ticks",
    "os.clock",
    "os.time",
    "math.random",
    "set_attribute",
    "entity.set(",
  }
  local problems = {}
  for i = 1, #paths do
    local text = source_text(paths[i])
    for j = 1, #needles do
      if text:find(needles[j], 1, true) ~= nil then
        problems[#problems + 1] = paths[i] .. " 含 " .. needles[j]
      end
    end
  end
  is_true(#problems == 0, table.concat(problems, "; "))
end)

-- ── 用例 15：100 轮授予/撤销不增长（索引只增，技能集回零）────────────────────

run("pkg.role-skill.leak-100-rounds", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  is_true(role.grant(1, "warrior"), "grant")
  for i = 1, 6 do
    role.skill_define("skill" .. i, { attrs_spec = { attack = i } })
  end

  local index_before = #codec.decode(world.kv["role.index"])
  local keys_before = #sorted_keys(world.kv)
  collectgarbage("collect")
  collectgarbage("collect")
  local before = collectgarbage("count")

  for _ = 1, 100 do
    for i = 1, 6 do
      role.grant_skill(1, "skill" .. i)
    end
    for i = 1, 6 do
      role.revoke_skill(1, "skill" .. i)
    end
    world.clear_events()
  end

  collectgarbage("collect")
  collectgarbage("collect")
  local growth = (collectgarbage("count") - before) / before * 100
  print(string.format("perf.role-skill.leak_delta_pct=%.3f", growth))

  eq_seq(sorted_keys(record_of(world, 1).skills), {}, "技能集回零")
  eq(#codec.decode(world.kv["role.index"]), index_before, "索引条目数不变（只增不摘）")
  eq(#sorted_keys(world.kv), keys_before, "KV 键数不变")
  is_true(growth <= 1, "100 轮后内存差 ≤ 1%，实得 " .. string.format("%.3f", growth) .. "%")
end)

-- ── 用例 16：技能集读面（升序名字数组 + 副本语义 + 空集边界）──────────────────

run("pkg.role-skill.read-skills-of", function()
  local world = new_world()
  local role = fresh(world)

  eq_seq(role.skills_of(1), {}, "无记录 ⇒ 空数组（不是 nil）")
  role.skill_define("zeta", { attrs_spec = {} })
  role.skill_define("alpha", { attrs_spec = {} })
  role.skill_define("mid", { attrs_spec = {} })
  role.define("warrior", { attrs_spec = {} })
  role.grant(1, "warrior")
  eq_seq(role.skills_of(1), {}, "有记录但无槽位 ⇒ 仍是空数组")

  -- 授予顺序与名字顺序故意相反：结果是升序，与装配顺序无关
  role.grant_skill(1, "zeta")
  role.grant_skill(1, "mid")
  role.grant_skill(1, "alpha")
  eq_seq(role.skills_of(1), { "alpha", "mid", "zeta" }, "升序名字数组")

  -- 副本语义：读面不能被调用方当成旁路写面
  local snapshot = role.skills_of(1)
  snapshot[1] = "tampered"
  snapshot[#snapshot + 1] = "extra"
  eq_seq(role.skills_of(1), { "alpha", "mid", "zeta" }, "改返回值不影响存储")

  role.revoke_skill(1, "mid")
  eq_seq(role.skills_of(1), { "alpha", "zeta" }, "撤销后从集合消失")

  -- `rank = 0` 的槽位也在：槽位存在与否是写面的事实，筛等级是 mod 的规则（I7）
  role.skill_define("zero", { attrs_spec = {} })
  role.grant_skill(1, "zero", 0)
  eq_seq(role.skills_of(1), { "alpha", "zero", "zeta" }, "rank=0 的槽位也在")
end)

-- ── 用例 17：单槽读面（字段只映槽位 + nil 边界 + 副本）─────────────────────────

run("pkg.role-skill.read-skill-state", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("fire", { attrs_spec = {}, charges = 7 })
  role.skill_define("plain", { attrs_spec = {} })
  role.grant_skill(1, "fire", 3)
  role.grant_skill(1, "plain")

  local fire = role.skill_state(1, "fire")
  eq(fire.rank, 3, "rank 与授予值相等")
  eq(fire.charges, 7, "声明了 charges ⇒ 槽位带 charges")
  eq(fire.capacity, nil, "未设容量 ⇒ 没有 capacity 字段")
  eq(role.skill_state(1, "plain").charges, nil, "未声明 charges ⇒ 没有 charges 字段")
  eq(role.skill_state(1, "nope"), nil, "无槽位 ⇒ nil（不是空表）")
  eq(role.skill_state(99, "fire"), nil, "无记录 ⇒ nil")
  expect_error(role.ERRORS.DEF, function()
    role.skill_state(1, "")
  end, "空技能名 ⇒ -DEF")

  local copy = role.skill_state(1, "fire")
  copy.rank = 99
  copy.charges = 0
  eq(role.skill_state(1, "fire").rank, 3, "改副本的 rank 不回写")
  eq(role.skill_state(1, "fire").charges, 7, "改副本的 charges 不回写")

  eq(role.set_capacity(1, "fire", 12), 12, "设容量")
  eq(role.skill_state(1, "fire").capacity, 12, "设容量后带 capacity")
  eq(role.skill_state(1, "fire").charges, 7, "放大不补满：余量仍是 7")
end)

-- ── 用例 18：实例容量放大（声明的 charges 不再是天花板）───────────────────────

run("pkg.role-skill.capacity-grow", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role.grant_skill(1, "pp")
  eq(role.skill_state(1, "pp").charges, 30, "建槽位时充满到声明的 30")

  eq(role.set_capacity(1, "pp", 60), 60, "set_capacity 返回生效后的容量")
  eq(role.skill_state(1, "pp").charges, 30, "放大不补满")
  eq(role.recharge(1, "pp"), 60, "recharge 充满到新上限")

  local left = nil
  for _ = 1, 60 do
    left = role.charge(1, "pp")
  end
  eq(left, 0, "连调 60 次才到 0（声明的 30 不再是天花板）")
  eq(role.charge(1, "pp"), 0, "0 不减负")
end)

-- ── 用例 19：实例容量缩容（余量夹紧 + 上限换成新值）───────────────────────────

run("pkg.role-skill.capacity-shrink", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role.grant_skill(1, "pp")

  eq(role.set_capacity(1, "pp", 10), 10, "缩容到 10")
  eq(role.skill_state(1, "pp").charges, 10, "余量夹紧到 10")
  for _ = 1, 10 do
    role.charge(1, "pp")
  end
  eq(role.charge(1, "pp"), 0, "打光后仍是 0（不减负）")
  eq(role.skill_state(1, "pp").charges, 0, "余量 0")
  eq(role.recharge(1, "pp"), 10, "回到新上限（不是声明的 30）")
end)

-- ── 用例 20：容量面的错误码与「未声明 charges」技能的下场 ──────────────────────

run("pkg.role-skill.capacity-errors", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("bare", { attrs_spec = {} })
  role.grant_skill(1, "bare")

  eq(role.charge(1, "bare"), 0, "未声明 charges 且无容量 ⇒ 不参与充能")
  eq(role.recharge(1, "bare"), 0, "同上（recharge 返回 0 且不写）")
  expect_error(role.ERRORS.UNKNOWN, function()
    role.set_capacity(1, "nope", 5)
  end, "无槽位 ⇒ -UNKNOWN")
  expect_error(role.ERRORS.UNKNOWN, function()
    role.set_capacity(99, "bare", 5)
  end, "无记录 ⇒ -UNKNOWN")
  expect_error(role.ERRORS.DEF, function()
    role.set_capacity(1, "bare", -1)
  end, "负 cap ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.set_capacity(1, "bare", 1.5)
  end, "小数 cap ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.set_capacity(1, "bare", "3")
  end, "字符串 cap ⇒ -DEF")

  eq(role.set_capacity(1, "bare", 3), 3, "未声明 charges 的技能也能设容量")
  eq(role.skill_state(1, "bare").charges, 3, "余量以 cap 初始化（与建槽位时充满同口径）")
  eq(role.charge(1, "bare"), 2, "设了容量就开始参与充能（D8 只在两者皆无时生效）")
end)

-- ── 用例 21：recharge 第 4 参（持久化容量；3 参路径逐字节同前）─────────────────

run("pkg.role-skill.capacity-recharge-cap", function()
  local world = new_world()
  local role = fresh(world)
  role.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role.grant_skill(1, "pp")

  -- 3 参路径：与本轮前的语义逐值相等，且不写 capacity 字段
  eq(role.recharge(1, "pp"), 30, "3 参：填到声明的 30")
  is_true(
    world.kv["role.actor.1"]:find("capacity", 1, true) == nil,
    "3 参路径的载荷不含 capacity 字段"
  )
  eq(role.recharge(1, "pp", 5), 30, "已满再充仍夹紧在 30")
  role.charge(1, "pp")
  eq(role.recharge(1, "pp", 2), 30, "n=2 充回上限")

  -- 第 4 参 = 持久化新容量（不是「临时上限」——那会造出第二个真源）
  eq(role.recharge(1, "pp", 1, 40), 31, "4 参：余量 30+1，容量同时换成 40")
  eq(role.skill_state(1, "pp").capacity, 40, "4 参把容量落进槽位")
  is_true(
    world.kv["role.actor.1"]:find("capacity", 1, true) ~= nil,
    "载荷里确实出现 capacity"
  )
  eq(role.recharge(1, "pp"), 40, "之后无参 recharge 以新容量为上限")

  -- 对照世界：只走 3 参 ⇒ 没有容量、上限仍是声明值
  local world2 = new_world()
  local role2 = fresh(world2)
  role2.skill_define("pp", { attrs_spec = {}, charges = 30 })
  role2.grant_skill(1, "pp")
  eq(role2.recharge(1, "pp"), 30, "对照世界 3 参仍是 30")
  eq(role2.skill_state(1, "pp").capacity, nil, "对照世界没有 capacity")
end)

-- ── 用例 22：窗口推入与计数（含 n=0、无记录、没推过的标签）────────────────────

run("pkg.role-skill.window-push-read", function()
  local world = new_world()
  local role = fresh(world)

  eq(role.window(1, "a", 5), 0, "无记录 ⇒ 0")
  eq(role.window_push(1, "a"), 1, "push 返回推入后的 ring 长度")
  role.window_push(1, "b")
  role.window_push(1, "a")
  role.window_push(1, "a")
  eq(role.window_push(1, "b"), 5, "五条都在界内")
  -- ring = a b a a b
  eq(role.window(1, "a", 5), 3, "最近 5 条里 a 出现 3 次")
  eq(role.window(1, "b", 5), 2, "b 出现 2 次")
  eq(role.window(1, "c", 5), 0, "没推过的标签 ⇒ 0")
  eq(role.window(1, "a", 0), 0, "n=0 ⇒ 0")
  eq(role.window(1, "a", 3), 2, "只看最近 3 条（a a b）")
  eq(role.window(1, "a", 2), 1, "最近 2 条是 a b")
  eq(role.window(1, "b", 1), 1, "最近 1 条是 b")
  eq(role.window(2, "a", 5), 0, "另一个 actor 的流是空的")
  is_true(type(world.kv["role.window.1"]) == "string", "窗口载荷是 codec 字符串")
end)

-- ── 用例 23：窗口回绕（挤掉最旧、长度恒为 cap、载荷体积）──────────────────────

run("pkg.role-skill.window-ring-wrap", function()
  local world = new_world()
  local role = fresh(world)
  local cap = role.WINDOW_CAP
  eq(cap, 100, "缺省容量 100")

  eq(role.window_push(1, "spell.z"), 1, "第一条")
  for _ = 2, cap do
    role.window_push(1, "spell.a")
  end
  eq(role.window(1, "spell.z", cap), 1, "满载时最旧的 z 还在")

  eq(role.window_push(1, "spell.a"), cap, "第 cap+1 条：长度恒为 cap")
  eq(role.window(1, "spell.z", cap), 0, "最旧的 z 被挤掉")
  eq(role.window(1, "spell.a", cap), cap, "回绕后整条 ring 都是 a")

  local raw = world.kv["role.window.1"]
  is_true(#raw <= 1024, "100 条载荷 ≤ 1 KiB，实得 " .. #raw .. " B")
end)

-- ── 用例 24：窗口的 n 边界（n > cap 报错、n 超史合法、参数校验）────────────────

run("pkg.role-skill.window-n-over-cap", function()
  local world = new_world()
  local role = fresh(world)
  local cap = role.WINDOW_CAP
  role.window_push(1, "a")

  expect_error(role.ERRORS.DEF, function()
    role.window(1, "a", cap + 1)
  end, "n > cap ⇒ -DEF（不静默截断）")
  eq(role.window(1, "a", cap), 1, "n == cap 通过：超史但 ≤ cap ⇒ 统计现有条目")
  expect_error(role.ERRORS.DEF, function()
    role.window(1, "a", -1)
  end, "负 n ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.window(1, "a", 2.5)
  end, "小数 n ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.window(1, "a", "3")
  end, "字符串 n ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.window(1, "", 3)
  end, "空标签 ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.window_push(1, "")
  end, "push 空标签 ⇒ -DEF")
  expect_error(role.ERRORS.DEF, function()
    role.window_push(1, 7)
  end, "push 非字符串 ⇒ -DEF")

  -- 安装期容量：小容量也能回绕；n 的判据跟着走
  local world2 = new_world()
  local role2 = fresh(world2, nil, 3)
  eq(role2.WINDOW_CAP, 3, "install({ window_cap = 3 }) 生效")
  role2.window_push(1, "a")
  role2.window_push(1, "a")
  role2.window_push(1, "a")
  role2.window_push(1, "b")
  eq(role2.window_push(1, "b"), 3, "cap=3 回绕后长度恒为 3")
  eq(role2.window(1, "b", 3), 2, "最近 3 条 = a b b")
  expect_error(role2.ERRORS.DEF, function()
    role2.window(1, "a", 4)
  end, "n=4 > cap=3 ⇒ -DEF")
  expect_error(role2.ERRORS.INSTALL, function()
    role2.install({ codec = codec, entity = world2.entity, host = world2.host, window_cap = 0 })
  end, "window_cap 非正整数 ⇒ -INSTALL")
end)

-- ── 用例 25：窗口与计数器互不干涉（I5 / I10 的隔离）───────────────────────────

run("pkg.role-skill.window-vs-counters", function()
  local world = new_world()
  local role = fresh(world, { turn = "entity" })
  role.skill_define("a", { attrs_spec = {} })
  role.grant_skill(1, "a")
  for _ = 1, 3 do
    world.handlers.turn({ entity = 1 })
  end
  eq(role.count(1, "turn"), 3, "三次 turn ⇒ 3")

  role.window_push(1, "a")
  role.window_push(1, "a")
  eq(role.count(1, "turn"), 3, "window_push 不动 counters（I10）")
  eq(role.window(1, "a", 10), 2, "窗口计到 2")

  role.reset_counters(1, "turn")
  eq(role.count(1, "turn"), 0, "清零生效")
  eq(role.window(1, "a", 10), 2, "reset_counters 不动窗口（I10）")
end)

-- ── 用例 26：兼容面台账（19 导出 / 7 错误码 / 4 事件 / 4 KV 键名）───────────────

end
