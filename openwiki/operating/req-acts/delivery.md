---
type: "Reference"
title: "需求级九幕 · 交付（ready → developing → accept → accept_gate）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-e05e3253cc972e8bd548b5cf
    resource: repo://.github/workflows/check.yml
  - id: openwiki-source-20786f1cdc5649019aa1a3b9
    resource: repo://.ygg/mods/agent-seed/src/domain/gates.lua
  - id: openwiki-source-330b7a469ba8c72568035b32
    resource: repo://.ygg/mods/agent-seed/src/domain/proposals.lua
  - id: openwiki-source-1a8a660d43047137a9cf72f5
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/proposals.lua
  - id: openwiki-source-b28cbd2e7e58503c61a79e3b
    resource: repo://.ygg/mods/agent-seed/tests/gates.lua
  - id: openwiki-source-6a5d7b301b84949a383f444c
    resource: repo://scripts/build-wasm.sh
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 需求级九幕 · 交付（ready → developing → accept → accept_gate）

> **id** req-acts/delivery · **幕** req: ready / developing / accept / accept_gate · **触发** 派发开发 / 交付受理前
> **读不到代价** hard-refusal（工件纪律漏读 → 交付被打回；证据口径漏读 → 绿而不可玩）
> **replaces**：`docs/context/req-acts/delivery.md`（搬运基线 `3a2536f`）
> **上游**：[../../architecture/README.md](../../architecture/README.md) · **相关**：[intake.md](intake.md) · [integration.md](integration.md) · [../rituals/dispatch.md](../rituals/dispatch.md)

## 4. 待开发（ready）

- **台词**（`req_dispatch_noop_blocked` / `req_dispatch_noop_none`）：`[DISPATCH] 0 candidates: jobs {{jobs}}/{{cap}} is full, or development candidates are not ready / lanes are taken ({{lanes}})`。
- **入口条件**：`plan_ok=true`、无 job、无 pending。
- **必做动作**：无主动作——等资源（job 槽位 / 泳道）；槽位空出时主会话可 `requirement.dispatch` 派开发。自动派发（declared single successor）在同回无 pending、无冲突、有空槽时触发。
- **出口判据**：被派进开发（认领泳道，`job=development`）。
- **本幕阅读**：[../../contracts/host/lanes.md](../../contracts/host/lanes.md)；[../scenes/README.md](../scenes/README.md)（`lane_claimed` / `lane_claim_*`）。

## 5. 开发中（developing）

- **台词**（`req_dispatch_item` stage=development / `req_amend_active`）：开发中修订只改进度描述、不动 deps：「{{id}} is developing: title/description updated → rev {{rev}} (progress kept)」。
- **必做动作**：执行者按 brief 实现；plan 阶段声明的知识库触碰面在同一特性分支就地更新（Doc sync）；完成后 `proposal.submit(development)`，工件带验收证据（集成需求须 `develop@<sha>`）。
- **出口判据**：development 提案 pending → 交付受理幕。
- **本幕阅读**：[../rituals/dispatch.md](../rituals/dispatch.md)（工件纪律 / submit / Doc sync / 集成纪律）。

### 文档同步（Doc sync，适用时）

`openwiki/` 是全仓始终最新的权威——不是变更日志（历史归 git 与结算产物）。若变更改变任何子系统的工作方式（内核 crates / egg CLI / Mod / serve / 工具链）：**plan 阶段声明触碰面**，**development 阶段同一分支就地更新**（决策 / 词汇 → `operating/` 对应页；行为 / 契约 → `contracts/` 对应页；幕台词 / 仪式 / 文案句式 → `operating/{acts,rituals,scenes}/`）。纯缺陷修复与措辞调整不触碰知识库。

**归属边界**：本节说的是**你自己这次改动**的 Doc sync。**跨批次的收口**（回看上批全部需求、把散落决策归位）**不派给 subagent，由主会话自办**——批结算在 `docs.maintenance.due` 登记该义务并阻塞下一次 `coordinate.submit`（见 [../../governance.md](../../governance.md) §6）。判据：**你改的东西你知道，所以同分支更新；跨批次的东西只有主会话有上下文，所以归主会话。**

## 6. 交付受理（accept）

- **台词**（`req_accept_ok`）：`[PROPOSAL ACCEPTED] {{id}} → {{req}} / {{stage}}, state → {{status}}{{change}} | {{pending}} still pending`；`req_accept_stale`：提案 rev 与需求 rev 不符即 stale，须 rebase 重交。
- **必做动作**：**主代理串行**审阅（一次一条，canonical order）——`proposal.accept` 或 `proposal.reject`。受理前核验工件齐备、证据可读。
- **出口判据**：accept → 验收门禁（`gate=accept`）；reject → 待开发（`plan_ok` 保持）。

## 6b. 验证门禁与证据（gates · live · evidence）

**基线是快照，验收标准是「不引入新失败」**：历史欠账不必为零，但不允许出现新的失败。

### 三件套：字面证据才算绿

三件套绿**只能**以逐字运行、记录真实退出码与用例数的证据声称：

```bash
cargo test --workspace
cargo test --example egg -p ygg-sandbox
cargo run -q --example selftest -p ygg-sandbox -- --mod .ygg/mods/agent-seed
```

`--test selftest` 目标**不存在**（实测 `exit 101`）。成员等价跑法（`cargo check`、单测选择、过滤跑）、`verify-suite --mode fast`、任何 `YGG_VERIFY_COMMANDS` 覆盖**只能证明那个替代物**，永不算三件套绿。

### Mod 语义矩阵

改 `.ygg/mods/agent-seed/src/**`、`mod.lua` 或地图措辞时，除 lint 与 all-1 playtest 外**追加** `selftest`（当前 96 例）。**all-1 playtest 只覆盖 happy path**——从不走 `gate.reject` / `requirement.amend` / `proposal.reject`，也看不到回合切割；`/tmp` 下的临时探针只能补充、永不替代；未覆盖场景必须**逐条声明**。

三问判据：① 挡住「改了 Mod 源码或地图措辞，只跑 lint + all-1 就称验收」；② 症状是分支语义与措辞静默回退而门禁全绿，缺陷只在真实跑局现形；③ 下次确认——该命令加一次变异探针（把旧语义放回去 → 必须转红），证明断言非空转。

### 四条工程硬约束

- **core 纯度**：`ygg-core` 的依赖清单属评审范围——出现渲染 / 平台 / IO 依赖即违规，**无例外机制**。
- **未接线必须声明**：门禁尚未接上（还没有测试）的模块，报告必须写「该门禁未覆盖」，**永不默认通过**。
- **wasm 构建禁用 wasm-pack**：异常处理模块需要 `wasm-bindgen --reference-types`，wasm-pack 不透传；构建链固定 `scripts/build-wasm.sh`（cargo + wasm-bindgen CLI 两步）。
- **契约一致性**：同一份 Mod 样本经 ajv 与 jsonschema 必须得到相同结果；该 CI 脚本落地前，这一面按上一条声明为未覆盖。

### 双跑基线与 FR-9

- **绝不静默改基线**：双跑 state hash 不一致时不得就地改 fixture——不一致本身就是跨平台确定性破裂；改基线需显式决定并留记录。
- **声明完全合规前**先补 wasm-bindgen-test 的**浏览器 lane**（真实浏览器 wasm 实例里重跑同一组断言），或取一份显式豁免决定；当前 node lane 覆盖 native/wasm 双跑等价，浏览器 lane 不存在。

### 证据规范

- **门禁绿不等于可玩**：单元门禁只证明它检查了的那件事；「可玩」是另一类断言，需要另一类证据。
- **分层列证**：迭代级（内核改动走内核链；表现层改动要真实浏览器走查 + 截图）；轮次级（整批合并后在 `develop` 集成态完整跑通示例游戏）。
- **证据锚在形态上**：命令 → 输出 + 退出码；工件 → 存在性 + 关键内容；源码 → `file:line`。
- **不凭印象**：不写「已验证 / 已完成」这类无证据完成句；不确定就说不确定，并写清下一步怎么验。

## 7. 旁路与覆盖层

- **amend**：递增 rev 并使 `plan_ok` 失效——门禁挂起时 stale、阻塞 approve；done 上 amend 派生新需求。
- **split / merge**：仅开发前且无活动任务/门禁/泳道/冲突的需求可做；父/源需求落 done + `closed_reason`。
- **conflict / resolve**：两需求双向冲突阻塞派发与收口，直到 `requirement.resolve`（keep_both / keep_first / revise）。
- `blocked`（冲突 / needs_amend）是显示覆盖层，不是微状态——判微状态仍以九幕为准。

## 8. 判据

① 阻断——用成员等价跑法冒充三件套绿、改了 Mod 语义只跑 lint + all-1、把跨批次收口派给 subagent、静默改双跑基线。② 症状——报告写「已验证」却无退出码、`--test selftest` 报 `exit 101`、`docs.maintenance.due` 在结算时才被想起。③ 确认——三件套三条命令的退出码与用例数逐条贴出、变异探针转红一次、`docs/design` 触碰面在 plan 阶段被点名。

维护责任 = 改门禁命令、证据口径或 Doc sync 归属的执行者；触发 = 门禁脚本/用例数变化、证据规范变化；检查 = 本页命令与 `scripts/`、`.github/workflows/check.yml` 逐字一致，用例数与最近一次实跑一致。
