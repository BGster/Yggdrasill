# Yggdrasill · 提示词面（beats）契约

> **定位**：7 个 data/texts 片段 × 383 键的职责归属与文案键分组表。
> **replaces**：DESIGN §2.12（L356–371）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[identity.md](identity.md)
> **搬运**：以下正文自 DESIGN.md L356–L371 逐字迁移，不重写语义。

### 2.12 提示词面（beats）契约

叙事文案与提示词全部存放在 `data/texts/` 片段（383 个 beats 键 + core），按域分组、单一职责（P2 零硬编码；`mod.lua` 装配回 TEXTS）：

| 片段 | 键数 | 职责 |
| --- | --- | --- |
| `beats-flow.json` | 63 | 回执版式、guide/goal 提示、拒绝文案（`rejected*` / `stage_expected`）、五幕 `<节点>_common` 行文（prologue/charter/coordinate/settle/release/retired） |
| `beats-requirements.json` | 144 | 需求族文案：派发/提案/门禁/打回/依赖校验/九幕行文（含集成确认/打回与台账错位提示） |
| `beats-lanes.json` | 29 | 泳道账本、认领/释放、分支命名拒绝 |
| `beats-map-guide.json` | 38 | 地图视图、菜单行、人机门禁提示、`map_note_*` 节点注记 |
| `beats-settlement.json` | 47 | 盘点四项、批结算、退役、发布幕文案；批结算自动维护追加五键（D-MA25） |
| `beats-harness.json` | 62 | 配置面板、身份注册、工具桥提示、`subagent_submit_cmd` 模板 |
| `core.json` | — | `scenes`（5）/ `endings`（1）/ `prompts`（5）/ `options`（27）/ `tuning`（8） |

文案键归属随微状态走（§2.7 的拒绝文案、各幕行文键在 `beats-requirements` / `beats-settlement`）；立局/集成/发布三幕的文案键已随代码任务落位并与本表对齐（闭环纪律：代码引用键 ⊆ 本表键，动态拼接键——`<phase>_common`、`map_note_*`、`*_misplaced`——按拼接/数据链核销）。

