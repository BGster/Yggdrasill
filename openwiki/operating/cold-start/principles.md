---
type: "Reference"
title: "架构与运营原则（P1–P11）— 约束一切实现、评审与文档组织"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-ea70eb6c045047448e446296
    resource: repo://.gitignore
  - id: openwiki-source-22b30a4cbdd3f021cc6c2529
    resource: repo://.ygg/mods/agent-seed/src/runtime/flow.lua
  - id: openwiki-source-58ff0a3a16c1d28f634ce8b3
    resource: repo://crates/ygg-core/src/transition.rs
  - id: openwiki-source-683c84ca41d2dbee8643a1d8
    resource: repo://crates/ygg-lint/src/rules.rs
  - id: openwiki-source-c41ca27b57f3fe5db379dae4
    resource: repo://crates/ygg-sandbox/src/host_contract/mod.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-6556f80f4ad5bd14105c33c6
    resource: repo://crates/ygg-sandbox/src/pack.rs
  - id: openwiki-source-2bd7560039b90f86b6e54851
    resource: repo://crates/ygg-store/src/sqlite/mod.rs
  - id: openwiki-source-23147ebf2408129337e69a1d
    resource: repo://packages/schema/schemas/manifest.json
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 架构与运营原则（P1–P11）— 约束一切实现、评审与文档组织

> **id** cold-start/principles · **幕** cold-start · **触发** 冷启动 / 设计取舍前
> **读不到代价** hard-refusal（取舍、评审与文档组织都以这十一条为准；不知道它们就会给出与仓库相反的方案）
> **replaces**：`docs/context/cold-start/principles.md`（搬运基线 `3a2536f`，原 CONTEXT §3 L62–96）
> **上游**：[overview.md](../../overview.md) · **相关**：[topology.md](topology.md) · [system-boundaries.md](system-boundaries.md) · [../../governance.md](../../governance.md)

## 1. 工程面

**P1 · 纯函数式迁移与事务性存档点（`(state, event) -> state'`）**
每一次 dispatch 都是全有或全无的原子迁移：处理器出错或耗尽指令燃料，该次 dispatch 期间对实体表与 KV 的全部修改**完全回滚**。SQLite 持久化模式下，事件日志行与状态快照在**单个数据库事务**里提交。

**P2 · 零硬编码——数据与契约驱动**
业务规则、状态迁移、叙述模板、提示节拍都归属 Mod 脚本（`src/**`，core / domain / view / runtime / handlers 五层）与 JSON 资源（`data/*.json`），**绝不**硬编码在 Rust 宿主或 TypeScript 扩展里。所有提示模板、子代理 brief 与叙述行都在 `data/texts/` 片段，由 `mod.lua` 装配回 TEXTS。

**P3 · 契约驱动的通用宿主**
Rust 宿主（`egg`）与内核（`ygg-sandbox`）对特定 Mod 的事件名、阶段名、叙述前缀**一无所知**。一切在 `pack.json`（`agent`、`inboxRoutes`、`tools`、`code`）与 `data/config.json`（`drive`、`settlement`、`allowedTools`）里声明。缺失或畸形的契约字段**加载即失败关闭**，零回退默认值。

**P4 · 少即是多——每个机制一条规范路径**
每个能力只有一个规范机制。冗余校验块、重复 Mod 副本、重叠的多回合异步仪式，一律合并为单源辅助函数与一回合内联仪式。

**P5 · 单一事实来源（Single Source of Truth）**

- **Schema**：`packages/schema/schemas/` 是 JSON Schema 的唯一权威（Rust 与 TypeScript 共享）。
- **Mod**：`.ygg/mods/agent-seed` 是唯一物理 Mod 目录；`packages/mods/agent-seed` 是指向它的相对符号链接（`../../.ygg/mods/agent-seed`）。
- **运行状态**：`<run-dir>/run.sqlite` 是新运行的单一存储文件。
- **知识**：仓库只有一个知识库 `openwiki/`（入口 [overview.md](../../overview.md)），治理见 [governance.md](../../governance.md)。

**P6 · 模块顶层纯度（`YGG-MOD-EFFECT`）**
多文件 Mod 中 `mod.lua` 是**唯一装配入口**。支撑模块（`src/**` 五层）顶层只能导出纯函数与常量；在模块顶层调用有副作用的宿主 API（`ygg.on` / `ygg.emit` / `ygg.narrate` / `ygg.choice` / `ygg.kv.set` / `ygg.entity.*` / `ygg.fsm.define`）会被**静态阶段**（`ygg-lint`）与**加载阶段**（`ygg-sandbox`）双重拒绝。这保证处理器注册顺序（进而 dispatch 顺序）由 `mod.lua` 显式控制，从不取决于 `require` 顺序。

**P7 · 响应式收敛效应先于选择（Choice）**
一旦调用 `ygg.choice`，内核终止当前迁移并丢弃仍在排队的内置事件。因此所有响应式副作用（`hooks.useEffect`：`auto_dispatch`、`tools.receipt`、`tools.reactive`）必须在 `render()` 顶部、在发出 `ygg.choice` **之前**冲刷并收敛（`MAX_PASSES = 4`）。

**P8 · 可选加入的能力降级（Wasm 纯度 vs 本地宿主能力）**
Lua 沙箱默认严格隔离（无 I/O、无时钟、确定性 PRNG、指令燃料上限），使 Mod 在浏览器 Wasm 里结果一致。本地宿主能力（`ygg.fs` VFS 与 `ygg.tool.call` 同步工具运行器）通过可选宿主 trait 注入；未挂载时 Mod**优雅降级而不破坏状态语义**。

**P9 · 保留构建缓存的持久化泳道工作区**
并行子代理泳道跨需求与跨运行复用持久化 worktree（按池：`<pool-root>/<repo>-lane-<k>`，默认池 + `lane.pool` 注册的具名池）。认领泳道时把已跟踪文件重置到 `HEAD`，同时保留未跟踪的构建目录（`target/`、`node_modules/`）——分支隔离但不吃冷编译。

**P10 · 数据红线与显式的每运行目标授权**

- **数据红线**：权威正文见仓库根 `AGENTS.md`；真实业务数据及其目录元信息一律不得由 agent 读取，需真实数据的验证由用户运行并贴回。本节只留指针。
- **决策归属**：`guide` 是缺省（里程碑动作需要 `ygg.choice_selected` 的人类菜单索引）；运行时切 `goal` 会把派发权移交主代理，要求在**每批盘点结算与退役**之前记录 `config.set goal.authorization=<note>`；授权注册前就进入该模式会立即告警（`[GOAL AUTH PENDING]`）。操作正文见 [../acts/coordinate.md](../acts/coordinate.md)。
- **注册纪律（用前注册）**：凡「用前注册」参数——`executor.<rN>.<stage>`、`profile.<handle>.*`、`goal.authorization`、`lane.pool`、`tools.root` / `tools.session` / `tools.sessions_root`——一律经 `config.set`（或专用注册动作）在使用前写入；未注册 = 消费点可读拒绝或提示（`[GOAL AUTH PENDING]` / `[TOOLS UNREGISTERED]` / `lane_pool_unknown`），**绝不静默假成功**。

**P11 · 宿主零语义（引擎/壳分界）**
宿主（内核与 CLI / 服务壳）不持有、不解析任何 Mod 领域概念；跨 Mod 复用的只有通用机制（存储、事件、路由、裸数据服务）。领域概念出现在宿主面即层次错位（来源：r8 serve 面板两级化）。

## 2. PLAY 家族注记

PLAY 不是工程原则的延续，而是**另一维度**，也不挤占未来工程原则编号。PLAY5 与 P10 数据红线同属「显式声明边界」家族；P11 ↔ P2 是零硬编码的宿主面投影。

## 3. 运行时台词与文案契约索引

P2 的文档面投影：mod 的叙事与文案**知识权威**在 `operating/` 运行流程层四件，mod `data/texts/` 仅为运行时投影——

- [../acts/](../acts/)：五幕进行时台词（`scenes` / `prompts` / `<node>_common` 正文）+ 无 egg 执行章；
- [../rituals/dispatch.md](../rituals/dispatch.md)：派发仪式（`subagent_brief` 策略段、`config_opened` 面板语义）；
- [../rituals/cleanup.md](../rituals/cleanup.md)：盘点清理政策（会话 / 待办清理正文）；
- [../scenes/README.md](../scenes/README.md)：七域文案契约索引（409 beats + core 36 = 445 唯一键；错误码 / 回执 / 台账模板 + 协议前缀归属）。

改文案**先改上述页面再同步 mod**（单向同步纪律见 [governance.md](../../governance.md)）；本节只放指针、不搬正文。

## 4. 输出与指针纪律

- **语言分工**：对用户说中文；代码、标识符、路径与命令保持原样英文。
- **结论先行**：报错或下结论时先说结论，证据跟在后面。
- **编辑前先读**：改一个文件之前先读它；一次改多个文件时逐文件说明改了什么、为什么。
- **大批量机械改写的回环**：先打印结果 → 再写入 → **立刻读回验证**。这类改写的失效是静默的（整段被吞、编辑锚点截断误删、表格错行），不读回就发现不了。
- **指针不复制正文**：指针槽只放指针，**永不复制**被指向页面的正文。指针表索引「已知目的地」——先经索引定位到真文本再动手，**永不凭记忆复述**（记忆会过期，页面会变）。

## 5. 凭据与数据卫生

- **`credentials.no-secret-in-artifacts`**（must-not）：凭据绝不进任何会话产物——报告、摘要、issue / PR 描述、粘贴的日志片段。
- **`credentials.no-secret-in-committed-resources`**（must-not）：敏感信息不得进入已提交资源，显式包括：`data/config.json`、playbook 资源（装载时装配的头与章节片段）、演进 Mod 及其输出、任何随仓提交的 Mod 资源或文档。
- **`credentials.settings-not-a-secret-vault`**（must）：运行时 KV 家族 `settings.*` 不是保险箱——静态缺省（`data/config.json`）与动态覆盖（`config.set` 写入的 `settings.*`）只承载非敏感开关、数字与经验文本（如 `evolution.mode` / `gates.require_all_green` / `evolution.max_per_run`）。
- **`credentials.secret-values-local-only`**（must）：敏感值——LLM API key、token、数据库 / 对象存储连接串、子代理模型参数（`YGG_SUBAGENT_*`）——只活在本地 `.env` / `.env.local`（both git-ignored），不在任何其它文件。
- **`credentials.mod-and-report-keys-only`**（must）：Mod 资源与报告只显示**键名**（如 `YGG_LLM_*`、`YGG_SUBAGENT_MODEL`），永不显示值——包括叙述文本、`beats` / `prompts`、收尾评审与任何走查报告。
- **`credentials.evolution-output-not-exempt`**（must-not）：局末演进输出（建议文本、评审工件、由它做出的资源编辑）同样属于已提交面。
- **`credentials.synthetic-data`**（must）：示例与测试数据一律合成；绝不引入真实个人数据。
- **`credentials.breach-disclosure`**（must）：违规后如实列出暴露了什么（具体文件 + 内容位置），不淡化、不辩解；随后停止受影响的操作并等待用户。

## 6. 判据

① 阻断——在 Rust 宿主里写 Mod 领域概念（违 P3 / P11）、在模块顶层调宿主副作用 API（违 P6）、新增第二份权威（违 P4 / P5）、把敏感值写进仓内资源（违第 5 节）。② 症状——同一机制两条实现路径、`data/texts/` 与 `operating/` 各写一份文案、`[GOAL AUTH PENDING]` 在结算时才出现。③ 确认——`ygg-lint` 与加载期双重纯度检查为绿、`grep` 敏感键名只出现在 `.env*`、每条 P 原则在本库或 `AGENTS.md` 有唯一权威句。

维护责任 = 改本页或触及以上任一原则的执行者；触发 = 新增 / 合并原则，或原则的载体搬家；检查 = 本页 11 条编号连续、每条在代码或本库有对应断言。
