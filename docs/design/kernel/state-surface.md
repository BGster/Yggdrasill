# Yggdrasill · 状态面：存储键契约与拒绝面矩阵
> **id** kernel/state-surface · **模块** ① ygg 内核 · **触发** 写新 KV 键 / 改拒绝面之前
> **读不到代价** silent-drift（键写错或漏写拒绝面 → 存储漂移与缺口看不见）

> **定位**：KV 状态存储键契约（含已删除键清单）与各微状态的拒绝面矩阵。
> **replaces**：DESIGN §2.7 + §2.8（L285–328）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[state-machine.md](state-machine.md) · [run-story.md](../../context/story/run-story.md) · [requirement-story.md](../../context/story/requirement-story.md)
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
| `docs.maintenance.due` | 批次号（string） | 批结算登记的文档维护义务；非空即阻塞下一次 `coordinate.submit`。解除：`config.set docs.maintenance.done=<等值批次>`（空值 = 显式放弃） |
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

**删除**：`R_ANALYZING`、`PHASE`（14 名派生表）、`analysis` 提案槽、`fsm.stage` 的 `coordinate`/`review`/`done` 三值（为什么删见 §2.2.1）。旧局存档真状态取值变化 → 不兼容，不迁移、开新局（升级纪律）。

#### 2.8.0 设计依据（持久化权威与存储后端）

**持久化状态只存在于 Rust（`ygg.kv` / `ygg.entity`）**；Lua 局部与全局变量在单次 dispatch 内是临时的，`YGG-R2` 会标记未持久化的实体句柄。**新局默认单文件 SQLite（`<run-dir>/run.sqlite`）**，`events`、`snapshots`、`inbox`、`kv_projection` 与 `vfs_files`（`artifacts/*`）在同一事务里原子提交；既有的 JSON 局（`autosave.json` + `events.jsonl`）自动检测并留在 JSON 后端，**不做隐式迁移**。

**依据**：状态权威若允许存在于 Lua 边，回放就不能保证「同一事件序列 ⇒ 同一状态」，而那正是机检、接手与审计的地基；`YGG-R2` 不是风格检查，它是这条断言的执行面。存储选择单文件是为了「一局 = 一个可拷走的文件」（原子、零孤儿文件），而兼容旧 JSON 局是为了不把历史局变成只能读不能续的碎骸——但“不迁移”是硬线：升级后旧局不 adopt，开新 run-id。

**判据**：① 阻断——在 Lua 边保存跨 dispatch 的状态（表、句柄、全局）；或向旧 JSON 局写入新结构、或对旧局做迁移；② 症状——回放两次 KV 不一致，或 `YGG-R2` 诊断出现，或新局 `run.json` 的 `store` 与 `--store` 不一致仍能打开；③ 确认——`cargo run -q --example playtest -- .ygg/mods/agent-seed`（回放稳定）与 `cargo test --workspace` 中 `--store` 断言、`ygg-lint` 的 `YGG-R2` 用例。

### 2.8.1 沙箱时序源：可用与不可用（时序契约）

**结论**：沙箱内没有**任何宿主提供的可读单调序号**。要做时序判断（新鲜度、先后、纪元），序号必须由 KV 自维护——一个单写口计数器 + 一处封存点，全部落 KV，读档 / 重放 / 换代都从同一份状态复算。做时序相关特性之前先读本节，不要重新试一遍下面四格。

| 候选时序源 | 沙箱内读面 | 单调性 | 能排两次写的先后吗 | 证据 |
| --- | --- | --- | --- | --- |
| `KvStore::revision`（存档信封的 `kv.revision`） | **无**：`ygg` 只装 `kv.get` / `kv.set` 两个方法 | **不可判**：仅**实际状态变更** +1，同值覆写幂等不计 | 不能 | `crates/ygg-core/src/kv.rs:8-9`（语义）、`crates/ygg-sandbox/src/host_api.rs:806`（装面只此两法，见 `:780` / `:792`） |
| `ygg.ticks()` | **有**（返回 `u64`），但**无推进者** | 名义单调，生产路径恒 0 | 不能 | `crates/ygg-sandbox/src/inject.rs:282`（安装）、`:94`（唯一推进路径 `advance_ticks`，全仓零生产调用）、`:118`（初值 0）、`:506`（唯一单测，只经宿主句柄推进） |
| 事件 `seq`（回执首行 `seq N`） | **无**：宿主信封字段，不注入 Lua | — | 不能 | `crates/ygg-sandbox/src/inject.rs:112`（`install` 的安装面只有 math 白名单 + `ygg.ticks`） |
| serve 观察面的 `kv.revision` | 宿主 HTTP 只读面，**Mod / Lua 不可达** | — | 不能（不是 Mod 内判据） | [serve.md](../host/serve.md)（存档信封 `{kv:{entries,revision}}`） |

`ygg.ticks()` 那一行最容易误读：它**不是不存在，是没有推进者**。读它静默返回 `0`、不报错、不拒绝——所以「调一下试试」发现不了，只能靠这一行。

**可用面**：`ygg.kv` 自维护计数器。三条硬要求：单写口、每次派发恰好 +1、序号与纪元封存都能从同一份 KV 复算（即「派生依赖必须是 KV 标量」的同一纪律）。

| 范例 | 键 | 单写口 |
| --- | --- | --- |
| `src/core/clock.lua:6` | `settings.clock_seq` | `M.bump()`，唯一调用点在 `src/view/render.lua` 的重入守卫（每次 Mod 派发恰好一次） |
| `src/domain/lanes.lua:169-170` | `lane.seq` | 泳道分配时自增 |
| `src/runtime/tools.lua:175` | `tool.seq.*` | 工具调用序号分配 |

**历史成本**（同一结论在代码注释里已有一份非权威副本：`src/core/clock.lua:14-17`、`src/domain/readsig.lua:11-14`）：r3 规划「签名新鲜度」时先走「验票制」（记下序号再比较），正是被上表前三格依次作废，改走「销票制」——作用域边界主动清 KV，于是「键非空」本身即「本作用域内已读」，不需要时钟、也不需要比序号。任何「读一个宿主时钟来判先后」的方案，在这条路径上已经失败过一次。


