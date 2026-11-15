# 立局幕 · charter
> **id** acts/charter · **幕** charter · **触发** 进入立局幕 / `charter.set` / `run.begin`
> **读不到代价** hard-refusal（三件套不齐则 `coordinate.submit` 被 `gate_charter_first` 拒）
> **ver** 3187d8

**场景台词（知识权威）**：立局 · 第 {{batch}} 批前。章程：「{{title}}」｜目标：{{goal}}｜成功判据：{{success}}。`charter.set` 在立局内可反复修订；三件套齐备后 `run.begin` 开工（章程随之锁定）。

**阶段策略（charter_common）**：章程三件套（标题 / 目标 / 成功判据）立好才可开工；`charter.set` 在立局内可反复修订，`run.begin` 后章程锁定。

**入口**：序幕后自动处于立局微状态。**必做**：① `charter.set`（或 boot 载荷一步立局）写齐三件套；② `requirement.add` 至少一条需求（否则 `coordinate.submit` 被 `gate_charter_first` 拒绝）。**出口**：`run.begin` → 协调幕（`[RUN BEGUN]`）。

**菜单语义对照**：`charter.set`（修订章程）、`run.begin`（开工）、`requirement.add`（异步立项）、`map.show`（看地图）、`config`（配置面板）。

**prompts.charter 语义**：下一步问题只保留动作名——「修订章程（charter.set），三件套齐备后开工（run.begin）？」

**本幕关联阅读**：[copy/flow.md](../copy/flow.md)（`charter_*` / `run_begun` / `gate_charter_first` 句式）；[../../design/run-story.md](../../design/run-story.md)（立局微状态）；需求写法与合并判据见 [req-acts/intake.md](../req-acts/intake.md)「1. 立项」。
