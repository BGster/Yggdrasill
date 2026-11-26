---
type: "Reference"
title: "协调幕 · coordinate — 派发、串行受理、两道门禁"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-9aedb7453b0c6f64c43d9da5
    resource: repo://.ygg/mods/agent-seed/data/texts/core.json
  - id: openwiki-source-645280a148f428290e9c5056
    resource: repo://.ygg/mods/agent-seed/src/core/config.lua
  - id: openwiki-source-20786f1cdc5649019aa1a3b9
    resource: repo://.ygg/mods/agent-seed/src/domain/gates.lua
  - id: openwiki-source-d57e21e5bbbce0b80489fd49
    resource: repo://.ygg/mods/agent-seed/src/domain/requirements.lua
  - id: openwiki-source-16d6aa00d55eb51a3953b319
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/choice.lua
  - id: openwiki-source-d5edd97ca7e6335c0cd02c35
    resource: repo://.ygg/mods/agent-seed/src/handlers/operate/review.lua
  - id: openwiki-source-1687a5fc04658c13a78d4590
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/gate_actions.lua
  - id: openwiki-source-3eb9b045e8a8eca336331ac7
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/integration.lua
  - id: openwiki-source-1a8a660d43047137a9cf72f5
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/proposals.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 协调幕 · coordinate — 派发、串行受理、两道门禁

> **id** acts/coordinate · **幕** coordinate · **触发** 进入协调幕 / 每次派发前
> **读不到代价** hard-refusal（载荷形状错、rev stale 被拒）
> **replaces**：`docs/context/acts/coordinate.md`（搬运基线 `3a2536f`）
> **上游**：[charter.md](charter.md) · **相关**：[../req-acts/intake.md](../req-acts/intake.md) · [../rituals/dispatch.md](../rituals/dispatch.md) · [../story/requirement-story.md](../story/requirement-story.md) · [../../contracts/host/lanes.md](../../contracts/host/lanes.md) · [../../contracts/kernel/state-machine.md](../../contracts/kernel/state-machine.md)

**场景台词（知识权威）**：任务「{{title}}」：{{goal}}｜{{count}} 条需求（{{open}} 条进行中）｜{{pending}} 条待受理提案。这一层你只做三件事：**派发、串行受理、过两道门禁**；其余需求的分析 / 计划 / 开发可异步并行。

## 1. 这一幕在做什么

**入口** `run.begin`。**循环（直到队列 drain）**：受理提案 → 过门禁 → 派发。**出口**：全部需求终态、0 待受理提案 / 0 运行 job / 0 未解冲突后，`coordinate.submit` → 盘点幕（`[COORDINATION DONE]`，`run.batch` 记批边界）。

顺序不是建议：**先受理（`proposal.accept`）→ 再过门禁（`gate.approve` / `gate.reject`）→ 再派发（`requirement.dispatch`）→ 最后 `coordinate.submit`**。跳序的典型症状是「批准了一个 rev 已旧的提案」——`gate.approve` 硬查 `gate_rev == req.rev`。

**协调层只做四件事**（`machine.coordinator-scope`）：派发、串行受理（accept / reject）、过两道门禁、走完批边界（`coordinate.submit` → 盘点四件 → `release.confirm` / `release.skip` → `review.note`）。它**从不自己开发**——特性的 feature→develop 集成属于该需求的开发执行者（`integration.confirm` / `integration.reject`），协调层也永不可绕过门禁。

## 2. 谁做决定：guide / goal

**只有 Mod 解释 `play.mode`**：runtime `settings.play.mode` 覆盖 `data/config.json` 的静态缺省；host CLI 不读、不解释。`--drive stdin|cmd:<path>|none` 只是宿主输入通道，与模式正交——goal 不依赖外部命令，删掉那条命令也不会破坏 goal。

- **`guide`（缺省）**：五个里程碑动作（`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`）只收菜单票；直发永远被拒（`[REJECTED:ACTION]`，主状态不变）。正确做法是把完整 `pendingChoice` 文本交给用户、等索引回来，**绝不代按**。
- **`*.reject` 不在票据门内**：它们不产生通过效果，是唯一纠正通道。
- **`goal`**：主代理读完整回执后自行派发——不是自动选 1、不经外部脚本；`pendingChoice` 仍完整打印，证据标准不变。切模式须用户当次授权后 `config.set play.mode=goal`，Mod 叙述 `handover_goal`（委托范围 / 证据不变 / Closing review 须交代）或 `handover_guide`（决策权归还人）。
- **`goal.authorization`**（`machine.goal-authorization`）：运行时 goal 必须逐局 `config.set goal.authorization=<单行说明>`（trim 后非空、≤200 字符、无换行；空值显式清除）。每批 `review.note` 前检查；缺失则叙述阻断、不写 `ending`、不前进。静态 goal 预设（仓库 owner 常设配置）不在检查范围；切回 guide 不查，resume 保留记录。该记录只证明「登记了一句话」，**绝不证明是用户亲口授权**，不得当身份证据。

## 3. 门禁与批边界（执行判据）

- **`proposal.accept` 只按阶段推进**：`analysis → planning`、`plan → plan_gate`、`development → accept`（交付同时释放泳道）。
- **`gate.approve`**：`plan_gate → ready`、`accept → done`。
- **`gate.reject` 都回 `planning`**：plan reject 置 `plan_ok=false`，须新计划；accept reject 保持旧值——未 amend 回 `ready` 重新开发，已 amend 因 `plan_ok` 失效停 `planning`。
- **approve 前硬查**：`gate_rev == req.rev`（amend 后旧批准 stale），以及 accept 轮的 `artifacts` 非空——**summary-only 不受理**。
- **`machine.accept-vs-gate`**：accept 只表示执行者自查后交付，不表示主代理验收过；**只有 `gate.approve` 是验收**。
- **`machine.gate-not-skippable`**：停在 `req.gate ~= ""` 的需求不可派发、不接受新提案、不可跳过；即使自动推进（`--drive cmd:…`）也一关一次 `gate.approve`，外部命令只提供索引。
- **关闭前置（共享）**：`coordinate.submit` / `verify.submit=true` / `review.note` 三者都要求 0 待受理提案、0 运行 job、0 未解冲突，且所有需求 `done`（或 `closed_reason` 为 split / merged / dropped 的后继全部 `done`）。
- **`coordinate.submit` 只在 coordinate 可用**：成功写 settle、`verify.passed=false`，并清本批 settlement 键（settings 预登记保留）。`verify.submit=false` 在 `gates.require_all_green=true` 时打回 coordinate 并清 settlement 键。

## 4. 常驻自循环与九幕投影

`machine.coordinator-chain`：内核 FSM 只有一条真转移 `live → retired`。一次完整的局：

```
charter（run.phase=="charter"）→ run.begin → coordinate（run.phase=="")
  ⇄ coordinate.submit → settle（verify.submit → settlement.evolution → settlement.sessions → settlement.todo）
  → release（四件齐备后自动推进）→ release.confirm|release.skip → review.note
  → 回 coordinate（run.phase="", run.batch+1）｜run.retire 武装 → retired
```

需求侧（`machine.pipeline-states`）：存储真值只有 **2 态**（`req.<i>.status` = `open` / `done`），其余全是派生——`split` / `merged` / `dropped` 是 `closed_reason` 的值（R11），不是状态。`open` 内九幕由唯一判定 `req_phase()` 派生：`draft → planning → plan_gate → ready → developing → dev_review → accept_gate → integrating → done`。三条打回路径（development 交付被 `proposal.reject` / accept 门 `gate.reject` / `integration.reject`）一律落 `ready`（R6：`plan_ok` 保留、`job=""`、释放泳道）；plan 门的 `gate.reject` 回 `planning` 且 `plan_ok=false`。两个叠加标记（`blocked` / `stale`）只影响显示，不是状态；`analyzing` 仅作旧存档兼容值。

**`machine.plan-gate`**：计划提案一提交就点火（gate-on-submit，计划侧无 `proposal.accept` 一跳）；approve → `plan_ok=true`（`ready` 是唯一可派发进开发的相位），reject → 回 `planning`、`plan_ok=false`、`rev+1`、反馈写入。

**`machine.accept-gate`**：主代理受理开发提案后点火；approve → `integrating`（`gate` 清空），**done 不在此颁发**——执行者还须落 `integration.confirm` 并带 `develop@<sha>` 证据；reject → `ready`。

## 5. 批边界后的知识库维护（本节更正一处旧文）

未武装的 `review.note` 批结算，在同一 handler 事务里登记 `docs.maintenance.due=<batch>`，并在回执里叙述 `[DOCS MAINTENANCE DUE]`；**它不派生需求行**——知识库维护是主会话自办（依据与两次实测见 [governance.md §6](../../governance.md)）。

> **旧文更正**：`docs/context/acts/coordinate.md` 里那条 `machine.batch-maintenance`（「自动追加一条维护更新 docs wiki 的需求」）是 2026-11-15 裁定**之前**的 playbook 原文；代码早已改为 KV 义务 + 批边界硬阻塞（`src/handlers/operate/review.lua` 的 `mark_docs_maintenance_due`）。两处并存即两个权威，本节以代码为准。

## 6. 管理面纪律（`machine.turn-cut`）

协调层的效果不得：渲染或选择、要求外部证据、通过人类门禁。不得绕过 `draft → planning` 合并或固定的 exec/actor 尾序。规范全文在 [state-machine.md](../../contracts/kernel/state-machine.md) §2.9；违反它的症状是拒绝载荷、不可重放、或出现第二条决策旁路。

## 7. 判据

① 阻断——跳序（先门禁后受理）、summary-only 验收、停在门禁上的需求被派发、guide 模式代按。② 症状——`gate.approve` 回 `stale`、`coordinate.submit` 被 pending / running / conflict 之一拒绝、`req.count` 因维护 +1。③ 确认——`requirement.list` 无停在门禁的行、`map.show` 的待受理提案计数为 0、`coordinate.submit` 回执只有 `[COORDINATION DONE]` 而不再带拒绝理由。

维护责任 = 改 `src/handlers/operate/**`、`src/domain/gates.lua` 或本幕正文的执行者；触发 = 门禁条件 / 相位判定 / 批边界语义变更；检查 = `agent_seed_semantics` 批边界与门禁用例 + 本页四条顺序与代码一致。
