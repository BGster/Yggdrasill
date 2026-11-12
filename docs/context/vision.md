# Yggdrasill · 产品愿景与双重身份

> **定位**：Yggdrasill 的统一命题与两种产品身份（游戏与互动小说平台 / Agent 工作流治理平台），产品形态决策的领域起点。
> **replaces**：CONTEXT §1（L25–42）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[../design/system-boundaries.md](../design/system-boundaries.md) · [principles.md](principles.md)
> **搬运**：以下正文自 CONTEXT.md L25–L42 逐字迁移，不重写语义。

## 1. 产品愿景与双重身份

Yggdrasill 围绕一个统一命题设计：**一个确定性的、数据驱动的规则内核，其中 Lua 脚本与经 Schema 校验的 JSON 资源定义可玩世界，而 AI 同时扮演内容创作者与自主玩家**。

这一架构在完全相同的内核上服务两种互补的产品身份：

### 1.1 游戏与互动小说平台（"Minecraft + AI Modder"）
- **核心价值**：一个纯函数式状态迁移引擎 `(state, event) -> state'`，既能先驱动文字冒险，也能在无需重写内核的前提下扩展为完整的战术/叙事 RPG（文字冒险 = RPG 减去战斗/地图/成长子系统）。
- **无限内容供给**：传统 UGC 平台受限于人类 modding 工作量。Yggdrasill 将一个小而正交的 Mod 契约（Lua 5.4 逻辑 + JSON Schema 数据）与 AI 生成与校验流水线（`ygg-lint` + `playtest`）配对，使自然语言前提可被编译为立即可玩的 Mod。
- **一次编写、处处运行**：内核用 Rust 构建，零渲染、零平台 I/O 依赖；同一内核可编译为 WebAssembly（`ygg-wasm`，用于浏览器）、原生链接进 Godot（`gdext`），并在权威服务器上原生运行（用于多人 lockstep 回放或微信小游戏客户端-服务器架构）。

### 1.2 Agent 工作流治理平台（"工作流即游戏，治理内核即引擎"）
- **核心价值**：软件工程多智能体协调被建模为一个可玩的 Mod（`agent-seed`）。Agent 不再依赖会随长上下文窗口衰减的脆弱静态提示指令，而是通过事件信封（event envelope）逐步与工作流状态机交互。
- **情境化可供性（Situational Affordances）**：每一次 dispatch 都返回一份完整叙述回执，描述当前状态、活动泳道、待决提案，以及显式的 `pendingChoice` 动作白名单。协议约束取代了纯提示式的纪律。
- **两层协调**：一个主协调者 Agent 分解需求、将并行的执行者子代理派发进隔离的持久化 Git worktree 泳道、串行审查提案、通过两个强制的人/协调者门禁（`plan_gate` 与 `accept_gate`），并以结构化的 roguelike 式自进化审查结算每一次运行。

---

