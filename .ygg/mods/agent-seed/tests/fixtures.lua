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
