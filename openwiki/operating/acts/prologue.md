---
type: "Reference"
title: "序幕 · prologue — 引擎就绪，两条纪律先立在心里"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-9aedb7453b0c6f64c43d9da5
    resource: repo://.ygg/mods/agent-seed/data/texts/core.json
  - id: openwiki-source-d57e21e5bbbce0b80489fd49
    resource: repo://.ygg/mods/agent-seed/src/domain/requirements.lua
  - id: openwiki-source-3eff3e9a196ee7b3affec3be
    resource: repo://.ygg/mods/agent-seed/src/handlers/intake/actions.lua
  - id: openwiki-source-22b30a4cbdd3f021cc6c2529
    resource: repo://.ygg/mods/agent-seed/src/runtime/flow.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 序幕 · prologue — 引擎就绪，两条纪律先立在心里

> **id** acts/prologue · **幕** prologue · **触发** 进程 boot（`event.start`，可带章程载荷）
> **读不到代价** silent-drift（纯叙述幕，漏读不改状态，但会失去双层代理的元工作流前提）
> **replaces**：`docs/context/acts/prologue.md`（搬运基线 `3a2536f`）
> **上游**：[charter.md](charter.md) · **相关**：[../scenes/README.md](../scenes/README.md) · [../../contracts/kernel/state-machine.md](../../contracts/kernel/state-machine.md)

**场景台词（知识权威）**：引擎就绪，ygg 世界已装载（引擎版本、当前相位、演进偏好仅作叙述——资源从不被自动改写）。这是双层代理元工作流：协调层只做派发、串行受理、过两道门禁；每个需求走九幕（立项 → 计划中 → 计划门禁 → 待开发 → 开发中 → 交付受理 → 验收门禁 → 集成 → 完成）。

## 1. 这一幕在做什么

**入口**：进程 boot（`event.start`，可带章程载荷）。**必做**：无——这一幕只陈述前提，不改状态。**出口**：进入立局幕。

两个前提在这里说清，后面每一幕都靠它们：

- **协调层不写代码**。它派发、串行受理、过门禁、走批边界；实现属于执行者。这条纪律是「主代理与执行者」双层结构的全部意义。
- **每个需求走九幕**，每一幕的推进都要一次真实动作与一次核验。永不跳过——跳过在事件账上看得见，不在聊天里。

**两种模式（guide / goal）共享同一纪律**：受理与两道门禁各需一次真实动作与一次核验；需求可异步追加（`--requirement` / `--proposal` 只进 inbox，不触发派发）。

## 2. 判据

① 阻断——把「引擎版本 / 相位 / 演进偏好」的叙述当成动作去做（例如照着叙述文字改写资源）。② 症状——有人把序幕当配置面，或以为 `--requirement` 会立刻开工。③ 确认——boot 回执里没有任何状态变更，`requirement.list` 的 inbox 行要等到协调幕被受理才动。

维护责任 = 改 `src/runtime/flow.lua` 的 boot 序或本幕正文的执行者；触发 = boot 叙述或双层结构变更；检查 = 本页两句前提与 `prologue` 回执逐句一致。
