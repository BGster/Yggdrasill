-- fixtures：开闸 / 裸拷贝副本上的用例（只在对应的 `--fixture <tag>` 里执行）。

test("fixture 基建：armed 副本真的上膛", function()
  local cfg = __ASSETS["ygg:agent/config"]
  assert_eq(cfg.readGate.ver["acts/coordinate"], "aaaaaa", "读签台账已上膛")
  assert_eq(#cfg.readGate.reqActs.plan, 1, "plan 覆盖清单")
  assert_eq(cfg.readGate.reqActs.plan[1], "req-acts/intake", "plan 覆盖文档")
  assert_eq(#cfg.readGate.gates.accept, 1, "accept 门禁覆盖")
  assert_eq(#cfg.readPlan.coordinate, 1, "readPlan 已改写")
end, "armed")

test("fixture 基建：bare 副本原样（台账仍是真实哈希）", function()
  local cfg = __ASSETS["ygg:agent/config"]
  -- 原版 config.json 本身就是开闸状态（10 条真实哈希台账）；`bare` = 一行不改的副本，
  -- 所以它的台账必须仍是真实哈希，不能是 `armed` 的那组固定值。
  assert_true(cfg.readGate ~= nil and cfg.readGate.ver ~= nil, "原版必须带 readGate.ver")
  local coord = cfg.readGate.ver["acts/coordinate"]
  assert_true(coord ~= nil, "原版台账必须覆盖 acts/coordinate")
  assert_true(coord ~= "aaaaaa", "裸拷贝不得出现 armed 的固定台账值")
end, "bare")

--- armed 副本的起跑线：立局 + 一条需求 + 派发（等价于 Rust 的 `armed_game`）。
local function armed_game()
  fresh()
  charter_and_begin()
  fire("requirement.add", "armed requirement|armed description|armed-r1")
  fire("requirement.dispatch", "0")
end

test("读签缺签：提交被拒且零副作用，拒台词自带命令/窗口/隔派发数", function()
  armed_game()

  reset()
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")
  local joined = table.concat(narrated(), "\n")
  assert_true(narrated_contains("read-gate/missing"), "拒绝必须点名维度：" .. joined)
  assert_true(
    joined:find("config.set read.acts/coordinate=aaaaaa", 1, true) ~= nil,
    "每个缺签文档必须带确切的签名命令：" .. joined
  )
  assert_true(
    joined:find("config.set read.req-acts/intake=bbbbbb", 1, true) ~= nil,
    "第二件缺签文档同样带命令：" .. joined
  )
  assert_true(joined:find("window 80", 1, true) ~= nil, "必须报告窗口值：" .. joined)
  assert_true(joined:find("dispatch(es) ago", 1, true) ~= nil, "必须报告已隔派发数：" .. joined)

  -- 零副作用：门禁没升起、提案没落库、签名账本没动。
  assert_eq(kv("req.1.gate"), "", "门禁没升起")
  assert_eq(kv("req.1.job"), "plan", "job 槽仍是 plan")
  assert_nil(kv("prop.1.status"), "提案没落库")
  assert_nil(kv("settings.read_index"), "签名账本没动")
end, "armed")

test("读签纪元：阶段边界之前取的签名会作废（stale）", function()
  armed_game()
  sign("acts/coordinate", "aaaaaa")
  sign("req-acts/intake", "bbbbbb")
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")
  assert_eq(kv("req.1.gate"), "plan", "提案抬高了 plan 门")

  -- plan 批准把需求推进到 development 阶段：纪元前进。
  sign("req-acts/delivery", "cccccc")
  click("gate.approve — ")
  assert_eq(kv("req.1.plan_ok"), true, "plan 批准生效")

  reset()
  fire("proposal.submit", "r1|1|development|armed dev|artifacts/r1-dev.md")
  local joined = table.concat(narrated(), "\n")
  assert_true(narrated_contains("read-gate/stale"), "拒绝必须点名 stale 维度：" .. joined)
  assert_true(
    joined:find("stage development began at", 1, true) ~= nil,
    "必须点名作废它的那个阶段边界：" .. joined
  )
  assert_true(
    joined:find("was signed at dispatch", 1, true) ~= nil,
    "必须报告签名的时序（在第几个派发签的）：" .. joined
  )
  assert_true(joined:find("Window 80", 1, true) ~= nil, "必须报告窗口：" .. joined)
end, "armed")

test("覆盖面 k/n：只统计本幕覆盖了几件，与「我签了几件」两个口径分开", function()
  fresh()
  charter_and_begin()

  -- ① 只签计划的第 1 件 → 本幕 1/2，其中「我签的」1 件。
  reset()
  sign("acts/coordinate", "9f2c1a")
  assert_eq(
    count_marker("this act has 1/2 docs signed, 1 of them by you"),
    1,
    "覆盖面 k 只数本幕：" .. table.concat(narrated(), " | ")
  )

  -- ② 另一个代理签下第 2 件 → 覆盖面升到 2/2，但「我签的」仍是 1 件。
  as_actor("other-agent")
  reset()
  sign("coordinate/dispatch", "c0ffee")
  assert_eq(
    count_marker("this act has 2/2 docs signed, 1 of them by you"),
    1,
    "覆盖面与署名数必须分得开：" .. table.concat(narrated(), " | ")
  )
  assert_eq(kv("settings.read_actor.coordinate/dispatch"), "other-agent", "署名按人记账")

  -- ③ 计划外的 doc：签名照旧成立，但不进分子（不拒签、不越界）。
  as_actor("selftest")
  reset()
  sign("acts/charter", "aaaaaa")
  assert_eq(
    count_marker("this act has 2/2 docs signed, 1 of them by you"),
    1,
    "计划外文档不进分子：" .. table.concat(narrated(), " | ")
  )
end, "plan2")

test("读签遗忘：窗口调到 0 时签名立刻过期，抬高窗口即解除", function()
  armed_game()
  fire("config.set", "readGate.window=0")
  sign("acts/coordinate", "aaaaaa")
  sign("req-acts/intake", "bbbbbb")

  reset()
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")
  local joined = table.concat(narrated(), "\n")
  assert_true(narrated_contains("read-gate/forgotten"), "必须点名 forgotten 维度：" .. joined)
  assert_true(joined:find("window 0", 1, true) ~= nil, "必须报告**配置的**窗口值：" .. joined)
  assert_true(joined:find("dispatch(es) ago", 1, true) ~= nil, "必须报告已隔派发数：" .. joined)
  assert_true(
    joined:find("config.set readGate.window=", 1, true) ~= nil,
    "必须给出解除阻塞的确切命令：" .. joined
  )

  -- 解除路径：抬高窗口 → 同一个动作通过。
  fire("config.set", "readGate.window=80")
  reset()
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")
  assert_true(
    not narrated_contains("[REJECTED"),
    "抬高窗口必须解除阻塞：" .. table.concat(narrated(), " | ")
  )
  assert_eq(kv("req.1.gate"), "plan", "提案抬高 plan 门")
end, "armed")

test("读签通过：签齐后提交放行，序号是 KV 标量可复算", function()
  armed_game()
  sign("acts/coordinate", "aaaaaa")
  sign("req-acts/intake", "bbbbbb")

  reset()
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")
  assert_true(
    not narrated_contains("[REJECTED"),
    "签齐后必须放行：" .. table.concat(narrated(), " | ")
  )
  assert_eq(kv("req.1.gate"), "plan", "提案抬高 plan 门")

  -- 序号是 KV 标量（读档可复算）：签名快照非零、派发计数单调。
  local sign_seq = kv("settings.sign_seq.acts/coordinate")
  assert_true(
    type(sign_seq) == "number" and sign_seq > 0,
    "mark_sign 必须记下真实的派发序号，实得：" .. tostring(sign_seq)
  )
  local clock = kv("settings.clock_seq")
  assert_true(
    type(clock) == "number" and clock > 0,
    "派发计数器必须住在 KV 里，实得：" .. tostring(clock)
  )
end, "armed")

test("读签他人署名：拒绝点名署名人与当前执行者（不是 missing）", function()
  armed_game()

  -- 另一个代理把本幕必读签齐（真路径上它就是当时的执行者）。
  as_actor("r6-planner")
  sign("acts/coordinate", "aaaaaa")
  sign("req-acts/intake", "bbbbbb")
  assert_eq(kv("settings.read_actor.acts/coordinate"), "r6-planner", "署名按人记账")

  -- 换另一个代理推进：签名有效（ver 对、序号新），但不是它签的。
  as_actor("main-agent")
  reset()
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")
  local joined = table.concat(narrated(), "\n")
  assert_true(
    narrated_contains("read-gate/other-actor"),
    "他人署名必须报 other-actor，不能塌成 missing：" .. joined
  )
  assert_true(joined:find("by r6-planner", 1, true) ~= nil, "必须点名署名者：" .. joined)
  assert_true(
    joined:find("dispatched as main-agent", 1, true) ~= nil,
    "必须点名当前执行者：" .. joined
  )
  assert_true(
    joined:find("0 of them by you", 1, true) ~= nil,
    "覆盖面与「我签的」两个数必须分开：" .. joined
  )

  -- 拒绝零副作用：提案不落库、门禁不挂。
  assert_nil(kv("prop.1.req"), "提案不落库")
  assert_eq(kv("req.1.gate"), "", "门禁没挂上")
  as_actor("selftest")
end, "armed")

test("读签不掩盖既有拒绝：amend 后的 stale 门禁理由优先", function()
  armed_game()
  sign("acts/coordinate", "aaaaaa")
  sign("req-acts/intake", "bbbbbb")
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")

  -- 待批期间 amend → gate_rev 与 rev 失配（既有 stale 判据）。零签名也在场，但先报既有原因。
  fire("requirement.amend", "r1||amended while pending")
  reset()
  fire("gate.approve", "r1")
  local joined = table.concat(narrated(), "\n")
  assert_true(
    not joined:find("read-gate/", 1, true),
    "读签理由不得抢占既有拒绝：" .. joined
  )
  assert_true(
    joined:find("is bound to rev", 1, true) ~= nil or narrated_contains("[REJECTED"),
    "既有的门禁理由必须先报：" .. joined
  )
end, "armed")

--- armed 副本的 plan 门起跑线：签齐三件 + plan 提案受理（等价 Rust 的 `armed_plan_gate`）。
local function armed_plan_gate(actor)
  armed_game()
  as_actor(actor)
  sign("acts/coordinate", "aaaaaa")
  sign("req-acts/intake", "bbbbbb")
  sign("req-acts/delivery", "cccccc")
  reset()
  fire("proposal.submit", "r1|1|plan|armed plan|artifacts/r1.md")
  assert_true(
    not narrated_contains("[REJECTED"),
    "plan 提案必须被受理：" .. table.concat(narrated(), " | ")
  )
  assert_eq(phase_of("r1"), "plan_gate", "停在 plan 门")
end

test("读签无身份：直派 gate.approve 被拒，且给出自我解除路径", function()
  armed_plan_gate("r6-planner")
  fire("config.set", "play.mode=goal")
  as_actor(nil) -- 真无身份

  reset()
  fire("gate.approve", "r1")
  local joined = table.concat(narrated(), "\n")
  assert_true(
    narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
    "无身份直派必须被拒：" .. joined
  )
  assert_true(narrated_contains("read-gate/no-identity"), "必须点名 no-identity 维度：" .. joined)
  assert_true(joined:find("YGG_ACTOR", 1, true) ~= nil, "必须给出身份开关：" .. joined)
  assert_true(joined:find("config.set read.", 1, true) ~= nil, "必须给出签名命令：" .. joined)

  -- 状态不动：门禁仍挂、计划未过。
  assert_eq(kv("req.1.gate"), "plan", "门禁仍挂")
  assert_eq(kv("req.1.plan_ok"), false, "计划未过")
  as_actor("selftest")
end, "armed")

test("读签票据降级：菜单路径只核覆盖率，并明说不区分署名", function()
  -- 另一个代理把该签的都签了——现场就是「签字的人不是点菜单的人」。
  armed_plan_gate("gate-ticket")
  as_actor(nil)

  click("gate.approve — ")
  local joined = table.concat(narrated(), "\n")
  assert_true(
    joined:find("[READ GATE · ticket]", 1, true) ~= nil,
    "票据路径必须声明自己的降级语义：" .. joined
  )
  assert_true(
    joined:find("signers are NOT distinguished", 1, true) ~= nil,
    "降级必须写明白，不能暗示：" .. joined
  )
  assert_true(not joined:find("read-gate/", 1, true), "票据路径不走拒绝分支：" .. joined)

  -- 降级不是放水：覆盖率仍然被核（这里已签齐），所以门禁真的过了。
  assert_eq(kv("req.1.plan_ok"), true, "计划通过")
  assert_eq(kv("req.1.gate"), "", "门禁清空")
  as_actor("selftest")
end, "armed")
