# Yggdrasill · 状态面：存储键契约与拒绝面矩阵

> **定位**：KV 状态存储键契约（含 D-MA23 删除清单）与各微状态的拒绝面矩阵。
> **replaces**：DESIGN §2.7 + §2.8（L285–328）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[state-machine.md](state-machine.md) · [run-story.md](run-story.md) · [requirement-story.md](requirement-story.md)
> **搬运**：以下正文自 DESIGN.md L285–L305 + DESIGN.md L306–L328 逐字迁移，不重写语义。

### 2.7 拒绝面矩阵

每个微状态的**拒绝面**与拒绝理由。拒绝一律走 `flow.refuse` 可读拒绝（`[REJECTED:*]` 前缀、`reject.*` 留痕），文案键按 beats 分组命名（§2.12）：

| 微状态 | 拒绝的事件 | 拒绝理由 |
| --- | --- | --- |
| live·立局 | 全部业务事件（requirement.*/proposal.*/gate.*/lane.*/coordinate.submit/settlement.*/review.note） | 章程未立，局不可开工（先 `charter.set` + `run.begin`） |
| live·协调 | settlement.* / review.note / release.*；integration.*（目标需求不在集成幕时） | 结算/发布/退役属盘点与发布幕；integration.* 前置=该需求处于集成幕 |
| live·盘点 | requirement.dispatch / proposal.* / gate.* / lane.* / coordinate.submit / release.* | 派发与门禁在盘点幕冻结（先完成盘点四项） |
| live·发布 | requirement.dispatch / proposal.* / gate.* / lane.* / coordinate.submit / settlement.* | 发布幕只认 release.* 与 review.note；`review.note` 另要求发布已声明 |
| live·retired | 全部业务事件 | 已退役（真状态判据②：全局行为改变） |
| open·立项 | proposal.* / gate.* / coordinate.submit（自身） | 未派发即无提案可交 |
| open·计划中 | proposal.accept / gate.* / 第二个 proposal.submit | 受理与门禁前置是提案已交 |
| open·计划门禁 | requirement.dispatch / 第二个 proposal.submit | 一次一门禁：先裁决挂起门禁 |
| open·待开发 | proposal.* / gate.* | 无在跑作业，无提案可受理 |
| open·开发中 | proposal.accept / gate.* / 第二个 proposal.submit | 受理与门禁前置是提案已交 |
| open·交付受理 | requirement.dispatch / gate.* | 门禁裁决以受理为前置 |
| open·验收门禁 | requirement.dispatch / 第二个 proposal.submit | 一次一门禁：先裁决挂起门禁 |
| open·集成 | requirement.dispatch / proposal.* / gate.* | 集成幕只认 integration.confirm / integration.reject |
| open·done | 除 list / amend / split / merge 外全部 | 终态只读（旁路迁移除外） |

### 2.8 状态存储契约

| 键 | 取值 | 作用 |
| --- | --- | --- |
| `fsm.stage` | `live` / `retired` | **唯一真状态（运行级）**；唯一转场 `live → retired` |
| `run.phase` | `charter` / `""` / `settle` / `release` | 运行级微状态标志（初值 `charter`，`event.start` 写入，R2） |
| `run.release.master_sha` | string | `release.confirm` 写入的 `master@<sha>` |
| `run.release.tag` | string | `release.confirm` 写入的批次标签 `vYYYY.MM.DD.N` |
| `run.release.skipped_reason` | string | `release.skip` 写入的理由 |
| `run.batch` | int（缺省 1） | 批计数 |
| `task.title` / `task.goal` / `task.success` | string | 章程三件套（`run.begin` 后锁定） |
| `req.<i>.status` | `open` / `done` | **唯一真状态（需求级）**；终态判定 = `status=="done"` |
| `req.<i>.closed_reason` | `""` / `split` / `merged` / `dropped` | 关闭原因（**字段，不再是状态**，R11） |
| `req.<i>.job` | `""` / `plan` / `development` | 在跑任务 |
| `req.<i>.gate` | `""` / `plan` / `accept` | 门禁挂起 |
| `req.<i>.gate_rev` | int | 门禁绑定的 rev（stale 判定） |
| `req.<i>.plan_ok` | bool | 立项 / 待开发的分界 |
| `req.<i>.integrating` | bool | 第 8 幕（集成） |
| `req.<i>.develop_sha` | string | `integration.confirm` 写入的集成证据 |
| `req.<i>.lane` / `rev` / `conflict*` / `needs_amend` / `feedback` / `artifacts` / `summary` / `hist` | 各式 | 沿用（见 §2.4 / §2.10） |

**删除**（D-MA23）：`R_ANALYZING`、`PHASE`（14 名派生表）、`analysis` 提案槽、`fsm.stage` 的 `coordinate`/`review`/`done` 三值。旧局存档真状态取值变化 → 不兼容，不迁移、开新局（r9 升级纪律）。

