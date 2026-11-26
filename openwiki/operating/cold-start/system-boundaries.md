---
type: "Reference"
title: "产品形态与三层系统边界 — 谁能驱动 Yggdrasill，靠什么契约"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-9706fc11c20facdfc630a8e9
    resource: repo://.ygg/mods/agent-seed/src/domain/readset.lua
  - id: openwiki-source-1c80d889f00dcfd1c85bbf4a
    resource: repo://.ygg/mods/agent-seed/src/domain/readsig.lua
  - id: openwiki-source-f9972c2cebb57d40e0aa06d9
    resource: repo://crates/ygg-sandbox/examples/egg/verify.rs
  - id: openwiki-source-422bb41f0e2d55ebad3aceea
    resource: repo://crates/ygg-sandbox/src/host_contract/agent.rs
  - id: openwiki-source-55ac8b91f1869ef9305b6b62
    resource: repo://crates/ygg-sandbox/src/host_contract/settlement.rs
  - id: openwiki-source-5629bb47afcaf3f792dfc0bc
    resource: repo://crates/ygg-sandbox/src/playtest.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 产品形态与三层系统边界 — 谁能驱动 Yggdrasill，靠什么契约

> **id** cold-start/system-boundaries · **幕** all · **触发** 冷启动 / 需要「系统由哪几层构成」的全景 / 改三层边界与 Adapter 义务之前
> **读不到代价** silent-drift（层边界讲错 → 后续所有契约解释失真）
> **replaces**：`docs/context/cold-start/system-boundaries.md`（搬运基线 `3a2536f`，原 DESIGN §1 L11–65）
> **上游**：[overview.md](../../overview.md) · **相关**：[vision.md](vision.md) · [../../contracts/host/cli-daemon.md](../../contracts/host/cli-daemon.md)

Yggdrasill 把**确定性规则执行**、**声明式工作流语义**与**宿主环境集成**分成三个严格有界的层。

## 1. 三层

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ Layer 2 · Presentation & Harness Adapter Shell                           │
│   Game Clients: Web (React + ygg-wasm), Godot 4 (gdext)                  │
│   Agent Adapters: 无内置执行壳（主会话直驱 egg CLI + 原生 Agent 工具）     │
│   CLI Daemon Consumers: Claude Code / Codex / Cursor hooks               │
│   Responsibilities: UI 渲染、subagent 进程派发/恢复、上下文遥测与压缩、   │
│                     数据红线防火墙                                        │
├──────────────────────────────────────────────────────────────────────────┤
│ Layer 1 · Playable Mod Package（.ygg/mods/agent-seed, test-adventure）    │
│   Executable Logic: mod.lua（唯一装配入口）+ 纯模块 src/**                │
│   Declarative: data/texts/*, data/config.json, pack.json, manifest.json  │
│   Responsibilities: 双层状态机、门禁判定、响应式 hooks、工具反应器、     │
│                     叙述回执、HarnessAction 投影                          │
├──────────────────────────────────────────────────────────────────────────┤
│ Layer 0 · Deterministic Rule Kernel, Sandbox & Storage Port               │
│   ygg-core（规则内核）· ygg-schema（Schema 校验）                        │
│   ygg-sandbox（Lua 5.4 确定性沙箱 + 宿主契约）                            │
│   ygg-store（单文件事务存储 + VFS → run.sqlite）                          │
│   Responsibilities: 原子 (state, event) -> state'、存档点回滚、          │
│                     VFS 与工具桥                                          │
└──────────────────────────────────────────────────────────────────────────┘
```

**层间的唯一方向**：Layer 1 发出自包含的 `HarnessAction` 规格（`summary.jobs[i].subagentAction`），经 Layer 0 的 `ygg.tool.call` 内联执行确定性仪式；Layer 2 只做「怎么把 brief 送到执行者手里」与「怎么渲染」，**不持有工作流语义**（P11）。

## 2. 跨治理引擎能力协商矩阵

任何编码 agent 宿主都能按自己的能力层级驱动 Yggdrasill，因为 Mod 产出的是**数据**（brief + 提交命令），不是一个写死的调用。

| 层级 | 宿主环境 | 子代理执行与恢复 | 上下文遥测与压缩 | 集成机制 |
| --- | --- | --- | --- | --- |
| **Tier 1 · 主会话直驱** | Pi Coding Agent 等原生 Agent 宿主 | 主会话直驱 egg CLI + 原生 `Agent` 工具（`resume`），必要时回退 `fullFallbackBrief` | 实时 token 比例（`soft ≥ 60%`、`hard ≥ 75%`），稳定检查点写 `artifacts/handoff-latest.md` | `egg open <run-root> --daemon` / `egg serve` |
| **Tier 2 · CLI / Hook 运行器** | Claude Code、Codex CLI、Cursor | 子进程调用或原生任务工具，消费 `subagentAction.brief` 与 `submitCommand` | 稳定检查点由 Hook 触发 `handoff_compact` | `egg open <run-root> --daemon` 或单次 `egg open <run-root> --event` |
| **Tier 3 · 提示委派** | 通用 LLM 聊天 / 自定义 Agent 循环 | 协调者把 `subagentAction.brief` 复制进子代理上下文，并转达 `proposal.submit` | 经 `vfs.write` 手动检查点交接 | 单次 CLI 回执（缺省人读文本；`--raw` 输出单行 JSON 信封） |

## 3. Adapter 一致性契约（五条义务）

三层架构的薄弱点在第 2 层：`subagentAction` 只是 `meta.summary` 里的**数据**，dispatcher 由 adapter 自行实现，于是「Mod 产出的 brief 被消费」不能只写成注释——它必须是**可断言的义务**。

| # | 义务 | 违反时的可见症状 |
| --- | --- | --- |
| **C1** | 派发给 subagent 的 prompt 必须含 `Mandatory references:` 与 `Submit:` 两条**行首锚线**（`^Submit:\s*\S`，后续非空） | 回执 `[ADAPTER CONTRACT] <rN> <stage>: …`，**不 spawn**，结果进 `degraded` |
| **C2** | `deltaPrompt` 单独使用时必须可判定；不可判定**不得消费**（fail-closed），退回完整 `brief` | `delta-unassessable`；回执写 `[ADAPTER NOTE]` |
| **C3** | 任何一次派发都必须把**实际使用的 prompt 逐字节**落进 `artifacts/<rN>-<stage>-brief.md` | `[BRIEF REFUSED] <path>：<原因>` 进回执，计入 `degraded` |
| **C4** | 归档 / 清理结论必须诚实：「查过且查空」或「没有库可查」才算证据，不得以「工具跑过」充当 `0/0` | 非证据态写 `sessions=0/0/unsettled:<kind>` |
| **C5** | 每份 brief 以 `## 续跑承接（Resume Handover）` 段收尾（`basis` / 前任执行者 / 未落地清单 / 硬约束 / 已归档制品） | `brief-missing-resume-handover`；无 `agentId` 时 `resume` 无法等价 |

**锚线为什么必须行首前缀而不是 `includes`**：`job.description` 是多行需求正文，正文里完全可能出现 `Mandatory references: …` 这几个字；任意 `includes` 判定会被一句话绕过——那是自指类断言的经典空转。**两条路必须同形状**：Core 的 `subagent_brief` 模板带这两行，adapter **本地拼装**的 brief 也必须带。

**`resume` 的等价物是制品**：`artifacts/<rN>-<stage>.md` 加门禁证据；续跑承接段只是让下一个执行者（可能是另一个 harness、另一个上下文窗口）知道上一任为什么停在门外。

## 4. 判据

① 阻断——在 Layer 2 里实现工作流语义（违 P11）、把 brief 的生成搬进 adapter（违 P2）、用 `includes` 判锚线。② 症状——回执出现 `[ADAPTER CONTRACT]` / `[BRIEF REFUSED]` 却仍 spawn、`degraded` 增长而无人看、brief 缺续跑承接段。③ 确认——`grep -rn 'Mandatory references:' artifacts/*-brief.md` 每条 brief 首部命中、`[BRIEF REFUSED]` 计数为 0、`degraded` 只在真实降级时非空。

维护责任 = 改 `host_contract/{agent,settlement}.rs`、adapter 实现或本页正文的执行者；触发 = 层级职责、Tier 矩阵或 C1–C5 变更；检查 = `crate 的 adapter 契约用例` + 本页五条义务与代码中错误码逐字一致。
