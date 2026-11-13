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

## 立局幕 · charter

**场景台词（知识权威）**：立局 · 第 {{batch}} 批前。章程：「{{title}}」｜目标：{{goal}}｜成功判据：{{success}}。`charter.set` 在立局内可反复修订；三件套齐备后 `run.begin` 开工（章程随之锁定）。

**阶段策略（charter_common）**：章程三件套（标题 / 目标 / 成功判据）立好才可开工；`charter.set` 在立局内可反复修订，`run.begin` 后章程锁定。

**入口**：序幕后自动处于立局微状态。**必做**：① `charter.set`（或 boot 载荷一步立局）写齐三件套；② `requirement.add` 至少一条需求（否则 `coordinate.submit` 被 `gate_charter_first` 拒绝）。**出口**：`run.begin` → 协调幕（`[RUN BEGUN]`）。

**菜单语义对照**：`charter.set`（修订章程）、`run.begin`（开工）、`requirement.add`（异步立项）、`map.show`（看地图）、`config`（配置面板）。

**prompts.charter 语义**：下一步问题只保留动作名——「修订章程（charter.set），三件套齐备后开工（run.begin）？」

## 协调幕 · coordinate

**场景台词（知识权威）**：任务「{{title}}」：{{goal}}｜{{count}} 条需求（{{open}} 条进行中）｜{{pending}} 条待受理提案。这一层你只做三件事：**派发、串行受理、过两道门禁**；其余需求的分析/计划/开发可异步并行。

**阶段策略（coordinate_common）**：先受理提案（`proposal.accept`），再过门禁（`gate.approve` / `gate.reject`），再派发（`requirement.dispatch`），最后 `coordinate.submit` 收口进盘点。

**阶段策略（guide_hint，map-guide 组）**：现场规则（field rules）按章节索引——每条规则的标题与摘要在菜单提示行给出，全文在 `.ygg/mods/agent-seed/data/playbook/` 对应章；guide 模式下里程碑动作只能由人从菜单点选。

**人机门禁拒绝（guide_human_blocked，知识权威）**：`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note` 是 pass 型动作，受人类动作门禁保护——guide 模式下只能由人从菜单点选（回复索引 → `ygg.choice_selected`）；直接单发一派发永远被拒（`[REJECTED:ACTION]`），主状态不变。正确做法：把完整 pendingChoice 文本交给用户并等待索引，绝不代按。

**入口**：`run.begin`。**必做**（循环直到队列 drain）：受理 → 门禁 → 派发。**出口**：全部需求终态、0 待受理提案 / 0 运行 job / 0 未解冲突后，`coordinate.submit` → 盘点幕（`[COORDINATION DONE]`，run.batch 记批边界）。

**菜单语义对照**：`requirement.dispatch`（按就绪序派发）、`proposal.accept`（串行受理）、`gate.approve`/`gate.reject`（过/打回门禁）、`coordinate.submit`（批边界）、`requirement.list`（看板）、`map.show`、`config`。

**prompts.coordinate 语义**：下一步问题只保留动作名——「受理提案、过门禁、派发，还是收口进盘点（coordinate.submit）？」

## 盘点幕 · settle

**场景台词（知识权威）**：任务「{{title}}」：{{count}} 条需求（{{done}} 条已终态）｜require_all_green {{gates}}。四项齐备（验证、演进登记、会话清理、待办清理）自动进发布幕。

**阶段策略（settle_common）**：盘点四项——`verify.submit` 先记全绿 → `settlement.evolution` 登记演进（可并入 `sessions=` / `todo=` 可选段一次事务）→ `settlement.sessions` → `settlement.todo`；四项齐备自动转发布幕（不占事件）。会话清理与待办清理的政策正文见 [cleanup-policy.md](cleanup-policy.md)。

**入口**：`coordinate.submit` 成功（verify.passed 复位为 false，本批 settlement 键清空）。**必做**：① `verify.submit` 记全绿（`all_green=false` 且 `require_all_green=true` 时打回协调幕返工）；② `settlement.evolution` 登记（summary 必填）；③④ 会话与待办清理声明。**出口**：四项齐备 → 发布幕（`[SETTLE READY]`，R9 自动转场）。

**菜单语义对照**：`verify_green` / `verify_fail`（记绿/记不绿）、`settlement_evolution`（登记演进）、`review_note`（批结算/退役）、`map.show`。

**prompts.settle 语义**：「门禁全绿了吗？依次验证 → 演进登记 → 会话 → 待办；四项齐备自动进发布幕。」

## 发布幕 · release

**场景台词（知识权威）**：第 {{batch}} 批 · 任务「{{title}}」{{declared}}。声明本批 develop→master 合并结果（`release.confirm` / `release.skip`）；已声明即可 `review.note` 批结算（未武装）或退役（`run.retire` 武装）。

**阶段策略（release_common）**：`release.confirm` 载荷声明 `master@<sha>` 与批次 tag（`v年.月.日.序号`），或 `release.skip` 声明跳过理由；任一成立即「发布已声明」，`review.note` 才可批结算/退役。

**入口**：盘点四项齐备自动进入。**必做**：发布声明二选一。**出口**：`review.note`——未武装 = 批结算回协调幕（`[BATCH n SETTLED]`，run.batch+1）；武装 = 写 ending 退役（`[ENDING · run retired]`）。

**菜单语义对照**：`release.confirm` / `release.skip`（声明/跳过）、`review_note`（收口）。

**prompts.release 语义**：「声明本批结果（release.confirm / release.skip）后 review.note 批结算；run.retire 武装时它是终局。」

## 退役终态 · retired

**场景台词（知识权威）**：本局已退役。

**阶段策略（retired_common）**：业务事件全拒；`map.show` / `requirement.list` 只读查询仍可用。

**ending 语义**：`[ENDING · run retired]` 前缀由 pack `agent.endingPrefix` 声明；结算汇总（任务、需求计数、批数、评分、演进候选、一句话教训）由 `endings.done` 键渲染，句式契约见 [copy-contract.md](copy-contract.md)。

**prompts.retired 语义**：「本局已退役：只读查询（map / requirement.list）。」

## 无 egg 执行本章（无 mod 降级路径）

不安装 egg / mod 时，本工作流仍可运行——**context 树（含本文）就是完整的工作流底座**：

1. **流程语义**：按本文五幕顺序与每幕「入口/必做/出口」人工执行；需求九幕与两道门禁的语义不变（立项 → 计划 → 计划门禁 → 开发 → 交付受理 → 验收门禁 → 集成 → 完成）。
2. **动作名的落地**：`charter.set` = 在会话中写下章程三件套并确认；`requirement.add/dispatch` = 维护需求清单（标题/描述/依赖/状态），按就绪序派发子代理；`proposal.accept` / `gate.approve` = 主会话逐条审阅子代理交付并明示受理/过门禁；`coordinate.submit` = 宣告本批需求全部终态；`verify.submit` = 记录本批验证证据；`settlement.*` = 写下演进登记、会话清理、待办清理三项声明；`release.confirm/skip` = 记录合并 SHA 与批次 tag；`review.note` = 写复盘（lesson|score）。
3. **文案行为可预知**：每个动作「会说什么、拒绝时说什么」查 [copy-contract.md](copy-contract.md)（句式模板 + 协议前缀归属）；派发仪式查 [dispatch-ritual.md](dispatch-ritual.md)；清理政策查 [cleanup-policy.md](cleanup-policy.md)。
4. **三个缺口（明示，不伪装）**：无 egg 时缺少的是——① **工具监控状态**（没有状态机自动判定微状态/就绪序/拒绝面，由主会话自律核对）；② **记录历史**（没有事件日志与自动存档，历史由会话 transcript 与 git 承担）；③ **强制流程合规**（门禁不再由机器 fail-closed 拦截，主会话必须按本文与 AGENTS.md 自律执行，评审者按同一清单核验）。
5. **禁止降级的项**：即使无 egg，「派发 → 提案 → 两道门禁 → 批结算」的顺序与「每步真实动作 + 核验」的证据标准**不降低**——降级的是工具，不是纪律。
