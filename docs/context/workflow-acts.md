# Yggdrasill · 幕台词与工作流进行时（workflow-acts）

> **定位**：五幕（立局/协调/盘点/发布/退役）的「进行时台词」知识权威——场景叙事、阶段策略、每幕入口/必做/出口动作清单，以及**无 egg 环境的人工执行法**。mod 的 `scenes` / `prompts` / 六个 `<node>_common` 键是本文的运行时投影（状态前缀 + 指针）；语义以本文为准。
> **replaces**：r2「mod 剧情提示词迁出」前散置于 `.ygg/mods/agent-seed/data/texts/core.json`（scenes/prompts）与 `beats-flow.json`（`*_common`）的幕叙事正文。
> **上游**：[../README.md](../README.md) · **相关**：[dispatch-ritual.md](dispatch-ritual.md) · [cleanup-policy.md](cleanup-policy.md) · [copy-contract.md](copy-contract.md) · [../design/run-story.md](../design/run-story.md)（微状态机契约，语义不重复讲述）

## 读法

- 本文是**给人与 agent 读的执行底座**：每一幕「我在哪 / 说什么 / 做什么 / 下一步动作名」。
- mod 运行时在每幕开场输出场景行（`scenes.<act>`），在每幕内输出菜单与阶段提示（`prompts.<act>`、`<act>_common`）；这些键值只保留状态变量与短句，**叙事与策略正文以本文为唯一权威**（改文案先改本文再同步 mod，见 [../wiki-governance.md](../wiki-governance.md)）。
- 菜单措辞（core `options` 27 键）由 mod 运行时渲染；本文只在各幕「菜单语义对照」小节解释每个菜单项的语义，不复制键值（键值契约见 [copy-contract.md](copy-contract.md) core 节）。

## 序幕 · prologue（boot）

**场景台词（知识权威）**：引擎就绪，ygg 世界已装载（引擎版本、当前相位、演进偏好仅作叙述——资源从不被自动改写）。这是双层代理元工作流：协调层只做派发、串行受理、过两道门禁；每个需求走九幕（立项 → 计划中 → 计划门禁 → 待开发 → 开发中 → 交付受理 → 验收门禁 → 集成 → 完成）。

**阶段策略（prologue_common）**：两种模式（guide/goal）共享同一纪律——受理与两道门禁各需一次真实动作与一次核验，永不跳过；需求可异步追加（`--requirement` / `--proposal` 只进 inbox）。

**入口**：进程 boot（`event.start`，可带章程载荷）。**必做**：无（纯叙述）。**出口**：进入立局幕。

**本幕关联阅读**：[copy-contract.md](copy-contract.md) flow 节（`play_*` / `rejected*` 句式）；[../design/state-machine.md](../design/state-machine.md)（微状态判定）。

## 立局幕 · charter

**场景台词（知识权威）**：立局 · 第 {{batch}} 批前。章程：「{{title}}」｜目标：{{goal}}｜成功判据：{{success}}。`charter.set` 在立局内可反复修订；三件套齐备后 `run.begin` 开工（章程随之锁定）。

**阶段策略（charter_common）**：章程三件套（标题 / 目标 / 成功判据）立好才可开工；`charter.set` 在立局内可反复修订，`run.begin` 后章程锁定。

**入口**：序幕后自动处于立局微状态。**必做**：① `charter.set`（或 boot 载荷一步立局）写齐三件套；② `requirement.add` 至少一条需求（否则 `coordinate.submit` 被 `gate_charter_first` 拒绝）。**出口**：`run.begin` → 协调幕（`[RUN BEGUN]`）。

**菜单语义对照**：`charter.set`（修订章程）、`run.begin`（开工）、`requirement.add`（异步立项）、`map.show`（看地图）、`config`（配置面板）。

**prompts.charter 语义**：下一步问题只保留动作名——「修订章程（charter.set），三件套齐备后开工（run.begin）？」

**本幕关联阅读**：[copy-contract.md](copy-contract.md) flow 节（`charter_*` / `run_begun` / `gate_charter_first` 句式）；[../design/run-story.md](../design/run-story.md)（立局微状态）；需求写法与合并判据见 AGENTS.md §1.2（需求粒度）。

## 协调幕 · coordinate

**场景台词（知识权威）**：任务「{{title}}」：{{goal}}｜{{count}} 条需求（{{open}} 条进行中）｜{{pending}} 条待受理提案。这一层你只做三件事：**派发、串行受理、过两道门禁**；其余需求的分析/计划/开发可异步并行。

**阶段策略（coordinate_common）**：先受理提案（`proposal.accept`），再过门禁（`gate.approve` / `gate.reject`），再派发（`requirement.dispatch`），最后 `coordinate.submit` 收口进盘点。

**阶段策略（guide_hint，map-guide 组）**：现场规则（field rules）按章节索引——每条规则的标题与摘要在菜单提示行给出，全文在 `.ygg/mods/agent-seed/data/playbook/` 对应章；guide 模式下里程碑动作只能由人从菜单点选。

**人机门禁拒绝（guide_human_blocked，知识权威）**：`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note` 是 pass 型动作，受人类动作门禁保护——guide 模式下只能由人从菜单点选（回复索引 → `ygg.choice_selected`）；直接单发一派发永远被拒（`[REJECTED:ACTION]`），主状态不变。正确做法：把完整 pendingChoice 文本交给用户并等待索引，绝不代按。

**入口**：`run.begin`。**必做**（循环直到队列 drain）：受理 → 门禁 → 派发。**出口**：全部需求终态、0 待受理提案 / 0 运行 job / 0 未解冲突后，`coordinate.submit` → 盘点幕（`[COORDINATION DONE]`，run.batch 记批边界）。

**菜单语义对照**：`requirement.dispatch`（按就绪序派发）、`proposal.accept`（串行受理）、`gate.approve`/`gate.reject`（过/打回门禁）、`coordinate.submit`（批边界）、`requirement.list`（看板）、`map.show`、`config`。

**prompts.coordinate 语义**：下一步问题只保留动作名——「受理提案、过门禁、派发，还是收口进盘点（coordinate.submit）？」

**本幕关联阅读**：**[需求级九幕进行时](#需求级九幕进行时)（本文下节，协调幕内每条需求的九幕指引）**；[dispatch-ritual.md](dispatch-ritual.md)（派发仪式全文）；[copy-contract.md](copy-contract.md) requirements / lanes 节（派发/提案/门禁/泳道句式）；[../design/requirement-story.md](../design/requirement-story.md)（九幕微状态表）、[../design/lanes.md](../design/lanes.md)（泳道池）、[../design/state-surface.md](../design/state-surface.md)（拒绝面矩阵）。

## 盘点幕 · settle

**场景台词（知识权威）**：任务「{{title}}」：{{count}} 条需求（{{done}} 条已终态）｜require_all_green {{gates}}。四项齐备（验证、演进登记、会话清理、待办清理）自动进发布幕。

**阶段策略（settle_common）**：盘点四项——`verify.submit` 先记全绿 → `settlement.evolution` 登记演进（可并入 `sessions=` / `todo=` 可选段一次事务）→ `settlement.sessions` → `settlement.todo`；四项齐备自动转发布幕（不占事件）。会话清理与待办清理的政策正文见 [cleanup-policy.md](cleanup-policy.md)。

**入口**：`coordinate.submit` 成功（verify.passed 复位为 false，本批 settlement 键清空）。**必做**：① `verify.submit` 记全绿（`all_green=false` 且 `require_all_green=true` 时打回协调幕返工）；② `settlement.evolution` 登记（summary 必填）；③④ 会话与待办清理声明。**出口**：四项齐备 → 发布幕（`[SETTLE READY]`，R9 自动转场）。

**菜单语义对照**：`verify_green` / `verify_fail`（记绿/记不绿）、`settlement_evolution`（登记演进）、`review_note`（批结算/退役）、`map.show`。

**prompts.settle 语义**：「门禁全绿了吗？依次验证 → 演进登记 → 会话 → 待办；四项齐备自动进发布幕。」

**本幕关联阅读**：[cleanup-policy.md](cleanup-policy.md)（会话/待办清理政策）；[copy-contract.md](copy-contract.md) settlement 节（`verify_*` / `settlement_*` / `review_*` 句式）；[../design/state-surface.md](../design/state-surface.md)（`verify.*` / `settlement.*` / `review.evolution.*` 存储键契约）；演进候选筛选纪律见 AGENTS.md §1.7 与 playbook evolution 章。

## 发布幕 · release

**场景台词（知识权威）**：第 {{batch}} 批 · 任务「{{title}}」{{declared}}。声明本批 develop→master 合并结果（`release.confirm` / `release.skip`）；已声明即可 `review.note` 批结算（未武装）或退役（`run.retire` 武装）。

**阶段策略（release_common）**：`release.confirm` 载荷声明 `master@<sha>` 与批次 tag（`v年.月.日.序号`），或 `release.skip` 声明跳过理由；任一成立即「发布已声明」，`review.note` 才可批结算/退役。

**入口**：盘点四项齐备自动进入。**必做**：发布声明二选一。**出口**：`review.note`——未武装 = 批结算回协调幕（`[BATCH n SETTLED]`，run.batch+1）；武装 = 写 ending 退役（`[ENDING · run retired]`）。

**菜单语义对照**：`release.confirm` / `release.skip`（声明/跳过）、`review_note`（收口）。

**prompts.release 语义**：「声明本批结果（release.confirm / release.skip）后 review.note 批结算；run.retire 武装时它是终局。」

**本幕关联阅读**：[copy-contract.md](copy-contract.md) settlement 节（`release_*` / `review_batch_ok` / `endings.done` 句式）；[../design/run-story.md](../design/run-story.md)（发布幕与常驻批周期）；集成 cadence 见 AGENTS.md §2 Settle and finish（no-FF / 批次 tag / develop reset）。

## 退役终态 · retired

**场景台词（知识权威）**：本局已退役。

**阶段策略（retired_common）**：业务事件全拒；`map.show` / `requirement.list` 只读查询仍可用。

**ending 语义**：`[ENDING · run retired]` 前缀由 pack `agent.endingPrefix` 声明；结算汇总（任务、需求计数、批数、评分、演进候选、一句话教训）由 `endings.done` 键渲染，句式契约见 [copy-contract.md](copy-contract.md)。

**prompts.retired 语义**：「本局已退役：只读查询（map / requirement.list）。」

**本幕关联阅读**：[copy-contract.md](copy-contract.md)（`endings.done` 与 `retired_common` 句式）；`--settle` 结算核验见 AGENTS.md §2 Settle and finish。

## 需求级九幕进行时（requirement-acts）

主流程五幕是「局的剧情」，需求级九幕是「每条需求的剧情」——协调幕内，每条需求各自走九幕（微状态）。本节给出**按幕展开的进行时指引**：场景台词（从 `beats-requirements.json` 对应键提炼）、入口条件 / 必做动作 / 出口判据、关联文件指针。

与 [../design/requirement-story.md](../design/requirement-story.md) 的关系：**那边是机器契约**（微状态表 / 转场表 / 旁路覆盖层的判定序与副作用键），**这边是进行时指引**（我在哪一幕 / 该做什么 / 做完看什么证据）——语义不重复讲述，以那边为契约权威，本节只做执行侧台词与动作清单。

### 九幕总览

| # | 幕 | 主人 | 一句话 |
| --- | --- | --- | --- |
| 1 | 立项 | 人 | 把需求写清楚（标题/描述/依赖） |
| 2 | 计划中 | 执行者 | 产出计划稿并提交 plan 提案 |
| 3 | 计划门禁 | **人** | 审计划：过或打回 |
| 4 | 待开发 | 资源 | 计划已过，等泳道槽位派发 |
| 5 | 开发中 | 执行者 | 实现并提交 development 提案 |
| 6 | 交付受理 | 主代理 | 串行审阅交付：受理或打回 |
| 7 | 验收门禁 | **人** | 过验收：核验 develop@<sha> 证据 |
| 8 | 集成 | 执行者 | 功能分支已并入 develop，确认 SHA |
| 9 | done | — | 终态，集成证据在册 |

三道打回（交付受理 `proposal.reject` / 验收门禁 `gate.reject` / 集成 `integration.reject`）**统一落「待开发」**（plan_ok 保持、job 清空、泳道释放），打回不重走计划门禁（除非需求被 amend 使 plan_ok 失效）。

### 1. 立项（draft）

- **场景台词**（`req_added`）：`[REQ ADDED] {{id}} "{{title}}" {{deps}} — {{counts}}`——需求已立项入册，带依赖与总数。
- **入口条件**：人决定要做一件事；先过「需求粒度：能合并就先合并」判据（AGENTS.md §1.2——同目标/同子系统/共享验收证据集的相邻工作不另立新需求，新需求须在描述里写不可合并理由）。
- **必做动作**：`requirement.add "<title>|<description>"`（可带外部幂等 id 与 deps 段）；描述写清目标、改动面、验收标准。
- **出口判据**：需求在册（draft），可被 `requirement.dispatch` 派发进计划。
- **本幕关联阅读**：[copy-contract.md](copy-contract.md) requirements 节（`req_added` / `req_dispatch_usage` / `req_bad_dep` / `req_cycle_dep` 句式）。

### 2. 计划中（planning）

- **场景台词**（`req_dispatch_ok` / `req_dispatch_item`）：`[DISPATCH] {{count}}: {{items}} | jobs {{jobs}}/{{cap}}`——需求已派发给执行者，计划作业在飞。
- **入口条件**：需求 draft 且依赖已就绪（deps 全 done）、有空闲 job 槽位。
- **必做动作**：主会话 `requirement.dispatch` 派发 plan 阶段；执行者读必读材料（[dispatch-ritual.md](dispatch-ritual.md) §2）后产出计划稿，`proposal.submit(plan)` 提交（内联 `exec=<agent_id>` 注册执行者）。
- **出口判据**：plan 提案挂起（pending）→ 自动升入计划门禁（交付即门禁，无需再受理）。
- **本幕关联阅读**：[dispatch-ritual.md](dispatch-ritual.md)（brief 结构 / 工件纪律 / submit 仪式）；[copy-contract.md](copy-contract.md)（`req_prop_*` 提案句式、`req_prop_plan_gate` 提示）。

### 3. 计划门禁（plan_gate）

- **场景台词**（`req_gate_ok` kind=plan）：`[GATE PASSED · plan] {{id}} → ready | evidence: {{artifacts}} | note: {{note}}`——计划已过审，转待开发；`req_accept_plan` 提示「plan 提案的裁决走 gate.approve/reject（交付即门禁），proposal.accept 只受理 development 提案」。
- **入口条件**：plan 提案 pending（`gate=="plan"`）。
- **必做动作**：**人**审计划稿，`gate.approve`（过）或 `gate.reject`（打回，须新计划）；guide 模式下只能由人从菜单点选（人机门禁，见协调幕 `guide_human_blocked`）。
- **出口判据**：approve → 待开发（`plan_ok=true`，同回可自动派发开发）；reject → 回计划中（`plan_ok=false`，须重新出计划）。
- **本幕关联阅读**：[copy-contract.md](copy-contract.md)（`req_gate_*` 拒绝面：own_props/own_job/stale/no_note 等）；[../design/state-surface.md](../design/state-surface.md)（`gate_rev==rev` stale 校验）。

### 4. 待开发（ready）

- **场景台词**（`req_dispatch_noop_blocked` / `req_dispatch_noop_none`）：`[DISPATCH] 0 candidates: jobs {{jobs}}/{{cap}} is full, or development candidates are not ready / lanes are taken ({{lanes}})`——计划已过但暂未派发（槽位或泳道占用）。
- **入口条件**：`plan_ok=true`、无 job、无 pending。
- **必做动作**：无主动作——等资源（job 槽位 / 泳道）；主会话可在槽位空出时 `requirement.dispatch` 派发开发。自动派发（declared single successor）在同回无 pending、无冲突、有空槽时触发。
- **出口判据**：被派发进开发（认领泳道，job=development）。
- **本幕关联阅读**：[../design/lanes.md](../design/lanes.md)（泳道池认领/释放）；[copy-contract.md](copy-contract.md) lanes 节（`lane_claimed` / `lane_claim_*` 拒绝句式）。

### 5. 开发中（developing）

- **场景台词**（`req_dispatch_item` stage=development / `req_amend_active`）：需求已认领泳道进入开发；中途修订 `req_amend_active`：「{{id}} is developing: title/description updated → rev {{rev}} (progress kept)」——开发中修订只改进度描述、不动 deps。
- **入口条件**：ready 且被 dispatch（泳道认领成功）。
- **必做动作**：执行者按 brief 实现；plan 阶段声明的 wiki 触碰面在同特性分支就地更新（Doc sync）；完成后 `proposal.submit(development)` 提交，工件带验收证据（集成需求须 `develop@<sha>`）。
- **出口判据**：development 提案 pending → 交付受理幕。
- **本幕关联阅读**：[dispatch-ritual.md](dispatch-ritual.md)（工件纪律 / submit / Doc sync / 集成纪律）；[copy-contract.md](copy-contract.md)（`req_amend_*` / `req_prop_*` 句式）。

### 6. 交付受理（accept）

- **场景台词**（`req_accept_ok`）：`[PROPOSAL ACCEPTED] {{id}} → {{req}} / {{stage}}, state → {{status}}{{change}} | {{pending}} still pending`——主代理已受理交付；`req_accept_stale`：提案 rev 与需求 rev 不符时判 stale，须 rebase 重交。
- **入口条件**：有 pending development 提案。
- **必做动作**：**主代理串行**审阅交付（一次一条，canonical order）——`proposal.accept`（受理，升入验收门禁）或 `proposal.reject`（打回待开发）。受理前核验工件齐备、证据可读。
- **出口判据**：accept → 验收门禁（gate=accept）；reject → 待开发（plan_ok 保持，重新开发）。
- **本幕关联阅读**：[copy-contract.md](copy-contract.md)（`req_accept_*` / `req_prop_busy` / `req_prop_no_summary` 句式）；受理纪律见 AGENTS.md §1.2（串行受理）。

### 7. 验收门禁（accept_gate）

- **场景台词**（`req_gate_ok` kind=accept）：`[GATE PASSED · accept] {{id}} → integrating | evidence: {{artifacts}}`——验收已过，转集成；`req_gate_no_evidence`：artifacts 为空时拒绝（summary-only 声明不受理）。
- **入口条件**：development 提案已受理（`gate=="accept"`）。
- **必做动作**：**人**过验收——`gate.approve`（须核验 `develop@<sha>` 证据与 artifacts 非空）或 `gate.reject`（打回待开发，plan_ok 保持）；gate_rev==req.rev 的 stale 校验同样适用。
- **出口判据**：approve → 集成幕（`integrating=true`）；reject → 待开发。
- **本幕关联阅读**：[copy-contract.md](copy-contract.md)（`req_gate_ok` / `req_gate_no_artifacts` / `req_gate_no_evidence` / `req_gate_stale` 句式）；[../design/state-surface.md](../design/state-surface.md)（验收证据键）。

### 8. 集成（integrating）

- **场景台词**（`integration_confirmed`）：`[INTEGRATION CONFIRMED] {{id}} develop@{{sha}} 已核验 → 完成（已终态 {{done}} 条）`；`integration_rejected`：集成打回 → 待开发，理由在册。
- **入口条件**：验收门禁通过（`integrating=true`）。
- **必做动作**：**执行者**在收口时自行集成功能分支 → `origin/develop`（对齐 → `git merge --no-ff` → 集成态门禁绿 → push develop），工件记录 `develop@<sha>`；主会话 `integration.confirm` 核验 SHA 后关需求，或 `integration.reject` 打回。
- **出口判据**：confirm → done（`status=done`，泳道释放）；reject → 待开发（写 feedback、释放泳道）。
- **本幕关联阅读**：[dispatch-ritual.md](dispatch-ritual.md) §5（集成纪律：no-FF / 不碰 master / 不碰他人泳道）；[copy-contract.md](copy-contract.md)（`integration_*` 句式、`integration_bad_sha` 证据形状）。

### 9. done（终态）

- **场景台词**（`integration_confirmed` 尾句 / `req_amend_derived`）：需求已终态；对 done 需求再修订会派生新需求：`[REQ DERIVED] {{id}} is done — the amend derives a new requirement {{new}} "{{title}}" (origin={{id}}, rev {{rev}})`。
- **入口条件**：`integration.confirm` 成功；旁路关闭（split/merge/dropped）时为 `closed_reason`。
- **必做动作**：无（终态）；泳道已释放，证据在册。
- **出口判据**：无（终态）。
- **本幕关联阅读**：[copy-contract.md](copy-contract.md)（`req_amend_derive_hist` / `req_after_done` / `req_resolve_*` 句式）；旁路语义见 [../design/requirement-story.md](../design/requirement-story.md)（覆盖层：split/merge/conflict/resolve）。

### 旁路与覆盖层（进行时提示）

- **amend**：递增 rev 并使 plan_ok 失效——门禁挂起时 stale、阻塞 approve；done 上 amend 派生新需求。
- **split / merge**：仅开发前且无活动任务/门禁/泳道/冲突的需求可做；父/源需求落 done + closed_reason。
- **conflict / resolve**：两需求间双向冲突阻塞派发与收口，直到 `requirement.resolve`（keep_both / keep_first / revise）。
- `blocked`（冲突 / needs_amend）是显示覆盖层，不是微状态——判微状态仍以上面九幕为准。

## 无 egg 执行本章（无 mod 降级路径）

不安装 egg / mod 时，本工作流仍可运行——**context 树（含本文）就是完整的工作流底座**：

1. **流程语义**：按本文五幕顺序与每幕「入口/必做/出口」人工执行；需求九幕与两道门禁的语义不变（立项 → 计划 → 计划门禁 → 开发 → 交付受理 → 验收门禁 → 集成 → 完成）。
2. **动作名的落地**：`charter.set` = 在会话中写下章程三件套并确认；`requirement.add/dispatch` = 维护需求清单（标题/描述/依赖/状态），按就绪序派发子代理；`proposal.accept` / `gate.approve` = 主会话逐条审阅子代理交付并明示受理/过门禁；`coordinate.submit` = 宣告本批需求全部终态；`verify.submit` = 记录本批验证证据；`settlement.*` = 写下演进登记、会话清理、待办清理三项声明；`release.confirm/skip` = 记录合并 SHA 与批次 tag；`review.note` = 写复盘（lesson|score）。
3. **文案行为可预知**：每个动作「会说什么、拒绝时说什么」查 [copy-contract.md](copy-contract.md)（句式模板 + 协议前缀归属）；派发仪式查 [dispatch-ritual.md](dispatch-ritual.md)；清理政策查 [cleanup-policy.md](cleanup-policy.md)。
4. **三个缺口（明示，不伪装）**：无 egg 时缺少的是——① **工具监控状态**（没有状态机自动判定微状态/就绪序/拒绝面，由主会话自律核对）；② **记录历史**（没有事件日志与自动存档，历史由会话 transcript 与 git 承担）；③ **强制流程合规**（门禁不再由机器 fail-closed 拦截，主会话必须按本文与 AGENTS.md 自律执行，评审者按同一清单核验）。
5. **禁止降级的项**：即使无 egg，「派发 → 提案 → 两道门禁 → 批结算」的顺序与「每步真实动作 + 核验」的证据标准**不降低**——降级的是工具，不是纪律。
6. **需求级九幕的人工执行**：无 egg 时每条需求同样走九幕，由主会话用一张需求清单手工跟踪——每行记「需求 id / 标题 / 依赖 / 当前幕 / 泳道分支 / 证据链接」。逐幕落地：① 立项 = 写清标题/描述/依赖并过合并判据；② 计划中 = 派子代理出计划稿并收回；③ 计划门禁 = 主会话（或人）明示「计划过/打回」；④ 待开发 = 计划已过、等泳道空位（无机器调度，主会话自律排队）；⑤ 开发中 = 子代理实现并交回工件（含 develop@<sha>，集成需求）；⑥ 交付受理 = 主会话逐条审阅并明示「受理/打回」；⑦ 验收门禁 = 人核验证据后明示「验收过/打回」；⑧ 集成 = 执行者已 no-FF 并入 develop，主会话核验 SHA 后标记完成；⑨ done = 终态入册。三道打回统一回「待开发」；rev stale 概念退化为「交付是否基于最新需求描述」的人工核对。每幕该说什么、拒绝时说什么仍查 [copy-contract.md](copy-contract.md)，各幕动作与证据以本文「需求级九幕进行时」为准——缺的只是机器的微状态判定、事件日志与 fail-closed 拦截。
