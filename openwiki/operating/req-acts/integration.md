---
type: "Reference"
title: "需求级九幕 · 集成（accept_gate → integrating → done）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-96948bd19535ffaa7f38e1cd
    resource: repo://.ygg/mods/agent-seed/src/domain/lanes.lua
  - id: openwiki-source-d57e21e5bbbce0b80489fd49
    resource: repo://.ygg/mods/agent-seed/src/domain/requirements.lua
  - id: openwiki-source-1687a5fc04658c13a78d4590
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/gate_actions.lua
  - id: openwiki-source-3eb9b045e8a8eca336331ac7
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/integration.lua
  - id: openwiki-source-b28cbd2e7e58503c61a79e3b
    resource: repo://.ygg/mods/agent-seed/tests/gates.lua
  - id: openwiki-source-55ac8b91f1869ef9305b6b62
    resource: repo://crates/ygg-sandbox/src/host_contract/settlement.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 需求级九幕 · 集成（accept_gate → integrating → done）

> **id** req-acts/integration · **幕** req: accept_gate / integrating / done · **触发** 验收门禁前 / 集成核验前
> **读不到代价** hard-refusal（证据清单漏项 → 验收被拒；SHA 不符 → `integration_bad_sha` 打回）
> **replaces**：`docs/context/req-acts/integration.md`（搬运基线 `3a2536f`）
> **上游**：[../../architecture/README.md](../../architecture/README.md) · **相关**：[delivery.md](delivery.md) · [../rituals/dispatch.md](../rituals/dispatch.md) · [../acts/release.md](../acts/release.md)

## 7. 验收门禁（accept_gate）

- **台词**（`req_gate_ok` kind=accept）：`[GATE PASSED · accept] {{id}} → integrating | evidence: {{artifacts}}`；`req_gate_no_evidence`：artifacts 为空即拒（**summary-only 声明不受理**）。
- **入口条件**：development 提案已受理（`gate=="accept"`）。
- **必做动作**：**人**过验收——`gate.approve`（须核验 `develop@<sha>` 与 artifacts 非空）或 `gate.reject`（打回待开发，`plan_ok` 保持）；`gate_rev==req.rev` 的 stale 校验同样适用。
- **出口判据**：approve → 集成幕（`integrating=true`）；reject → 待开发。

### 场景化证据清单（按需求类型选列核验）

**通用子句（对所有行生效）**：证据只度量「代码存在」不合格。凡引入 / 依赖**开关型、feature-flag 型**能力（「缺席即失效 / 关掉即失效」），必须追加「**在真实配置下处于生效态**」的证据：`data/config.json` 的生效值 + 对应 `*_ships_armed_*` 用例名与结果。历史代价：某版三套门禁 473/158/77 全绿，而 `readGate` / `readPlan` 根本不存在。**schema 只校形状，不校值域**：可选段可以整段缺席，必填键可以是 `false`（schema 合法但保护静默失效）——两种形态都只能靠 armed 断言钉住。

| 需求类型 | 必须亲测的证据 |
| --- | --- |
| **交付洁净度（所有类型必核）** | 执行者回执中 `git-delivery-check.sh --text` 原文行：`ok=1` 且 `dirty` / `unpushed` / `stash` / `conflict_debris` / `fmt` 五项计数全 0；`n/a` / `waived` 项须附理由。**只有「工作树 clean」字样、没有这一行的，按证据缺失不受理** |
| **开关型 / feature-flag** | ① `data/config.json` 实测生效（非空表 / 布尔 `true`，不是「键在这里」）；② 同分支 `*_ships_armed_*` 语义用例存在且全绿（贴用例名）；③ 关掉该配置的副本仍走退让路径。典型对象：`readPlan` / `readGate`（可选段，缺席即 inert）、`gates.require_all_green`（required，但 `false` 即静默失效） |
| 跨仓库同步 | 源↔目标 `diff -rq` 为空；二进制 `sha256` 双侧一致；目标仓 boot 三件套实测；知识库闭包 ok=1；数据红线零触碰；子模块指针未动 |
| Mod 代码 / 文案 | `scripts/sync-seed-mod.sh --check`；`cargo run -p ygg-lint -- --pack .ygg/mods/agent-seed`（期望 `0 error(s), 0 warning(s)`）；`cargo run -q --example selftest -p ygg-sandbox -- --mod .ygg/mods/agent-seed` 全绿（含新增用例）；`cargo run --example playtest -- .ygg/mods/agent-seed` 的 all-1 走到 `[ENDING` |
| 宿主 crates | 字面三件套（`cargo test --workspace` ∧ `--example egg` ∧ `selftest`），记录真实退出码与计数 |
| 纯文档 / 规则 | `wiki-consistency` ok=1；受影响的键数 / 枚举实测对齐；语义测试不变红 |
| 需求图 / 台账 | `requirement.list` 终态数对得上；分支模型 / 注册项存在性实测 |

四问：① 本需求属于哪类？② 该类每项证据跑了吗（贴末行 / 输出）？③ 没跑的那项为何不适用（写明）？④ **有没有把「已存在」错当成「已生效」？**

## 8. 集成（integrating）

- **台词**（`integration_confirmed`）：`[INTEGRATION CONFIRMED] {{id}} develop@{{sha}} 已核验 → 完成（已终态 {{done}} 条）`；`integration_rejected`：打回 → 待开发，理由在册。
- **必做动作**：**执行者**在收口时自行集成功能分支 → `origin/develop`（对齐 → `git merge --no-ff` → 集成态门禁绿 → push develop），工件记录 `develop@<sha>`；主会话 `integration.confirm` 核验 SHA 后关需求，或 `integration.reject` 打回。
- **出口判据**：confirm → done（`status=done`，泳道释放）；reject → 待开发（写 feedback、释放泳道）。

### 集成纪律（development 收口时全流程）

1. **谁做**：该需求的 development 执行者在**自己的泳道**内完成；主会话永不做迭代级集成。集成按仓库经 `origin/develop` 串行化——被拒的 develop push 说明别的泳道先落地，重新对齐后重试；栈叠泳道按 base-first 顺序集成。
2. **准入**：功能分支自身门禁全绿才能开始；这份绿**不结转**，集成后必须在集成态重新验证。
3. `git fetch origin`。
4. **对齐**：先查本地 `develop` 与 `origin/develop` 是否已分叉——分叉即停，分析后报告用户，**不自动解决**；未分叉则 `git pull origin develop --ff-only`。
5. **合并**：`git merge --no-ff <feature>`（一切分支合并一律 no-FF，禁止 fast-forward）。只合并**本地** ref，**绝不合并 `origin/feature/*`**——远端 ref 可能指向旧头，会造出重复提交与孤立合并节点。
6. **集成态再验证**：在集成工作树内证明集成态（不复用分支那份绿），通过后才 push `develop`。
7. **收口判据**：只 push 功能分支不算收口——需求只有合入 `develop` 且在集成态验证后才关闭，工件必须携带 `develop@<sha>`。
8. **历史改写**：非常规路径；确需改写时 `git push --force-with-lease origin <feature>`，禁止裸 push。
9. **为何不 rebase**：rebase + 逐提交 cherry-pick 与 no-FF 常设决策直接冲突，2026-11-15 判为陈旧、不予迁移（当时 playbook 内两套说法并存，即双规范面漂移的实证）。冲突按 no-FF merge 常规解决。
10. **永不触碰 `master` 或他人泳道分支**。轮次级 develop→master、reset-develop、批次 tag 与部署的操作权威见 [../acts/release.md](../acts/release.md)。

## 9. done（终态）

- **台词**（`integration_confirmed` 尾句 / `req_amend_derived`）：`[REQ DERIVED] {{id}} is done — the amend derives a new requirement {{new}} "{{title}}" (origin={{id}}, rev {{rev}})`。
- **入口条件**：`integration.confirm` 成功；旁路关闭（split / merge / dropped）时为 `closed_reason`。
- **必做动作**：无（终态）；泳道已释放，证据在册。
- **本幕阅读**：[../scenes/README.md](../scenes/README.md)（`req_amend_derive_hist` / `req_after_done` / `req_resolve_*`）；旁路语义见 [../story/requirement-story.md](../story/requirement-story.md)。

## 10. 判据

① 阻断——只 push 功能分支就宣布收口、在 `origin/feature/*` 上合并、agent 代点验收门禁、把「键存在」当「生效」。② 症状——工件无 `develop@<sha>`、`integration_bad_sha`、集成后门禁未复跑而分支绿仍在引用、`master` 上出现功能提交。③ 确认——`git log --merges origin/develop -3` 有本需求的 `--no-ff` 节点、`git branch --contains <sha>` 不含 master、`git-delivery-check.sh --text` 原文行贴出。

维护责任 = 改集成流程、证据清单或旁路语义的执行者；触发 = 集成仪式变化、证据类型新增、SHA 证据形状变化；检查 = 本页命令与 `scripts/`、`.pi/tools/` 现行脚本一致，证据行可在最近一次工件里逐字找到。
