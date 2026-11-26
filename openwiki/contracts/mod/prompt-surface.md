---
type: "Reference"
title: "提示词面（beats）契约"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-f6b499648596f0a05b479153
    resource: repo://.ygg/mods/agent-seed/data/texts/beats-harness.json
  - id: openwiki-source-3b9d5c08b79ce6e6218a7954
    resource: repo://.ygg/mods/agent-seed/data/texts/beats-requirements.json
  - id: openwiki-source-9aedb7453b0c6f64c43d9da5
    resource: repo://.ygg/mods/agent-seed/data/texts/core.json
  - id: openwiki-source-cfe8ec9e3ed7fc4dc881025e
    resource: repo://.ygg/mods/agent-seed/mod.lua
  - id: openwiki-source-a6633215523d35fcad78d575
    resource: repo://.ygg/mods/agent-seed/src/view/guide.lua
  - id: openwiki-source-da01aaf4db0507a7401ab486
    resource: repo://.ygg/mods/agent-seed/tests/surface.lua
  - id: openwiki-source-da9af83703d953f139bc0632
    resource: repo://scripts/gen-scene-readfirst.py
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 提示词面（beats）契约

> **id** mod/prompt-surface · **模块** ③ agent-seed mod · **触发** 改文案键面或键账之前
> **读不到代价** silent-drift（键面错 → 文案缺失或回执读不到）
> **replaces**：DESIGN §2.12（L356–371）（搬运基线 `3a2536f`）
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[identity.md](identity.md) · [../operating/scenes/README.md](../../operating/scenes/README.md)

## 1. 键面总账

叙事文案与提示词全部存放在 `data/texts/` 片段（**418 个 beats 键** + `core.json` 的 options/tuning/endings **36 键**，共 **454 个唯一键**），按域分组、单一职责（零硬编码；`mod.lua` 装配回 TEXTS）。

| 片段 | 键数 | 职责 |
| --- | --- | --- |
| `beats-flow.json` | 63 | 回执版式、guide/goal 提示、拒绝文案（`rejected*` / `stage_expected`）、六幕 `<节点>_common` 行文（prologue / charter / coordinate / settle / release / retired） |
| `beats-requirements.json` | 144 | 需求族：派发 / 提案 / 门禁 / 打回 / 依赖校验 / 九幕行文（含集成确认与打回、台账错位提示） |
| `beats-lanes.json` | 36 | 泳道账本、池注册（`lane.pool`）与认领 / 释放、分支命名拒绝、池化标签叙事 |
| `beats-map-guide.json` | 38 | 地图视图、菜单行、人机门禁提示、`map_note_*` 节点注记 |
| `beats-settlement.json` | 54 | 盘点四项、批结算、退役、发布幕；批结算文档维护义务五键；tools 机证七键 |
| `beats-harness.json` | 83 | 配置面板、身份注册、工具桥提示、`subagent_submit_cmd` 模板；tools 注册面七键 |
| `core.json` | — | `scenes`(5) / `endings`(1) / `prompts`(5) / `options`(27) / `tuning`(8) |

文案键归属随微状态走（拒绝文案、各幕行文键在 `beats-requirements` / `beats-settlement`）。**闭环纪律：代码引用键 ⊆ 本表键**；动态拼接键（`<phase>_common`、`map_note_*`、`*_misplaced`）按拼接 / 数据链核销。

## 2. 正文归属列

mod 职责收缩为「状态管理 + 需求看板 + 上下文索引提示」后，文案的**知识权威**在运行流程层文档，mod `data/texts/` 是**运行时投影**（418 beats + core 36；宿主零改动，因为它没有文案缺省机制）：

| 知识形态 | 键范围 | 正文权威 | mod 侧形态 |
| --- | --- | --- | --- |
| 状态行文（错误码 / 回执 / 台账 / 面板 / 菜单，454 唯一键） | 全部 beats 键 + core options/tuning/endings | [../operating/scenes/README.md](../../operating/scenes/README.md)（七域索引 + 逐域句式表） | **原文留 mod**（运行时物理必须） |
| 幕台词与阶段策略 | 6 个 `*_common`、`scenes.*`、`prompts.*`、`guide_human_blocked`、`guide_hint` | [../operating/acts/](../../operating/acts/) + [../operating/req-acts/](../../operating/req-acts/)（幕 1:1 一文件 + 需求级三份） | 收缩为「状态前缀 / `{{变量}}` 头 + 权威文档指针」；`scenes.*` 值尾另带脚本生成的 `[READ FIRST]` 必读块（`scripts/gen-scene-readfirst.py`，见 [../operating/scenes/README.md](../../operating/scenes/README.md)） |
<!-- openwiki: broken internal link [../operating/rituals/dispatch.md] file "../operating/rituals/dispatch.md" does not exist. Fix the href or restore the target, then delete this comment. -->
<!-- openwiki: broken internal link [../operating/rituals/fallback.md] file "../operating/rituals/fallback.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| 派发仪式（3 键） | `subagent_brief`、`config_opened`、`subagent_fallback_note` | [../operating/rituals/dispatch.md](../../operating/rituals/dispatch.md) + [fallback.md](../../operating/rituals/fallback.md) | 事实头留 mod + 指针 |
<!-- openwiki: broken internal link [../operating/rituals/cleanup.md] file "../operating/rituals/cleanup.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| 清理政策（2 键） | `settlement_sessions_unsetted_hint`、`settlement_todo_segment` | [../operating/rituals/cleanup.md](../../operating/rituals/cleanup.md) | 判据短句 + 指针 |

**单向同步纪律**：改文案先改权威域文档再同步 mod 键值。指针锚点存在性由 `selftest` 的 `narrative_pointer_anchors_exist_in_docs_context` 用例机器闭环；逐域键集由 `copy_contract_domain_fragments_cover_every_text_key` 用例闭环。owner = texts / 契约编辑者；trigger = 任一键或指针变化；**失败 = 缺键、死链或计数漂移，不得验收。**

## 3. 迁移适配

- 上表中所有 `docs/context/**` 权威路径已迁至 `openwiki/operating/**`；mod 侧指针字符串与 `gen-scene-readfirst.py` 的输入根随之改指 `openwiki/`（同一 commit，勿留旧前缀）。
- `scenes.*` 的 `[READ FIRST]` 块仍由脚本生成，页首四行头（id / 幕 / 触发 / 读不到代价）保持不变——它是读取面的机器契约，不随知识库搬迁而变。
