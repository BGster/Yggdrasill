# 需求级九幕进行时 · 交付（req-acts/delivery）
> **id** req-acts/delivery · **幕** req: ready/developing/accept/accept_gate · **触发** 派发 development / 交付受理前
> **读不到代价** hard-refusal（工件纪律漏读 → 交付被打回；`.env.local` 未 source → 派发不合规）
> **ver** f95112

## 4. 待开发（ready）

- **场景台词**（`req_dispatch_noop_blocked` / `req_dispatch_noop_none`）：`[DISPATCH] 0 candidates: jobs {{jobs}}/{{cap}} is full, or development candidates are not ready / lanes are taken ({{lanes}})`——计划已过但暂未派发（槽位或泳道占用）。
- **入口条件**：`plan_ok=true`、无 job、无 pending。
- **必做动作**：无主动作——等资源（job 槽位 / 泳道）；主会话可在槽位空出时 `requirement.dispatch` 派发开发。自动派发（declared single successor）在同回无 pending、无冲突、有空槽时触发。
- **出口判据**：被派发进开发（认领泳道，job=development）。
- **本幕关联阅读**：[../../design/lanes.md](../../design/lanes.md)（泳道池认领/释放）；[copy/lanes.md](../copy/lanes.md)（`lane_claimed` / `lane_claim_*` 拒绝句式）。

## 5. 开发中（developing）

- **场景台词**（`req_dispatch_item` stage=development / `req_amend_active`）：需求已认领泳道进入开发；中途修订 `req_amend_active`：「{{id}} is developing: title/description updated → rev {{rev}} (progress kept)」——开发中修订只改进度描述、不动 deps。
- **入口条件**：ready 且被 dispatch（泳道认领成功）。
- **必做动作**：执行者按 brief 实现；plan 阶段声明的 wiki 触碰面在同特性分支就地更新（Doc sync）；完成后 `proposal.submit(development)` 提交，工件带验收证据（集成需求须 `develop@<sha>`）。
- **出口判据**：development 提案 pending → 交付受理幕。
- **本幕关联阅读**：[rituals/dispatch.md](../rituals/dispatch.md)（工件纪律 / submit / Doc sync / 集成纪律）；[copy/requirements.md](../copy/requirements.md)（`req_amend_*` / `req_prop_*` 句式）。

## 5b. 文档同步（Doc sync，适用时）

docs wiki 是全仓始终最新的权威——不是变更日志，历史归 git 与结算产物。若变更改变任何子系统的工作方式（内核 crates / egg CLI / Mod / serve / 工具链）：plan 阶段声明触碰面；development 阶段在同一特性分支就地更新对应文件（决策/词汇 → `docs/context/` 对应台账行或词条；行为/契约 → `docs/design/` 对应主题文件；幕台词/仪式/文案句式 → `docs/context/` 的 [acts/](../acts/) / [rituals/dispatch.md](../rituals/dispatch.md) / [copy/README.md](../copy/README.md) 七域索引）。纯缺陷修复与措辞调整不触碰文档。

## 6. 交付受理（accept）

- **场景台词**（`req_accept_ok`）：`[PROPOSAL ACCEPTED] {{id}} → {{req}} / {{stage}}, state → {{status}}{{change}} | {{pending}} still pending`——主代理已受理交付；`req_accept_stale`：提案 rev 与需求 rev 不符时判 stale，须 rebase 重交。
- **入口条件**：有 pending development 提案。
- **必做动作**：**主代理串行**审阅交付（一次一条，canonical order）——`proposal.accept`（受理，升入验收门禁）或 `proposal.reject`（打回待开发）。受理前核验工件齐备、证据可读。
- **出口判据**：accept → 验收门禁（gate=accept）；reject → 待开发（plan_ok 保持，重新开发）。
- **本幕关联阅读**：[copy/requirements.md](../copy/requirements.md)（`req_accept_*` / `req_prop_busy` / `req_prop_no_summary` 句式）；串行受理判据见 [acts/coordinate.md](../acts/coordinate.md)。

## 旁路与覆盖层（进行时提示）

- **amend**：递增 rev 并使 plan_ok 失效——门禁挂起时 stale、阻塞 approve；done 上 amend 派生新需求。
- **split / merge**：仅开发前且无活动任务/门禁/泳道/冲突的需求可做；父/源需求落 done + closed_reason。
- **conflict / resolve**：两需求间双向冲突阻塞派发与收口，直到 `requirement.resolve`（keep_both / keep_first / revise）。
- `blocked`（冲突 / needs_amend）是显示覆盖层，不是微状态——判微状态仍以上面九幕为准（九幕一览见 [README.md](../README.md)）。
