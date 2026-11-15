# 协调幕 · coordinate
> **id** acts/coordinate · **幕** coordinate · **触发** 进入协调幕 / 每次派发前
> **读不到代价** hard-refusal（载荷形状错、rev stale 被拒）
> **ver** 000000

**场景台词（知识权威）**：任务「{{title}}」：{{goal}}｜{{count}} 条需求（{{open}} 条进行中）｜{{pending}} 条待受理提案。这一层你只做三件事：**派发、串行受理、过两道门禁**；其余需求的分析/计划/开发可异步并行。

**阶段策略（coordinate_common）**：先受理提案（`proposal.accept`），再过门禁（`gate.approve` / `gate.reject`），再派发（`requirement.dispatch`），最后 `coordinate.submit` 收口进盘点。

**阶段策略（guide_hint，map-guide 组）**：现场规则（field rules）按章节索引——每条规则的标题与摘要在菜单提示行给出，全文在 `.ygg/mods/agent-seed/data/playbook/` 对应章；guide 模式下里程碑动作只能由人从菜单点选。

**人机门禁拒绝（guide_human_blocked，知识权威）**：`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note` 是 pass 型动作，受人类动作门禁保护——guide 模式下只能由人从菜单点选（回复索引 → `ygg.choice_selected`）；直接单发一派发永远被拒（`[REJECTED:ACTION]`），主状态不变。正确做法：把完整 pendingChoice 文本交给用户并等待索引，绝不代按。

**goal / guide 决策权（完整判据）**：

- 只有 Mod 解释 `play.mode`：runtime `settings.play.mode` 覆盖 `data/config.json` 静态缺省，host CLI 不读不解释。`--drive stdin|cmd:<path>|none` 只是宿主输入通道，与模式正交；goal 不依赖外部命令，删除该命令不会破坏 goal。
- `guide`（默认）的五个里程碑只收菜单票，直发始终拒绝；`*.reject` 不在票据门内，因为它们不产生通过效果，是唯一纠正通道。`goal` 中主代理读完整回执后自行派发，不是自动选 1、不经外部脚本，pendingChoice 仍完整打印且证据标准不变。
- 用户当次授权后可 `config.set play.mode=goal`；Mod 叙述 `handover_goal`（委托范围/证据不变/Closing review 须交代）或 `handover_guide`（决策权归还人），菜单只提供切向另一模式的边。
- 运行时 goal 必须逐局 `config.set goal.authorization=<单行说明>`（trim 后非空、≤200、无换行；空值显式清除）。每批结算与退役前的 `review.note` 都检查；缺失则叙述阻断、不写 ending、不前进。静态 goal 预设是仓库 owner 常设配置，不在检查范围；切回 guide 不查，resume 保留记录。该记录只证明登记了一句话，绝不证明是用户亲口授权，不能当身份证据。

**门禁与批边界（执行判据）**：`proposal.accept` 只按阶段推进——analysis→planning、plan→plan_gate、development→accept（交付同时释放泳道）；`gate.approve` 使 plan_gate→ready、accept→done。`gate.reject` 都回 planning：plan reject 置 `plan_ok=false` 须新计划；accept reject 保持旧值，未 amend 回 ready 重新开发，已 amend 因 plan_ok 失效停 planning。approve 前硬查 `gate_rev==req.rev`（amend 后旧批准 stale）及 accept 的 artifacts 非空；summary-only 不受理。

`coordinate.submit` / `verify.submit=true` / `review.note` 共享关闭前置：0 pending proposals、0 running jobs、0 未解冲突，且所有需求 done 或 split/merged/dropped 的后继全 done。`coordinate.submit` 仅在 coordinate 可用，成功写 settle、`verify.passed=false` 并清本批 settlement 键（settings 预登记保留）；`verify.submit=false` 在 `gates.require_all_green=true` 时打回 coordinate 并清 settlement 键。

**入口**：`run.begin`。**必做**（循环直到队列 drain）：受理 → 门禁 → 派发。**出口**：全部需求终态、0 待受理提案 / 0 运行 job / 0 未解冲突后，`coordinate.submit` → 盘点幕（`[COORDINATION DONE]`，run.batch 记批边界）。

**菜单语义对照**：`requirement.dispatch`（按就绪序派发）、`proposal.accept`（串行受理）、`gate.approve`/`gate.reject`（过/打回门禁）、`coordinate.submit`（批边界）、`requirement.list`（看板）、`map.show`、`config`。

**prompts.coordinate 语义**：下一步问题只保留动作名——「受理提案、过门禁、派发，还是收口进盘点（coordinate.submit）？」

**本幕关联阅读**：**[需求级九幕进行时](../req-acts/intake.md)（协调幕内每条需求的九幕指引）**；[rituals/dispatch.md](../rituals/dispatch.md)（派发仪式全文）；[copy/requirements.md](../copy/requirements.md) / [copy/lanes.md](../copy/lanes.md)（派发/提案/门禁/泳道句式）；[../../design/requirement-story.md](../../design/requirement-story.md)（九幕微状态表）、[../../design/lanes.md](../../design/lanes.md)（泳道池）、[../../design/state-surface.md](../../design/state-surface.md)（拒绝面矩阵）。
