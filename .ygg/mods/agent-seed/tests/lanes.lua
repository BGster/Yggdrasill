
test("worktree_claim 绑泳道台账 / release 释放（含非法分支名拒绝）", function()
  fresh()
  charter_and_begin()
  fire("requirement.add", "synthetic requirement|synthetic description|subsys-r1")

  -- ① 正常认领：工具结果折进泳道台账与需求绑定。
  fire("tool.call", "git.worktree_claim|lane=2;req=r1;branch=feature/aa11-feat-a|t1")
  reset()
  fire(
    "tool.result",
    "t1|git.worktree_claim|ok|0|lane=2;req=r1;branch=feature/aa11-feat-a;base=abc1234;clean=1;path=/tmp/wt2"
  )
  assert_eq(count_marker("[WORKTREE CLAIMED]"), 1, "必须叙述已认领")
  assert_eq(kv("lane.2.req"), "r1", "台账记需求")
  assert_eq(kv("lane.2.branch"), "feature/aa11-feat-a", "台账记分支")
  assert_eq(kv("lane.2.base"), "abc1234", "台账记基线")
  assert_eq(kv("lane.2.clean"), "1", "台账记干净位")
  assert_eq(kv("lane.2.since"), 1, "台账记派发序号")
  assert_eq(kv("req.1.lane"), 2, "需求绑到泳道 2")
  assert_eq(kv("tool.reacted.t1"), true, "反应闩锁已落")

  -- ② 新 external_id 重放：只填幂等键，闩锁保证域折叠只做一次。
  reset()
  fire(
    "tool.result",
    "t1|git.worktree_claim|ok|0|lane=2;req=r1;branch=feature/aa11-feat-a;base=abc1234;clean=1;path=/tmp/wt2|ext-2"
  )
  assert_eq(count_marker("[WORKTREE CLAIMED]"), 0, "重放不得再折叠一次")

  -- ③ 非法分支名：叙述为拒绝，台账与 reject 账都不动。
  local rejects_before = kv("reject.count")
  fire("tool.call", "git.worktree_claim|lane=3;req=r1;branch=feature/r1|t3")
  reset()
  fire(
    "tool.result",
    "t3|git.worktree_claim|ok|0|lane=3;req=r1;branch=feature/r1;base=abc1234;clean=1;path=/tmp/wt3"
  )
  assert_eq(count_marker("[WORKTREE CLAIM REFUSED]"), 1, "非法分支名必须叙述为拒绝")
  assert_nil(kv("lane.3.branch"), "台账不得写")
  assert_eq(kv("reject.count"), rejects_before, "工具面的拒绝不得移动 reject 账")

  -- ④ 释放：工具带的是泳道号，不是需求号。
  fire("tool.call", "git.worktree_release|lane=2|t2")
  reset()
  fire("tool.result", "t2|git.worktree_release|ok|0|lane=2;released=1")
  assert_eq(count_marker("[WORKTREE RELEASED]"), 1, "必须叙述已释放")
  assert_eq(kv("lane.2.req"), "", "台账清空")
  assert_eq(kv("req.1.lane"), 0, "需求解绑")
end)
