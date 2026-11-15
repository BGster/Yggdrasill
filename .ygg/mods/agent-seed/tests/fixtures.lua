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
