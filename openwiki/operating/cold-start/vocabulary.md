---
type: "Reference"
title: "术语速览与核心领域词汇 — 一切动作名的权威定义"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c1b4b807feda7647ba638e19
    resource: repo://.ygg/mods/agent-seed/data/config.json
  - id: openwiki-source-f2294347eab5094b120f7ba9
    resource: repo://.ygg/mods/agent-seed/src/domain/executor.lua
  - id: openwiki-source-1c80d889f00dcfd1c85bbf4a
    resource: repo://.ygg/mods/agent-seed/src/domain/readsig.lua
  - id: openwiki-source-d57e21e5bbbce0b80489fd49
    resource: repo://.ygg/mods/agent-seed/src/domain/requirements.lua
  - id: openwiki-source-73e0e7bd617e061fcace7a96
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/lanes.lua
  - id: openwiki-source-b640c22b435c5b19de7f36dd
    resource: repo://.ygg/mods/agent-seed/src/handlers/system/config.lua
  - id: openwiki-source-422bb41f0e2d55ebad3aceea
    resource: repo://crates/ygg-sandbox/src/host_contract/agent.rs
  - id: openwiki-source-2bd7560039b90f86b6e54851
    resource: repo://crates/ygg-store/src/sqlite/mod.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 术语速览与核心领域词汇 — 一切动作名的权威定义

> **id** cold-start/vocabulary · **幕** cold-start · **触发** 冷启动 / 读懂任何动作名
> **读不到代价** hard-refusal（术语错则动作语义错）
> **replaces**：`docs/context/cold-start/vocabulary.md`（搬运基线 `3a2536f`，原 CONTEXT 术语表 + §2）
> **上游**：[overview.md](../../overview.md) · **相关**：[../../contracts/host/lanes.md](../../contracts/host/lanes.md) · [../../contracts/kernel/state-machine.md](../../contracts/kernel/state-machine.md)

## 1. 速查表

| 术语 | 中文释义 |
| --- | --- |
| `Run` | 运行实例（常驻自循环：立局 → 批循环 → 显式退役） |
| `stage`（Run Stage） | 运行级真状态（`live` / `retired`）；协调 / 盘点 / 发布是 `run.phase` 微状态 |
| `Requirement`（`r1..rN`） | 需求（交付单元） |
| `Proposal`（`p1..pM`） | 提案（不可变交付提交） |
| `Gate`（`plan_gate` / `accept_gate`） | 两道人门禁 |
| `Lane` | 持久化泳道工作树池（按池组织，每池并行度 `job_capacity`） |
| `Executor Ledger` | 执行者台账（哪个子代理做了哪个阶段） |
| `HarnessAction`（`subagentAction`） | 治理引擎动作指令（brief + 提交命令） |
| `Stable Checkpoint` | 稳定检查点（可安全压缩上下文与交接） |
| `dispatch` / `brief` | 派发 / 派发稿（本仓开发流程用词，见 §3） |

## 2. 核心领域词汇

| 术语 | 领域定义 |
| --- | --- |
| **Run（运行实例）** | 由 `run_id` 标识的隔离执行实例，位于 `.ygg/runs/<run_id>/`，持久化为单个 SQLite（`run.sqlite`）：事件日志、状态快照、收件箱队列、虚拟文件系统。r9 起运行是**常驻自循环**——运行级终局不再由 `review.note` 自动触发，而是周期批结算 + 显式退役（`done` 仅经 `run.retire` 武装可达）；升级不迁移旧局（装载失败即开新局）。 |
| **Run Phase（双层状态）** | 外层真状态 `STAGE = live / retired`（`fsm.stage`），内层微状态 `run.phase` 取 `charter` / `""` / `settle` / `release`（空串 = 常驻主态，标识层规范化为 `coordinate`）：`charter`（三件套 + `run.begin`）→ `coordinate`（派发、串行受理、双门禁、清障）→ `settle`（四项线性：`verify.submit` → `settlement.evolution` → `settlement.sessions` → `settlement.todo`）→ `release`（`release.confirm` / `release.skip` 宣告 develop→master 结果）→ `review.note` 回 `coordinate`（`run.batch` +1）；`retired` 只能经 `run.retire` 武装后由 `review.note` 抵达。需求级 `done` 与运行退役态只是字面量同形，语义正交。 |
| **Requirement（`r1..rN`）** | 一个交付单元。真状态只有 `open` / `done`；`split` / `merged` / `dropped` 降为 `closed_reason`（旁路终态判定 = `status=="done"` 且带 reason）。展示阶段由 `plan_ok` / `job` / `gate` / `conflict` / `integrating` 投影为九幕微状态（`req_phase()` 单一判定源）：`draft` → `planning` → `plan_gate` → `ready` → `developing` → `dev_review` → `accept_gate` → `integrating` → `done`，旁路 `closed`。 |
| **Persistent Lane Pool（`lane-1..4`）** | 有界池，最多 4 个可复用 Git worktree（`~/tmp/worktrees/<repo>-lane-<k>`；具名池见 [lanes 契约](../../contracts/host/lanes.md)）。认领时把分支重置到 `HEAD` 但保留未跟踪构建缓存（`target/`、`node_modules/`）；释放时停在 detached `HEAD`，永不锁死分支。泳道分支名强制形态 `feature/{4位uuid}-{语义词段}`，两条入口（`lane.claim` / `git.worktree_claim`）同规则，非法名可读拒绝。 |
| **Proposal（`p1..pM`）** | 执行者针对特定 `(req_id, rev, stage)` 产出的不可变交付提交，引用已归档产物 `artifacts/<req>-<stage>.md` 并绑定执行者身份（`exec=<agent_id>`）。在协调者串行 `proposal.accept` / `proposal.reject` 之前不改需求状态。 |
| **Mandatory Gates** | 每个需求两个不可跳过的检查点。`plan_gate` 批准技术方案并自动派发开发；`accept_gate` 要求非空产物证据与匹配修订（`gate_rev == req.rev`），过后进 `integrating`——feat→develop 由开发执行者在泳道内完成（`integration.confirm` / `integration.reject`），确认后才 `done` 并释放泳道。 |
| **Executor Ledger（`executor.<rN>.<stage>`）** | 自声明的审计轨迹：谁执行了某需求的某阶段（`analysis` / `plan` / `development`）。跨阶段交接或换人需显式理由（`<new_id>\|<reason>`）；占位 ID（`pi-ygg-harness`、`main`、`default`、`unregistered`）在恢复决策中被过滤。台账 id 与提案 `actor=` 章在三态互证显示（`[verified]` / `[mismatch:actor=…]` / `[unverified]`，只显示不拒绝）——账本仍是权责权威。 |
| **Profile Registry（`profile.<handle>.*`）** | **自报身份卡**（不是凭据）：`config.set profile.<handle>.<key>` 写入的 `settings.profile.*` 平铺 KV（保留键 `harness` / `session_id` / `handle` / `role` / `assignments`，自由键 `^[a-z0-9_]{1,32}$`）。回答「这个句柄是谁」；与台账**互链不冗余**（渲染层按 `handle == executor id` 字符串 join）。投影：`map.show full` 的 `[PROFILES]` 行、`agent.summary` 的 `profiles`（≤8 张）。 |
| **In-Archive VFS（`artifacts/*`）** | `run.sqlite` 内的虚拟文件系统表（`vfs_files`），经 `ygg.fs.*` 暴露给沙箱、经 `vfs.write` / `vfs_put` 暴露给外部工具；存方案、报告、演进审查与压缩交接，带 SHA-256 完整性。 |
| **Tools Registry（`tools.*`）** | **自声明的工具注册表**：`tools.root` = 机证启用开关（脚本根目录，未注册 = 人证回退）；`tools.session` / `tools.sessions_root` = 保留参数键（沙箱读不到进程环境，argv 只能来自 KV）；`tools.<name>` = 脚本路径覆盖（`.sh` 结尾）。与台账 / Profile 同构：直写、幂等 upsert、不占人类门禁、未注册 = 消费点可读拒绝。正文见 [../../contracts/host/harness-tools.md](../../contracts/host/harness-tools.md)。 |
| **Synchronous Tool Bridge（`ygg.tool.call`）** | 可选加入的宿主能力，允许 Mod 在同一次 dispatch 内同步调用白名单内的确定性仪式（`git.worktree_claim`、`git.lane_probe`、`git.worktree_release`、`fs.artifact_probe`、`gate.verify_suite`），消除多回合往返。 |
| **HarnessAction（`subagentAction`）** | Mod 在 `summary.jobs[i].subagentAction` 发出的结构化指令（`subagent.spawn` / `subagent.resume`），携带渲染后的 `brief`、`deltaPrompt`、`fullFallbackBrief`、`worktreePath`、`artifactPath` 与 `submitCommand`，使任何适配器都能执行泳道而无需重写工作流。 |
| **Stable Checkpoint** | 静止状态（`available == true` ∧ `pendingTools == 0` ∧（`gates > 0` ∨ `jobs == 0` ∨ `conflicts > 0`）），此时压缩上下文与写结构化交接（`artifacts/handoff-latest.md`）不会丢在途子代理输出。 |

## 3. 本仓开发用词（与产品术语分开）

本仓的开发**不走**上面这条需求状态机，它是一套提示词 + 技能 + 子代理的循环（见仓库根 `AGENTS.md`）。两套词不要混用：

| 本仓开发用词 | 含义 |
| --- | --- |
| **dispatch（派发）** | 主会话把一个自洽任务交给执行者：brief 八段（背景 / 工作区 / 必读 / 硬约束 / 动作 / 验收 / 回传 / 禁改）+ 独立上下文 |
| **brief（派发稿）** | 一次派发的完整文本；执行者不看会话也能独立推进 |
| **执行者（executor）** | 承接派发的子代理（`kernel-dev`、`pkg-author`、通用 worker），泳道内改代码、跑门禁、提交 |
| **泳道（lane / worktree）** | 每个仓库一个 worktree；同仓串行、异仓并行 |
| **核验（verification）** | 主会话独立复核：commit + 净 diff + 门禁复跑 + 活链实测——不轻信汇报 |
| **批结算** | 一批任务收尾：验证清单、文档回流、`git merge --no-ff` 进 `develop` |

## 4. 判据

① 阻断——把产品术语（`requirement.dispatch`、`proposal.submit`、`plan_gate`）当成本仓开发流程的名字，或反向用「派发 / 泳道」描述 egg 内部机制。② 症状——同一句话里混用两套动词、把 `run.phase` 与需求九幕说成一回事、把 `done` 的运行态当需求完成。③ 确认——`grep -n 'requirement.dispatch' AGENTS.md` 只出现在「产品行为」语境，开发指令一律用 §3 的词。

维护责任 = 新增或改名任何动作 / 状态的执行者；触发 = 状态机、注册表或动作名变更；检查 = 本表每条与 `src/domain/**` 及契约页的同名条目逐字一致，两套词的混用点在评审中被点名。
