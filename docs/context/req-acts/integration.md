# 需求级九幕进行时 · 集成（req-acts/integration）
> **id** req-acts/integration · **幕** req: accept_gate/integrating/done · **触发** 验收门禁前 / 集成核验前
> **读不到代价** hard-refusal（证据清单漏项 → 验收被拒；SHA 不符 → `integration_bad_sha` 打回）
> **ver** 73bc9e

## 7. 验收门禁（accept_gate）

- **场景台词**（`req_gate_ok` kind=accept）：`[GATE PASSED · accept] {{id}} → integrating | evidence: {{artifacts}}`——验收已过，转集成；`req_gate_no_evidence`：artifacts 为空时拒绝（summary-only 声明不受理）。
- **入口条件**：development 提案已受理（`gate=="accept"`）。
- **必做动作**：**人**过验收——`gate.approve`（须核验 `develop@<sha>` 证据与 artifacts 非空）或 `gate.reject`（打回待开发，plan_ok 保持）；gate_rev==req.rev 的 stale 校验同样适用。
- **出口判据**：approve → 集成幕（`integrating=true`）；reject → 待开发。
- **本幕关联阅读**：[copy/requirements.md](../copy/requirements.md)（`req_gate_ok` / `req_gate_no_artifacts` / `req_gate_no_evidence` / `req_gate_stale` 句式）；[../../design/kernel/state-surface.md](../../design/kernel/state-surface.md)（验收证据键）。
- **场景化证据清单**（按需求类型选列核验，避免“看过就算”）：

  **通用子句（对所有行生效）**：证据只度量「代码存在」是不合格的。凡本需求引入/依赖**开关型、feature-flag 型**能力（「缺席即失效 / 关掉即失效」的那一类），必须追加「**在真实配置下处于生效态**」的证据：`data/config.json` 里的生效值 + 对应的 `*_ships_armed_*` 用例名与结果。r4 首版交付三套门禁 473/158/77 全绿而 `readGate`/`readPlan` 根本不存在，就是这一条缺失的代价。**schema 只校形状，不校值域**：可选段可以整段缺席（r4 实况），必填键可以是 `false`（schema 合法但保护静默失效）——两形态都只能靠 armed 断言钉。

  | 需求类型 | 必须亲测的证据 |
  | --- | --- |
  | **交付洁净度（所有需求类型必核）** | 执行者回执中的 `git-delivery-check.sh --text` 原文行：`ok=1` 且 `dirty`/`unpushed`/`stash`/`conflict_debris`/`fmt` 五项计数全 0；`n/a` / `waived` 项须附理由。**回执里只有「工作树 clean」字样、没有这一行的，按证据缺失不受理**。本行不随需求类型变化——代码需求它验「改动已提交且已推送」，纯文档需求同样验（文档改动也会漏提交） |
  | **开关型 / feature-flag 特性**（「缺席即失效 / 关掉即失效」的能力） | ① `data/config.json` 实测存在**且生效**（非空表 / 布尔 `true`，不是「键在这里」）；② 同特性分支的 `*_ships_armed_*` 语义用例存在且全绿（贴用例名）；③ 剥掉/关掉该配置的副本仍走退让路径（回归保护未丢）。典型对象：`readPlan`/`readGate`（可选段，缺席即 inert）、`gates.require_all_green`（required，但 `false` 即静默失效） |
  | 跨仓库同步（mod/CLI/文档搬运） | 源↔目标 `diff -rq` 为空；二进制 `sha256` 双侧一致；目标仓 boot 三件套实测；docs 闭包 ok=1；数据红线零触碰；子模块指针未动 |
  | Mod 代码/文案改动 | `scripts/sync-seed-mod.sh --check`；`cargo run -p ygg-lint -- --pack .ygg/mods/agent-seed`（期望 `0 error(s), 0 warning(s)`）；`cargo test -p ygg-sandbox --test agent_seed_semantics` 全绿（含新增用例）；`cargo run --example playtest -- .ygg/mods/agent-seed` 的 all-1 走到 `[ENDING` |
  | 宿主 crates 改动 | 字面三件套（`cargo test --workspace` ∧ `--example egg` ∧ `agent_seed_semantics`），记录真实退出码与计数 |
  | 纯文档/规则修改 | `docs-consistency` ok=1；受影响的键数/枚举实测对齐；语义测试不变红 |
  | 需求图/台账操作 | `requirement.list` 终态数对得上；分支模型/注册项存在性实测 |

  清单是三问的一环：① 本需求属于哪类？② 该类的每项证据跑了吗（贴末行/输出）？③ 没跑的那项为何不适用（写明）。新增一问：**④ 有没有把「已存在」错当成「已生效」？**

## 8. 集成（integrating）

- **场景台词**（`integration_confirmed`）：`[INTEGRATION CONFIRMED] {{id}} develop@{{sha}} 已核验 → 完成（已终态 {{done}} 条）`；`integration_rejected`：集成打回 → 待开发，理由在册。
- **入口条件**：验收门禁通过（`integrating=true`）。
- **必做动作**：**执行者**在收口时自行集成功能分支 → `origin/develop`（对齐 → `git merge --no-ff` → 集成态门禁绿 → push develop），工件记录 `develop@<sha>`；主会话 `integration.confirm` 核验 SHA 后关需求，或 `integration.reject` 打回。
- **出口判据**：confirm → done（`status=done`，泳道释放）；reject → 待开发（写 feedback、释放泳道）。
- **本幕关联阅读**：[rituals/dispatch.md](../rituals/dispatch.md)（集成纪律：no-FF / 不碰 master / 不碰他人泳道）；[copy/requirements.md](../copy/requirements.md)（`integration_*` 句式、`integration_bad_sha` 证据形状）。
- **集成纪律（development 阶段收口时）**：子代理自己把功能分支集成进 `develop`：对齐 `origin/develop` → `git merge --no-ff` → 集成态门禁全绿 → push `develop` → 在工件中记录 `develop@<sha>`。**永不触碰 `master` 或他人泳道分支**。轮次级 develop→master、reset-develop 与批次 tag 的操作权威见 [acts/release.md](../acts/release.md) 发布幕（集成分工 只保存裁决与指针）。

## 9. done（终态）

- **场景台词**（`integration_confirmed` 尾句 / `req_amend_derived`）：需求已终态；对 done 需求再修订会派生新需求：`[REQ DERIVED] {{id}} is done — the amend derives a new requirement {{new}} "{{title}}" (origin={{id}}, rev {{rev}})`。
- **入口条件**：`integration.confirm` 成功；旁路关闭（split/merge/dropped）时为 `closed_reason`。
- **必做动作**：无（终态）；泳道已释放，证据在册。
- **出口判据**：无（终态）。
- **本幕关联阅读**：[copy/requirements.md](../copy/requirements.md)（`req_amend_derive_hist` / `req_after_done` / `req_resolve_*` 句式）；旁路语义见 [../story/requirement-story.md](../story/requirement-story.md)（覆盖层：split/merge/conflict/resolve）。
