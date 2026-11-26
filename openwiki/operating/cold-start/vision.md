---
type: "Reference"
title: "产品愿景与双重身份 — 一个内核，两种产品"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-8339725b1afcf7133cd86688
    resource: repo://.ygg/mods/agent-seed/pack.json
  - id: openwiki-source-9ce67273a358a1bdc9d71e1e
    resource: repo://.ygg/mods/agent-seed/src/domain/wire.lua
  - id: openwiki-source-ef4b88be1d64012bd99b7527
    resource: repo://crates/ygg-core/src/lib.rs
  - id: openwiki-source-60a2819491a959c4754363a8
    resource: repo://crates/ygg-lint/src/lib.rs
  - id: openwiki-source-55ac8b91f1869ef9305b6b62
    resource: repo://crates/ygg-sandbox/src/host_contract/settlement.rs
  - id: openwiki-source-61a53c8e1a9f6204afd357f2
    resource: repo://crates/ygg-wasm/src/lib.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 产品愿景与双重身份 — 一个内核，两种产品

> **id** cold-start/vision · **幕** cold-start · **触发** 冷启动 / 判断产品身份
> **读不到代价** silent-drift
> **replaces**：`docs/context/cold-start/vision.md`（搬运基线 `3a2536f`，原 CONTEXT §1 L25–42）
> **上游**：[overview.md](../../overview.md) · **相关**：[system-boundaries.md](system-boundaries.md) · [principles.md](principles.md)

## 1. 统一命题

Yggdrasill 围绕一个命题设计：**一个确定性的、数据驱动的规则内核，其中 Lua 脚本与经 Schema 校验的 JSON 资源定义可玩世界，而 AI 同时扮演内容创作者与自主玩家。**

同一内核服务两种互补身份。

### 1.1 游戏与互动小说平台（"Minecraft + AI Modder"）

- **核心价值**：纯函数式迁移引擎 `(state, event) -> state'` 既能先驱动文字冒险，也能在不重写内核的前提下扩成完整战术 / 叙事 RPG（文字冒险 = RPG 减去战斗 / 地图 / 成长）。
- **无限内容供给**：传统 UGC 受限于人类 modding 工时；Yggdrasill 把「小而正交的 Mod 契约（Lua 5.4 逻辑 + JSON Schema 数据）」与「AI 生成与校验流水线（`ygg-lint` + `playtest`）」配对，使自然语言前提可编译成立即可玩的 Mod。
- **一次编写、处处运行**：内核 Rust 编写、零渲染零平台 IO；同一内核可编成 WebAssembly（`ygg-wasm`，浏览器）、原生链进 Godot（`gdext`），也可在服务器上原生跑（多人 lockstep 回放 / 小游戏客户端-服务器）。

### 1.2 Agent 工作流治理平台（"工作流即游戏，治理内核即引擎"）

- **核心价值**：多智能体软件工程协调被建模为一个可玩 Mod（`agent-seed`）。Agent 不靠会随长上下文衰减的静态提示，而是用事件信封一步步和状态机交互。
- **情境化可供性**：每次 dispatch 返回完整叙述回执——当前状态、活动泳道、待决提案、显式 `pendingChoice` 动作白名单。**协议约束取代纯提示纪律**。
- **两层协调**：一个主协调者分解需求、把并行执行者派进隔离的持久化 worktree 泳道、串行审提案、过两道人门（`plan_gate` / `accept_gate`），并以结构化 roguelike 式自进化审查结算每一局。

## 2. 身份边界：产品 vs 本仓开发

这两个身份描述的是**产品能力**，不是本仓的开发流程。第一性区分：

- **产品在做的事**：egg 让一个 agent 宿主按「需求 → 派发 → 提案 → 两门 → 批结算」跑工作流；这是本仓要**建造并验证**的东西。
- **本仓开发做的事**：我们自己改这个仓库时，用的是提示词 + 技能 + 子代理的循环（`AGENTS.md`），不是在自己身上跑 egg。
- 混用的代价：把产品仪式套到自己头上会更慢更脆；把本仓流程写进产品语义会让内核承担编排职责（违 [principles P11](principles.md)）。

一句话：**egg 是产品，subagent + skill 是本仓的手**。

## 3. 判据

① 阻断——用产品术语描述本仓开发步骤、或把「跑通一次 egg 需求」当成改代码的前置条件。② 症状——`AGENTS.md` 里出现由产品状态机驱动的开发指令、评审要求「先 dispatch 再改文件」、把 repo 开发失败归因于 egg 末跑。③ 确认——`AGENTS.md` 的开发循环只含 brief / 执行者 / 泳道 / 门禁复跑 / 合并五个词；`grep -n 'requirement.dispatch\|proposal.submit' AGENTS.md` 的结果都落在「产品行为」一段内。

维护责任 = 改产品定位或本仓开发范式的执行者；触发 = 新增产品身份、或开发流程从提示词循环换成别的载体；检查 = 本页 §2 与 `AGENTS.md` 的「This repo is a product being built」段逐条对应。
