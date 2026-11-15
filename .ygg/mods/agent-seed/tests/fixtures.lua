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
