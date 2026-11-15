
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

test("lane_probe 记真相 / 冲突雷达阻塞 / 失败只警告 / resolve 是唯一出口", function()
  fresh()
  charter_and_begin()
  fire("requirement.add", "left half|description|subsys-a")
  fire("requirement.add", "right half|description|subsys-b")
  fire("lane.claim", "1|r1|||1")
  fire("lane.claim", "2|r2|||1")

  -- ① 探针落 KV，此时还没有重叠。
  fire("tool.call", "git.lane_probe|lane=1;base=abc;branch=feature/aa11-left|t1")
  reset()
  fire(
    "tool.result",
    "t1|git.lane_probe|ok|0|lane=1;branch=feature/aa11-left;base=abc;head=1111111;commits=2;clean=1;files=src/a.lua,src/b.lua"
  )
  assert_eq(kv("lane.1.probe.commits"), 2, "提交数")
  assert_eq(kv("lane.1.probe.clean"), "1", "干净位")
  assert_eq(kv("lane.1.probe.head"), "1111111", "HEAD")
  assert_eq(kv("lane.1.probe.files"), "src/a.lua,src/b.lua", "触碰文件")
  assert_eq(kv("lane.1.probe.call"), "t1", "探针来源 call")
  assert_eq(kv("req.1.conflict"), "", "尚无冲突")

  -- ② 共享文件 → 由**与人工路径同一个写入器**抬高冲突。
  fire("tool.call", "git.lane_probe|lane=2;base=abc;branch=feature/aa11-right|t2")
  reset()
  fire(
    "tool.result",
    "t2|git.lane_probe|ok|0|lane=2;branch=feature/aa11-right;base=abc;head=2222222;commits=1;clean=1;files=src/b.lua,src/c.lua"
  )
  assert_eq(count_marker("[CONFLICT RADAR]"), 1, "雷达必须叙述一次")
  assert_eq(kv("req.1.conflict"), "overlapping file: src/b.lua", "冲突正文")
  assert_eq(kv("req.1.conflict_peer"), "r2", "对手需求")
  assert_eq(kv("req.1.conflict_note"), "overlapping file: src/b.lua", "备注")
  assert_eq(kv("req.1.conflict_source"), "radar", "来源标记为雷达")
  assert_eq(kv("req.2.conflict_peer"), "r1", "反向也记")
  assert_eq(kv("req.conflicts"), 2, "冲突总数")
  assert_eq(phase_of("r1"), "blocked", "r1 受阻")
  assert_eq(phase_of("r2"), "blocked", "r2 受阻")

  -- ③ 失败探针（commits=0）只警告，绝不阻塞：不产生新冲突，状态不动。
  fire("tool.call", "git.lane_probe|lane=2;base=abc;branch=feature/aa11-right|t3")
  reset()
  fire(
    "tool.result",
    "t3|git.lane_probe|ok|0|lane=2;branch=feature/aa11-right;base=abc;head=2222222;commits=0;clean=1;files=src/b.lua"
  )
  assert_eq(count_marker("[LANE PROBE FAILED]"), 1, "失败探针必须叙述警告")
  assert_eq(kv("req.conflicts"), 2, "冲突总数不变")
  assert_eq(kv("lane.2.probe.commits"), 0, "探针真相照记")

  -- ④ `requirement.resolve` 是唯一出口：四个键一起回空。
  fire("requirement.resolve", "r1,r2|keep_both|re-checked")
  assert_eq(kv("req.conflicts"), 0, "冲突总数归零")
  assert_eq(kv("req.1.conflict"), "", "正文清空")
  assert_eq(kv("req.1.conflict_peer"), "", "对手清空")
  assert_eq(kv("req.1.conflict_note"), "", "备注清空")
  assert_eq(kv("req.1.conflict_source"), "", "来源清空")
  assert_eq(kv("req.2.conflict_source"), "", "反向来源清空")
  assert_eq(phase_of("r1"), "draft", "回到 draft")
end)
