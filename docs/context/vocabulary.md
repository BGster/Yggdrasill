# Yggdrasill · 术语速览与核心领域词汇

> **定位**：全部领域术语的中英速查表与权威定义，是 wiki 内所有文件的词汇上游。
> **replaces**：CONTEXT 术语速览（L7–24）+ §2（L43–61）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[decisions-agent.md](decisions-agent.md) · [../design/state-machine.md](../design/state-machine.md)
> **搬运**：以下正文自 CONTEXT.md L7–L24 + CONTEXT.md L43–L61 逐字迁移，不重写语义。

## 术语速览（Glossary）

高频术语的中英对照速览，完整定义见 [§2 核心领域词汇](#2-核心领域词汇)：

| 术语 | 中文释义 |
| --- | --- |
| `Run` | 运行实例（常驻自循环：立局 → 批循环 → 显式退役） |
| `stage`（Run Stage） | 运行级真状态（`live` / `retired`）；协调 / 盘点 / 发布为 `run.phase` 微状态 |
| `Requirement`（`r1..rN`） | 需求（交付单元） |
| `Lane`（Persistent Lane Pool） | 持久化泳道工作树池。r1（D-MA24）起按**池**组织：默认池（""，运行仓自己的池，键形 `lane.<k>.*` 沿旧）+ 具名池（`lane.<label>.<k>.*`，`lane.pool "label\|path"` 注册本地检出路径），并行度按池计（每池 k ∈ 1..job_capacity）；多仓需求 = 注册 + 预挂牌 |
| `Lane Pool Registry`（`lane.pools` / `lane.pool.<label>.path`） | 具名池注册表（r1，D-MA24）：label 形状 `^[a-z][a-z0-9-]{0,23}$`（保留字 seq/pools），path 是传给宿主 `--repo` 的本地检出路径（仅形状校验，不验是否 git 仓——首次 claim 由宿主可读回传） |
| `Proposal`（`p1..pM`） | 提案（不可变交付提交） |
| `Gate`（`plan_gate` / `accept_gate`） | 强制门禁 |
| `Executor Ledger` | 执行者台账 |
| `HarnessAction`（`subagentAction`） | 治理引擎动作指令 |
| `Stable Checkpoint` | 稳定检查点 |

---

## 2. 核心领域词汇

| 术语 | 领域定义 |
| --- | --- |
| **Run（运行实例）** | 由 `run_id` 标识的单个隔离执行实例，位于 `.ygg/runs/<run_id>/` 下，持久化于单个 SQLite 数据库（`run.sqlite`），内含事件日志、状态快照、收件箱队列与虚拟文件系统产物。r9 起运行是**常驻自循环**：运行级终局不再由 review.note 自动触发，而是周期性批结算 + 显式退役（`done` 仅经 `run.retire` 武装可达）；升级不迁移旧局（装载失败即开新局）。 |
| **Run Phase（`run_phase()`，运行级双层状态）** | 一次 Run 的双层状态（D-MA23）：外层真状态 `STAGE = live / retired`（`fsm.stage`），内层微状态 `run.phase`（KV 取 `"charter"/""/"settle"/"release"`，空串 = 常驻主态协调，标识层规范化为 `coordinate`）：`charter`（立局三件套，`charter.set` + `run.begin`，boot 由 `event.start` 启动载荷承载章程）→ `coordinate`（常驻主态自循环：派发泳道、串行受理、双门禁、冲突清障）→ `settle`（盘点四项线性：`verify.submit` → `settlement.evolution` → `settlement.sessions` → `settlement.todo`）→ `release`（发布幕：`release.confirm` / `release.skip` 宣告 develop→master 结果，四项完成 R9 效应自动进入）→ `review.note` 批结算回 `coordinate`（`run.batch` +1）；`retired`（退役终态，仅 `run.retire` 武装后的 review.note 可达）。需求级 `done` 与运行退役态只是字面量同形（语义正交）。 |
| **Requirement（`r1..rN`，需求）** | 由状态机跟踪的一个交付单元。真状态只有 `open` / `done` 两个（D-MA23 R11）；`split` / `merged` / `dropped` 降为 `closed_reason` 字段（旁路终态判定 = `status=="done"` 且带 reason）。展示阶段由 `plan_ok`、`job`、`gate`、`conflict`、`integrating` 属性投影为九幕微状态（`req_phase()` 单一判定源）：`draft` 立项 → `planning` 计划中 → `plan_gate` 计划门禁 → `ready` 待开发 → `developing` 开发中 → `dev_review` 交付受理 → `accept_gate` 验收门禁 → `integrating` 集成 → `done`；旁路 `closed`（由 `closed_reason` 区分 split/merged/dropped）。 |
| **Persistent Lane Pool（`lane-1..4`，持久化泳道池）** | 一个有界池，最多 4 个可复用的 Git worktree（`~/tmp/worktrees/<repo>-lane-<k>`）。认领泳道时将其分支重置为 `HEAD` 但不删除未跟踪的构建缓存（`target/`、`node_modules/`）；释放泳道时将其停放在 detached `HEAD` 上，从而永不锁死分支。泳道分支名执行强制形态 `feature/{4位uuid}-{语义词段}`（`lane.claim` 与 `git.worktree_claim` 两条入口同规则，非法名可读拒绝；默认名生成机制已删除）——契约细节见 DESIGN §2.10。 |
| **Proposal（`p1..pM`，提案）** | 由执行者子代理针对特定 `(req_id, rev, stage)` 元组产出的不可变交付提交（`proposal.submit`），引用一个已归档的 VFS 产物（`artifacts/<req>-<stage>.md`）并绑定执行者身份（`exec=<agent_id>`）。提案在协调者串行执行 `proposal.accept` 或 `proposal.reject` 之前不会改变需求状态。 |
| **Mandatory Gates（`plan_gate`、`accept_gate`，强制门禁）** | 每个需求的两个不可跳过的质量检查点。`plan_gate` 批准技术方案（`plan_gate` → `ready` 幕）并响应式地自动派发开发。`accept_gate` 要求非空产物证据（`artifacts`）与匹配的修订号（`gate_rev == req.rev`），过后进 `integrating` 集成幕（D-MA22 分工：feature→develop 由开发执行者在泳道内集成，`integration.confirm` / `integration.reject`），集成确认后才 `done` 并释放其泳道。 |
| **Executor Ledger（`executor.<rN>.<stage>`，执行者台账）** | 一条自声明的审计轨迹，记录哪个子代理 ID 执行了某个需求的某个阶段（`analysis`、`plan`、`development`）。跨阶段交接或同一阶段更换为不同执行者 ID 需要显式理由（`<new_id>\|<reason>`）。占位 ID（`pi-ygg-harness`、`main`、`default`、`unregistered`）在恢复决策中被过滤掉。r3 起，台账 id 与提案的宿主 `actor=` 章在 `requirement.list` trace 上做三态互证显示（`[verified]` / `[mismatch:actor=…]` / `[unverified]`，识别不拒绝）——账本仍是权责权威，互证只是显示 join。 |
| **Profile Registry（`profile.<handle>.*`，身份注册表）** | 一张**自报身份卡**（self-reported registry，不是凭据）：`config.set profile.<handle>.<key>` 逐键注册的 `settings.profile.*` 平铺 KV（保留键 `harness` / `session_id` / `handle` / `role` / `assignments`，自由键 `^[a-z0-9_]{1,32}$`）。回答「这个句柄是谁」（harness / 原生 session id / 角色自报分工快照）；与执行者台账**互链不冗余**（渲染层按 `handle == executor id` 字符串 join，权责仍以账本为准）。投影：`map.show full` 的 `[PROFILES]` 行与 `agent.summary` 的 `profiles` 数组（≤8 张，handle 升序）。 |
| **In-Archive VFS（`artifacts/*`，归档内虚拟文件系统）** | `run.sqlite` 内的一张虚拟文件系统表（`vfs_files`），通过 `ygg.fs.*` 暴露给 Lua 沙箱，通过 `vfs.write` / JSON-RPC `vfs_put` 暴露给外部工具。存储方案、开发报告、演进审查与压缩交接，带 SHA-256 完整性。 |
| **Synchronous Tool Bridge（`ygg.tool.call`，同步工具桥）** | 一个可选加入的宿主能力，允许 Mod 在同一次 dispatch 回合内同步调用白名单内的确定性宿主仪式（`git.worktree_claim`、`git.lane_probe`、`git.worktree_release`、`fs.artifact_probe`、`gate.verify_suite`），消除多回合异步回调往返。 |
| **HarnessAction（`subagentAction`，治理引擎动作指令）** | Mod 在 `summary.jobs[i].subagentAction` 内发出的一条结构化机器可读指令（`subagent.spawn` 或 `subagent.resume`），携带渲染后的 `brief`、`deltaPrompt`、`fullFallbackBrief`、`worktreePath`、`artifactPath` 与 `submitCommand`，使任何上层治理引擎适配器都能执行泳道而无需重新实现工作流逻辑。 |
| **Stable Checkpoint（稳定检查点）** | 一个静止的运行状态（`available == true` $\land$ `pendingTools == 0` $\land$（`gates > 0` $\lor$ `jobs == 0` $\lor$ `conflicts > 0`）），在此状态下上下文压缩与结构化交接（`artifacts/handoff-latest.md`）可以安全发生而不丢失在途子代理输出。 |

---

