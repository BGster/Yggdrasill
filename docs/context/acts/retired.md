# 退役终态 · retired
> **id** acts/retired · **幕** retired · **触发** `run.retire` 武装后的最后一次 `review.note`
> **读不到代价** silent-drift（终态只读，误读不会推进也不会损坏状态）
> **ver** c7f2a3

**场景台词（知识权威）**：本局已退役。

**阶段策略（retired_common）**：业务事件全拒；`map.show` / `requirement.list` 只读查询仍可用。

**ending 语义**：`[ENDING · run retired]` 前缀由 pack `agent.endingPrefix` 声明；结算汇总（任务、需求计数、批数、评分、演进候选、一句话教训）由 `endings.done` 键渲染，句式契约见 [copy/core.md](../copy/core.md)。

**退役核验**：武装后走最后一轮 `coordinate.submit → verify → settlement → review.note`，出现 `[ENDING · run retired]` 后运行 `--settle`，确认 `settlement.json` `ok=true` 及四开关（events 非空 / 无 failed step / ending 存在 / inbox 已消费）。对开放常驻局提前 `--settle` 无害但不结算（`reached=open, ok=false`，回到 running）。批结算后待新 `--requirement` 进料或武装退役；主会话结束前按 cleanup-policy 清空 todo（含完成与历史），不得带入下一局。

**prompts.retired 语义**：「本局已退役：只读查询（map / requirement.list）。」

**本幕关联阅读**：[copy/core.md](../copy/core.md)（`endings.done`）与 [copy/flow.md](../copy/flow.md)（`retired_common`）；[rituals/cleanup.md](../rituals/cleanup.md)（最终清理判据）。
