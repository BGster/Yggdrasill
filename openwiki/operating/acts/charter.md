---
type: "Reference"
title: "立局幕 · charter — 三件套立好才能开工"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-9aedb7453b0c6f64c43d9da5
    resource: repo://.ygg/mods/agent-seed/data/texts/core.json
  - id: openwiki-source-20786f1cdc5649019aa1a3b9
    resource: repo://.ygg/mods/agent-seed/src/domain/gates.lua
  - id: openwiki-source-9ce67273a358a1bdc9d71e1e
    resource: repo://.ygg/mods/agent-seed/src/domain/wire.lua
  - id: openwiki-source-ba1c15010aed43f1ce2468ae
    resource: repo://.ygg/mods/agent-seed/src/handlers/intake/charter.lua
  - id: openwiki-source-d2f4a8be1848c02ad741c27e
    resource: repo://.ygg/mods/agent-seed/src/view/menus.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 立局幕 · charter — 三件套立好才能开工

> **id** acts/charter · **幕** charter · **触发** 进入立局幕 / `charter.set` / `run.begin`
> **读不到代价** hard-refusal（三件套不齐则 `coordinate.submit` 被 `gate_charter_first` 拒）
> **replaces**：`docs/context/acts/charter.md`（搬运基线 `3a2536f`）
> **上游**：[operating/scenes/README.md](../scenes/README.md) · **相关**：[prologue.md](prologue.md) · [../story/run-story.md](../story/run-story.md) · [../req-acts/intake.md](../req-acts/intake.md)

**场景台词（知识权威）**：立局 · 第 {{batch}} 批前。章程：「{{title}}」｜目标：{{goal}}｜成功判据：{{success}}。

## 1. 这一幕在做什么

序幕后自动处于立局微状态。**章程三件套**——标题 / 目标 / 成功判据——立好才可开工：

```
$A --event charter.set --payload '"<标题>|<目标>|<成功判据>"'
$A --event requirement.add --payload '"<需求标题>|<需求描述>"'   # 至少一条
$A --event run.begin
```

- `charter.set` 在立局内**可反复修订**（最后一次为准）；在其它相位调用直接被拒（`charter_locked`，回执带当前相位名）。
- boot 时可直接带章程载荷一步立局；等价形态是 `new` 后立即 `charter.set`。
- `run.begin` **锁定章程**：`task.title` / `task.goal` / `task.success` 此后只读，改它要退役重开。

## 2. 必做与出口

**必做**：① `charter.set` 写齐三件套；② `requirement.add` **至少一条**需求。

两道门分别落在两处，别混淆：

- **`run.begin` 自身**就要求三件套齐备（`charter_complete()` 用 `ygg.kv.get` 裸读 `task.*`、不带默认值、逐项 `trim` 后非空），否则回 `charter_missing`——三件套这关过不了，连协调幕都进不去。
- **零需求**在 `coordinate.submit` 被 `gate_charter_first` 拒（`src/domain/gates.lua`）。

**`run.begin` 顺手作废本幕签名**：`readsig.clear()` ——立局幕读过的必读件不跨到协调幕，进幕重读。这是「按幕签名」的第一个实例：签名不是一次性通行证，是每一幕各自的门票。

**出口**：`run.begin` → 协调幕（回执 `[RUN BEGUN]`）。

**为什么要有「至少一条需求」**：只有章程没有需求时，协调幕没有可派发的东西，`coordinate.submit` 会成就一个空批——空批让「批次」这个计数单位失去意义，也让结算与维护义务无从对齐。

## 3. 菜单语义对照

| 动作 | 语义 |
| --- | --- |
| `charter.set` | 修订章程（立局内唯一可重复的写动作） |
| `run.begin` | 开工（锁定章程，进协调幕） |
| `requirement.add` | 异步立项（只进 inbox，不触发派发） |
| `map.show` | 看地图（操作视图 + 全章索引） |
| `config` | 配置面板（`config.set` 的只读视图） |

`prompts.charter` 的下一步问题只保留动作名：「修订章程（`charter.set`），三件套齐备后开工（`run.begin`）？」——它不解释三件套是什么，因为那是本页的事。

## 4. 判据

① 阻断——三件套缺失（标题空、目标空、成功判据空）、零需求、或 `run.begin` 之后仍试图 `charter.set`。② 症状——`coordinate.submit` 回执带 `gate_charter_first`、章节标题在批回执里显示为占位符、`charter.set` 在 `run.begin` 后返回 `PHASE` 类拒绝。③ 确认——`$A --event map.show` 的章节标题与成功判据非空，`requirement.list` 至少一行，且 `charter.set` 的修订在 `run.begin` 后不再被接受。

维护责任 = 改 `src/handlers/charter`、`src/runtime/flow.lua` 或本幕正文的执行者；触发 = 三件套语义或门禁条件变更；检查 = 立局微状态用例 + 本页三处 payload 形状与代码一致。
