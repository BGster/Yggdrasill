# Yggdrasill · 产品形态与三层系统边界

> **定位**：三层架构图、跨治理引擎能力协商矩阵（Tier 1–3）与 Adapter 一致性契约五义务（C1–C5）。
> **replaces**：DESIGN §1（L11–65）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[../context/vision.md](../context/vision.md) · [cli-daemon.md](cli-daemon.md)
> **搬运**：以下正文自 DESIGN.md L11–L65 逐字迁移，不重写语义。

## 1. 产品形态与三层系统边界

Yggdrasill 将确定性规则执行、声明式工作流语义与宿主环境集成分离为三个严格有界的层：

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ Layer 2 · Presentation & Harness Adapter Shell                             │
│   - Game Clients: Web Browser (React + ygg-wasm), Godot 4 (gdext)          │
│   - Agent Adapters: 无内置执行壳（扩展层已删除，主会话直驱 egg CLI）      │
│     CLI Daemon Consumers (Claude Code / Codex / Cursor hooks)              │
│   Responsibilities: UI rendering, subagent process spawning/resuming,      │
│   context window telemetry & compaction, data red-line firewall.           │
├────────────────────────────────────────────────────────────────────────────┤
│ Layer 1 · Playable Mod Package (.ygg/mods/agent-seed, test-adventure)      │
│   - Executable Logic: Entry manifest (mod.lua) + pure modules (src/**)     │
│   - Declarative Resources: data/texts/*, data/config.json,                 │
│     data/playbook/*, pack.json, manifest.json                              │
│   Responsibilities: Two-layer state machine, gate predicates, reactive     │
│   hooks, tool reactors, narrative receipts, and HarnessAction projection.  │
├────────────────────────────────────────────────────────────────────────────┤
│ Layer 0 · Deterministic Rule Kernel, Sandbox & Storage Port                │
│   - Rule Kernel (ygg-core) + Schema Validator (ygg-schema)                 │
│   - Deterministic Lua 5.4 Sandbox & Host Contract (ygg-sandbox)            │
│   - Single-File Transactional Store & VFS (ygg-store -> run.sqlite)        │
│   Responsibilities: Atomic (state, event) -> state' transitions,           │
│   savepoint rollback, instruction fuel enforcement, VFS & tool bridging.   │
└────────────────────────────────────────────────────────────────────────────┘
```

### 1.1 跨治理引擎能力协商矩阵

由于 Layer 1（`agent-seed`）发出自包含的 `HarnessAction` 规格（`summary.jobs[i].subagentAction`），并经 Layer 0（`ygg.tool.call`）内联执行确定性仪式，任何编码 agent 宿主都能按其能力层级驱动 Yggdrasill：

| 层级 | 宿主环境 | 子代理执行与恢复 | 上下文遥测与压缩 | 集成机制 |
| --- | --- | --- | --- | --- |
| **Tier 1 · 主会话直驱** | Pi Coding Agent 等原生 Agent 宿主 | 主会话直驱 egg CLI + 原生 `Agent` 工具（`resume`），自动回退到 `fullFallbackBrief` | 实时 token 比例（`soft >= 60%`、`hard >= 75%`），稳定检查点写 `artifacts/handoff-latest.md` | 常驻能力 = 宿主原生 `egg --daemon` / `serve` |
| **Tier 2 · CLI / Hook 运行器** | Claude Code、Codex CLI、Cursor | 子进程调用或原生任务工具，消费 `subagentAction.brief` 与 `submitCommand` | 在稳定检查点由 Hook 触发的 `handoff_compact` | `egg --daemon` 或单次 `--resume --event` CLI |
| **Tier 3 · 提示委派** | 通用 LLM 聊天 / 自定义 Agent 循环 | 协调者把 `subagentAction.brief` 复制进子代理上下文并转达 `proposal.submit` | 经 `vfs.write` 手动检查点交接 | 单次 CLI 回执（缺省人读文本；`--raw` 显式旗标输出单行 JSON 信封）（`narrative` + `pendingChoice`） |

### 1.2 Adapter 一致性契约（任何 harness 都必须满足的五条义务）

三层架构的薄弱点在第 2 层：`subagentAction` 只是 `meta.summary` 里的**数据**，dispatcher 完全由 adapter 自行实现，于是「Mod 产出的 brief 被消费」不能只写成注释约定。本节把它固化为**可断言的义务规范**：落点由各 harness adapter 自行实现；本仓库当前无内置 adapter（执行壳已删除，主会话直驱 egg CLI + 原生 Agent 工具）。

| # | 义务 | 违反时的可见症状 |
| --- | --- | --- |
| **C1** | 派发给 subagent 的 prompt 必须含 `Mandatory references:` 与 `Submit:` 两条**行首锚线**（`^Submit:\s*\S` 要求后续非空） | 回执 `[ADAPTER CONTRACT] <rN> <stage>: …`，**不 spawn**，结果进 `degraded` |
| **C2** | `deltaPrompt` 若被单独使用，必须可判定；不可判定时**不得消费**（fail-closed），退回完整 `brief` | `delta-unassessable`；回执写 `[ADAPTER NOTE]` |
| **C3** | 任何一次派发都必须把**实际使用的 prompt 逐字节**落进 `artifacts/<rN>-<stage>-brief.md` | `[BRIEF REFUSED] <path>：<原因>` 进回执，并计入 `degraded` |
| **C4** | 归档/清理结论必须诚实：「查过且查空」或「没有库可查」才算证据，不得以「工具跑过」充当 `0/0`——不知道落点时宁可说没查，不谎报查空（现由 D-MA21 提示词方案承载，声明制见 §2.6 `settlement.sessions`） | 非证据态写 `sessions=0/0/unsettled:<kind>` |
| **C5** | 每一份 brief 都以 `## 续跑承接（Resume Handover）` 段收尾（`basis` / 前任执行者 / 未落地清单 / 硬约束 / 已归档制品） | `brief-missing-resume-handover`；无 `agentId` 时 `resume` 无法等价 |

**锚线为什么必须是「行首前缀」而不是 `includes`**：`job.description` 是多行需求正文，正文里完全可能出现 `Mandatory references: …` 这几个字，用任意 `includes` 判定会被一句话绕过——那是自指类断言的经典空转。**两条路必须同形状**：Core 的 `subagent_brief` 模板（`data/texts/` 片段装配）带这两行，adapter **本地拼装**的 brief 也必须带。**`resume` 的等价物是制品**（`artifacts/<rN>-<stage>.md` 加门禁证据），续跑承接段只是让下一个执行者（可能是另一个 harness、另一个上下文窗口）知道上一任为什么停在门外。

---

