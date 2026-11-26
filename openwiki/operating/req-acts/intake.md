---
type: "Reference"
title: "需求级九幕 · 进料（draft → planning → plan_gate）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-d57e21e5bbbce0b80489fd49
    resource: repo://.ygg/mods/agent-seed/src/domain/requirements.lua
  - id: openwiki-source-1687a5fc04658c13a78d4590
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/gate_actions.lua
  - id: openwiki-source-1a8a660d43047137a9cf72f5
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/proposals.lua
  - id: openwiki-source-4dcd58388243470923e255aa
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/requirement_ops.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 需求级九幕 · 进料（draft → planning → plan_gate）

> **id** req-acts/intake · **幕** req: draft / planning / plan_gate · **触发** 立项 / 派发计划前
> **读不到代价** hard-refusal（合并判据与决策落点规则漏读 → 立项被指出「本可合并」或决策被重复占位）
> **replaces**：`docs/context/req-acts/intake.md`（搬运基线 `3a2536f`）
> **上游**：[../../architecture/README.md](../../architecture/README.md) · **相关**：[delivery.md](delivery.md) · [integration.md](integration.md) · [../story/requirement-story.md](../story/requirement-story.md)

五幕是「局的剧情」，需求级九幕是「每条需求的剧情」——协调幕内每条需求各自走九幕微状态。本页是**执行侧指引**（我在哪一幕 / 做什么 / 看什么证据）；机器契约（微状态表 / 转场 / 旁路判定序）的权威是 [requirement-story](../story/requirement-story.md)。

三道打回（`proposal.reject` / `gate.reject` / `integration.reject`）**统一落「待开发」**：`plan_ok` 保持、`job` 清空、泳道释放；打回不重走计划门禁（除非需求被 amend 使 `plan_ok` 失效）。

## 1. 立项（draft）

- **台词**（`req_added`）：`[REQ ADDED] {{id}} "{{title}}" {{deps}} — {{counts}}`。
- **入口条件**：人决定要做一件事。先过**需求粒度**：同目标、同子系统，或共享一个计划与一套验收证据的小改动必须合并；只有独立交付+验收、跨仓库/跨泳道、风险隔离才可新建。相邻新需求描述必须写**不可合并理由**，缺失即视为可合并。
- **决策就地入册**：真正新的架构决策**不取号、不记台账**——在实现它的同一次提交里写进**它治理那份文档**的「设计依据」段（规范 + 依据 + 三问判据）；需求描述与 brief 只写「这条决策落在哪个文件哪一节」。两泳道同时写同一条决策时，后写者以既有段落为基线**原地合并**。历史教训：靠「预订编号」协调时，两泳道从同一 base 各取下一号，造成同一编号被双重认领（证据 `d0b4a7c` / `6848138`）——取号不是协调机制，锚点才是。
- **必做动作**：`requirement.add "<title>|<description>"`（可带外部幂等 id 与 deps 段）。
- **出口判据**：需求在册（draft），可被 `requirement.dispatch` 派进计划。
- **本幕阅读**：[../scenes/README.md](../scenes/README.md)（`req_added` / `req_dispatch_usage` / `req_bad_dep` / `req_cycle_dep`）。

## 2. 计划中（planning）

- **台词**（`req_dispatch_ok` / `req_dispatch_item`）：`[DISPATCH] {{count}}: {{items}} | jobs {{jobs}}/{{cap}}`。
- **入口条件**：需求 draft、依赖已就绪（deps 全 done）、有空闲 job 槽位。
- **必做动作**：主会话 `requirement.dispatch` 派 plan 阶段；执行者读完必读材料（[../rituals/dispatch.md](../rituals/dispatch.md) §3）产出计划稿，`proposal.submit(plan)` 提交（内联 `exec=<agent_id>` 注册执行者）。
- **出口判据**：plan 提案挂起 → **自动**升入计划门禁（交付即门禁，无需再受理）。
- **本幕阅读**：[../rituals/dispatch.md](../rituals/dispatch.md)（brief 结构 / 工件纪律 / submit 仪式）、[../scenes/README.md](../scenes/README.md)（`req_prop_*` 与 `req_prop_plan_gate`）。

## 3. 计划门禁（plan_gate）

- **台词**（`req_gate_ok` kind=plan）：`[GATE PASSED · plan] {{id}} → ready | evidence: {{artifacts}} | note: {{note}}`；`req_accept_plan` 提示：plan 提案裁决走 `gate.approve/reject`（交付即门禁），`proposal.accept` 只受理 development 提案。
- **入口条件**：plan 提案 pending（`gate=="plan"`）。
- **必做动作**：**人**审计划稿，`gate.approve` 或 `gate.reject`；guide 模式下只能由人从菜单点选（人机门禁，见 [../acts/coordinate.md](../acts/coordinate.md) `guide_human_blocked`）。
- **出口判据**：approve → 待开发（`plan_ok=true`，同一回可自动派发开发）；reject → 回计划中（`plan_ok=false`，须重新出计划）。
- **本幕阅读**：[../scenes/README.md](../scenes/README.md)（`req_gate_*` 拒绝面：`own_props` / `own_job` / `stale` / `no_note`）、[../../contracts/kernel/state-surface.md](../../contracts/kernel/state-surface.md)（`gate_rev==rev` 的 stale 校验）。

## 4. 判据

① 阻断——为能合并的小改动新开需求（描述无不可合并理由）、把新决策写成「D-MA 编号预订」而不写进治理文档、guide 模式下由 agent 代点计划门禁。② 症状——执行者回执指出「这条只是前一条的一部分」、两个泳道写同一节决策而互相覆盖、`req_gate` 拒绝面出现 `own_props` / `stale`。③ 确认——`requirement.list` 里每条新需求带不可合并理由；`git log -S'<决策关键词>'` 显示决策只在其治理文档里成文一次；`req_gate_ok` 带人类菜单索引。

维护责任 = 改立项判据、决策落点规则或计划门禁语义的执行者；触发 = 粒度判据变化 / 新增旁路 / 门禁载荷变化；检查 = 上述三问与 `src/domain/requirements.lua`、`src/handlers/requirements/*.lua` 的判定一致。
