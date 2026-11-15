# 需求级九幕进行时 · 进料（req-acts/intake）
> **id** req-acts/intake · **幕** req: draft/planning/plan_gate · **触发** 立项 / 派发 plan 前
> **读不到代价** hard-refusal（合并判据与决策号规则漏读 → 立项被指出「本可合并」或 D-MA 双占）
> **ver** d5ff35

主流程五幕是「局的剧情」，需求级九幕是「每条需求的剧情」——协调幕内，每条需求各自走九幕（微状态）。本节给出**按幕展开的进行时指引**：场景台词（从 `beats-requirements.json` 对应键提炼）、入口条件 / 必做动作 / 出口判据、关联文件指针。

与 [../../design/requirement-story.md](../story/requirement-story.md) 的关系：**那边是机器契约**（微状态表 / 转场表 / 旁路覆盖层的判定序与副作用键），**这边是进行时指引**（我在哪一幕 / 该做什么 / 做完看什么证据）——语义不重复讲述，以那边为契约权威，本节只做执行侧台词与动作清单。九幕一览表见 [召回入口](../README.md)。

三道打回（交付受理 `proposal.reject` / 验收门禁 `gate.reject` / 集成 `integration.reject`）**统一落「待开发」**（plan_ok 保持、job 清空、泳道释放），打回不重走计划门禁（除非需求被 amend 使 plan_ok 失效）。

## 1. 立项（draft）

- **场景台词**（`req_added`）：`[REQ ADDED] {{id}} "{{title}}" {{deps}} — {{counts}}`——需求已立项入册，带依赖与总数。
- **入口条件**：人决定要做一件事；先过「需求粒度：能合并就先合并」——同目标、同子系统，或共享一个计划与一套验收证据的小改动必须合并；只有独立交付+验收、跨仓库/跨泳道、风险隔离才可新建。相邻新需求描述必须写不可合并理由，缺失即视为可合并。三问：① 阻断 `requirement.add` / `--requirement`；② 症状是执行者指出所谓新需求只是前一条的一部分而描述无不可合并理由；③ 走 `requirement.list`，逐条确认新需求带该理由。
- **决策 ID 预分配**：真正新的架构决策由主会话在派发时串行分配 `D-MAx`，写入需求描述和 brief；既有决策复用编号。执行者绝不自取 `max+1`——两泳道从同一 base 取号曾造成 D-MA16 双重认领（历史证据 `d0b4a7c` / `6848138`）；超出 brief 的 ID 返回协调者，不自取。
- **必做动作**：`requirement.add "<title>|<description>"`（可带外部幂等 id 与 deps 段）；描述写清目标、改动面、验收标准。
- **出口判据**：需求在册（draft），可被 `requirement.dispatch` 派发进计划。
- **本幕关联阅读**：[copy/requirements.md](../copy/requirements.md)（`req_added` / `req_dispatch_usage` / `req_bad_dep` / `req_cycle_dep` 句式）。

## 2. 计划中（planning）

- **场景台词**（`req_dispatch_ok` / `req_dispatch_item`）：`[DISPATCH] {{count}}: {{items}} | jobs {{jobs}}/{{cap}}`——需求已派发给执行者，计划作业在飞。
- **入口条件**：需求 draft 且依赖已就绪（deps 全 done）、有空闲 job 槽位。
- **必做动作**：主会话 `requirement.dispatch` 派发 plan 阶段；执行者读必读材料（[rituals/dispatch.md](../rituals/dispatch.md) §3）后产出计划稿，`proposal.submit(plan)` 提交（内联 `exec=<agent_id>` 注册执行者）。
- **出口判据**：plan 提案挂起（pending）→ 自动升入计划门禁（交付即门禁，无需再受理）。
- **本幕关联阅读**：[rituals/dispatch.md](../rituals/dispatch.md)（brief 结构 / 工件纪律 / submit 仪式）；[copy/requirements.md](../copy/requirements.md)（`req_prop_*` 提案句式、`req_prop_plan_gate` 提示）。

## 3. 计划门禁（plan_gate）

- **场景台词**（`req_gate_ok` kind=plan）：`[GATE PASSED · plan] {{id}} → ready | evidence: {{artifacts}} | note: {{note}}`——计划已过审，转待开发；`req_accept_plan` 提示「plan 提案的裁决走 gate.approve/reject（交付即门禁），proposal.accept 只受理 development 提案」。
- **入口条件**：plan 提案 pending（`gate=="plan"`）。
- **必做动作**：**人**审计划稿，`gate.approve`（过）或 `gate.reject`（打回，须新计划）；guide 模式下只能由人从菜单点选（人机门禁，见 [acts/coordinate.md](../acts/coordinate.md) `guide_human_blocked`）。
- **出口判据**：approve → 待开发（`plan_ok=true`，同回可自动派发开发）；reject → 回计划中（`plan_ok=false`，须重新出计划）。
- **本幕关联阅读**：[copy/requirements.md](../copy/requirements.md)（`req_gate_*` 拒绝面：own_props/own_job/stale/no_note 等）；[../../design/state-surface.md](../../design/kernel/state-surface.md)（`gate_rev==rev` stale 校验）。
