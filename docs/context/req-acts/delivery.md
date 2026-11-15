# 需求级九幕进行时 · 交付（req-acts/delivery）
> **id** req-acts/delivery · **幕** req: ready/developing/accept/accept_gate · **触发** 派发 development / 交付受理前
> **读不到代价** hard-refusal（工件纪律漏读 → 交付被打回；`.env.local` 未 source → 派发不合规）
> **ver** de0268

## 4. 待开发（ready）

- **场景台词**（`req_dispatch_noop_blocked` / `req_dispatch_noop_none`）：`[DISPATCH] 0 candidates: jobs {{jobs}}/{{cap}} is full, or development candidates are not ready / lanes are taken ({{lanes}})`——计划已过但暂未派发（槽位或泳道占用）。
- **入口条件**：`plan_ok=true`、无 job、无 pending。
- **必做动作**：无主动作——等资源（job 槽位 / 泳道）；主会话可在槽位空出时 `requirement.dispatch` 派发开发。自动派发（declared single successor）在同回无 pending、无冲突、有空槽时触发。
- **出口判据**：被派发进开发（认领泳道，job=development）。
- **本幕关联阅读**：[../../design/host/lanes.md](../../design/host/lanes.md)（泳道池认领/释放）；[copy/lanes.md](../scenes/lanes.md)（`lane_claimed` / `lane_claim_*` 拒绝句式）。

## 5. 开发中（developing）

- **场景台词**（`req_dispatch_item` stage=development / `req_amend_active`）：需求已认领泳道进入开发；中途修订 `req_amend_active`：「{{id}} is developing: title/description updated → rev {{rev}} (progress kept)」——开发中修订只改进度描述、不动 deps。
- **入口条件**：ready 且被 dispatch（泳道认领成功）。
- **必做动作**：执行者按 brief 实现；plan 阶段声明的 wiki 触碰面在同特性分支就地更新（Doc sync）；完成后 `proposal.submit(development)` 提交，工件带验收证据（集成需求须 `develop@<sha>`）。
- **出口判据**：development 提案 pending → 交付受理幕。
- **本幕关联阅读**：[rituals/dispatch.md](../rituals/dispatch.md)（工件纪律 / submit / Doc sync / 集成纪律）；[copy/requirements.md](../scenes/requirements.md)（`req_amend_*` / `req_prop_*` 句式）。

## 5b. 文档同步（Doc sync，适用时）

docs wiki 是全仓始终最新的权威——不是变更日志，历史归 git 与结算产物。若变更改变任何子系统的工作方式（内核 crates / egg CLI / Mod / serve / 工具链）：plan 阶段声明触碰面；development 阶段在同一特性分支就地更新对应文件（决策/词汇 → `docs/context/` 对应台账行或词条；行为/契约 → `docs/design/` 对应主题文件；幕台词/仪式/文案句式 → `docs/context/` 的 [acts/](../acts/) / [rituals/dispatch.md](../rituals/dispatch.md) / [copy/README.md](../scenes/README.md) 七域索引）。纯缺陷修复与措辞调整不触碰文档。**归属边界（用户 2026-11-15 裁定）**：本节说的是**你自己这次改动**的 Doc sync——就地在同一分支更新你改动所影响的文件。**跨批次的 docs wiki 收口**（回看上批全部需求、把散落的决策/词汇/契约归位）**不派给 subagent，由主会话自办**：批结算会在 `docs.maintenance.due` 登记该义务并阻塞下一次 `coordinate.submit`（见 [../../wiki-governance.md](../../wiki-governance.md) §6）。判据很简单：**你改的东西你知道，所以同分支更新**；**跨批次的东西只有主会话有上下文，所以归主会话**。

## 6. 交付受理（accept）

- **场景台词**（`req_accept_ok`）：`[PROPOSAL ACCEPTED] {{id}} → {{req}} / {{stage}}, state → {{status}}{{change}} | {{pending}} still pending`——主代理已受理交付；`req_accept_stale`：提案 rev 与需求 rev 不符时判 stale，须 rebase 重交。
- **入口条件**：有 pending development 提案。
- **必做动作**：**主代理串行**审阅交付（一次一条，canonical order）——`proposal.accept`（受理，升入验收门禁）或 `proposal.reject`（打回待开发）。受理前核验工件齐备、证据可读。
- **出口判据**：accept → 验收门禁（gate=accept）；reject → 待开发（plan_ok 保持，重新开发）。
- **本幕关联阅读**：[copy/requirements.md](../scenes/requirements.md)（`req_accept_*` / `req_prop_busy` / `req_prop_no_summary` 句式）；串行受理判据见 [acts/coordinate.md](../acts/coordinate.md)。

## 5c. 验证门禁与证据（gates · live · evidence）

**基线是快照，验收标准是「不引入新失败」**：历史欠账不必为零，但不允许出现新的失败。

### 三件套：字面证据才算绿

本仓的三件套绿**只能**以逐字运行并记录真实退出码与用例数的证据声称：`cargo test --workspace` ∧ `cargo test --example egg -p ygg-sandbox` ∧ `cargo test -p ygg-sandbox --test agent_seed_semantics`。成员等价跑法（`cargo check`、单测选择、过滤跑）、`verify-suite --mode fast`、任何 `YGG_VERIFY_COMMANDS` 覆盖，**只能证明那个替代物**，永远不算三件套绿。

### Mod 语义矩阵

改动 `.ygg/mods/agent-seed/src/**`（状态机 / 门禁 / 渲染层）、`mod.lua` 或地图措辞时，门禁清单在 lint 与 all-1 playtest 之外**追加** `cargo test -p ygg-sandbox --test agent_seed_semantics`。**all-1 playtest 只覆盖 happy path**——它从不走 `gate.reject` / `requirement.amend` / `proposal.reject`，也看不到回合切割机制；`/tmp` 下的临时探针只能补充、永不替代；每个未覆盖场景必须在报告里**逐条声明**。

三问判据：① 它挡住「改了 Mod 源码或地图措辞，只跑 lint + all-1 就称验收」；② 违规的可见症状是分支语义与地图措辞静默回退而所有门禁仍绿，缺陷只在真实跑局里才现形；③ 下次怎么确认——该命令（当前 96 例）加一次变异探针（把旧语义放回去 → 必须转红），证明断言非空转。

### 四条工程硬约束

- **core 纯度**：`ygg-core` 的 `Cargo.toml` 依赖清单属于评审范围——出现渲染 / 平台 / IO 依赖即为违规，**无例外机制**。
- **未接线必须声明**：门禁尚未接上（还没有测试）的模块，报告里必须写明「该门禁未覆盖」，**永不默认通过**。
- **wasm 构建禁用 wasm-pack**：异常处理模块需要 `wasm-bindgen --reference-types`，wasm-pack 不透传这个 flag；构建链固定为 `scripts/build-wasm.sh`（cargo + wasm-bindgen CLI 两步）。
- **契约一致性**：同一份 Mod 样本经 ajv 与 jsonschema crate 必须得到相同结果；该 CI 脚本尚未落地前，这一面按上一条声明为未覆盖。

### FR-9 与双跑基线

- **绝不静默改基线**：双跑 state hash 不一致时，不得就地改 fixture 基线——不一致本身就是跨平台的确定性破裂；改基线需显式决定并留记录。
- **声明完全合规前**先补 wasm-bindgen-test 的**浏览器 lane**（在真实浏览器 wasm 实例里重跑同一组断言），或取一份显式豁免决定；当前 node lane 覆盖 native/wasm 双跑等价，浏览器 lane 不存在。

### 证据规范（live · evidence）

- **门禁绿不等于可玩**：单元门禁只能证明它检查了的那件事。「游戏可玩」是另一类断言，需要另一类证据。
- **分层列证**：迭代级（内核改动走内核链；表现层改动要真实浏览器走查 + 截图）；轮次级（整批合并后在 `develop` 集成态把示例游戏完整跑通）。
- **证据锚在形态上**：命令 → 输出 + 退出码；工件 → 存在性 + 关键内容；源码 → `file:line`。
- **不凭印象**：不写「已验证 / 已完成」这类无证据完成句；不确定就说不确定，并写清下一步怎么验。



## 旁路与覆盖层（进行时提示）

- **amend**：递增 rev 并使 plan_ok 失效——门禁挂起时 stale、阻塞 approve；done 上 amend 派生新需求。
- **split / merge**：仅开发前且无活动任务/门禁/泳道/冲突的需求可做；父/源需求落 done + closed_reason。
- **conflict / resolve**：两需求间双向冲突阻塞派发与收口，直到 `requirement.resolve`（keep_both / keep_first / revise）。
- `blocked`（冲突 / needs_amend）是显示覆盖层，不是微状态——判微状态仍以上面九幕为准（九幕一览见 [README.md](../README.md)）。
