# Yggdrasill · 架构与运营原则（P 列表）

> **定位**：P1–P11 工程原则与 PLAY 家族注记，约束一切实现、评审与文档组织。
> **replaces**：CONTEXT §3（L62–96）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[topology.md](topology.md) · [../design/system-boundaries.md](../design/system-boundaries.md)
> **搬运**：以下正文自 CONTEXT.md L62–L96 逐字迁移，不重写语义。

## 3. 架构与运营原则（工程面 P 列表 + 游玩面 PLAY 节）

1. **P1 · 纯函数式迁移与事务性存档点（`(state, event) -> state'`）**
   - 每一次 dispatch 都是一次全有或全无的原子迁移。若某个 Lua 处理器出错或耗尽指令燃料，该次 dispatch 期间对实体表与 KV 存储的全部修改都会被完全回滚。在 SQLite 持久化模式下，事件日志行与状态快照在单个数据库事务中提交。
2. **P2 · 零硬编码 — 数据与契约驱动（`D11/D15`）**
   - 业务规则、状态迁移、叙述模板与提示节拍都归属于 Mod 脚本（`src/**`，按 core/domain/view/runtime/handlers 五层组织）与 JSON 资源（`data/*.json`），绝不硬编码在 Rust 宿主代码或 TypeScript 扩展代码中。所有提示模板、子代理 brief 与叙述行都存放在 `data/texts/` 片段（由 `mod.lua` 装配回 TEXTS）。
3. **P3 · 契约驱动的通用宿主**
   - Rust 宿主（`egg`）与内核（`ygg-sandbox`）对特定 Mod 的事件名、阶段名或叙述前缀一无所知。一切都在 `pack.json`（`agent`、`inboxRoutes`、`tools`、`code`）与 `data/config.json`（`drive`、`settlement`、`allowedTools`）中声明。缺失或畸形的契约字段在加载时即失败关闭（fail closed），零回退默认值。
4. **P4 · 少即是多 — 每个机制一条规范路径**
   - 每个能力都有唯一的规范机制。冗余的校验块、重复的 Mod 副本、或重叠的多回合异步仪式，都被合并为单源辅助函数与 1 回合内联仪式。
5. **P5 · 单一事实来源（Single Source of Truth）**
   - **Schema**：`packages/schema/schemas/` 是 JSON Schema 的唯一权威（由 Rust 与 TypeScript 共享）。
   - **Mod**：`.ygg/mods/agent-seed` 是唯一的物理 Mod 目录；`packages/mods/agent-seed` 是指向其的相对符号链接（`../../.ygg/mods/agent-seed`）。
   - **运行状态**：`<run-dir>/run.sqlite` 是新运行的单一存储文件。
   - **文档**：`docs/` 下仅有一个受索引约束的 LLM Wiki 树（入口 `docs/README.md`；领域与决策在 `docs/context/`，架构与契约在 `docs/design/`；旧两单文件为薄壳索引，r2）。
6. **P6 · 模块顶层纯度（`YGG-MOD-EFFECT`）**
   - 在多文件 Mod 中，`mod.lua` 是唯一的装配入口。支撑模块（`src/**` 五层）在顶层只能导出纯函数与常量；在模块顶层调用有副作用的宿主 API（`ygg.on`、`ygg.emit`、`ygg.narrate`、`ygg.choice`、`ygg.kv.set`、`ygg.entity.*`、`ygg.fsm.define`）会在静态阶段（`ygg-lint`）与加载阶段（`ygg-sandbox`）被双重拒绝。这保证了处理器注册顺序（进而 dispatch 顺序）由 `mod.lua` 显式控制，而从不取决于 `require` 顺序。
7. **P7 · 响应式收敛效应先于选择（Choice）**
   - 一旦调用 `ygg.choice`，内核就终止当前迁移并丢弃任何仍排队的内置事件。因此，所有响应式副作用（`hooks.useEffect`，如 `auto_dispatch`、`tools.receipt`、`tools.reactive`）必须在 `render()` 顶部、在发出 `ygg.choice` 之前先冲刷并收敛（`MAX_PASSES = 4`）。
8. **P8 · 可选加入的能力降级（Wasm 纯度 vs 本地宿主能力）**
   - Lua 沙箱默认严格隔离（无 I/O、无时钟、确定性 PRNG、指令燃料上限），使 Mod 在浏览器 Wasm 中运行结果完全一致。本地宿主能力（`ygg.fs` VFS 与 `ygg.tool.call` 同步工具运行器）通过可选的宿主 trait 注入；当未挂载时，Mod 优雅降级而不破坏状态语义。
9. **P9 · 保留构建缓存的持久化泳道工作区**
   - 并行子代理泳道跨需求与跨运行复用持久化 worktree 目录（r1/D-MA24 起按池：`<pool-root>/<repo>-lane-<k>`，默认池 + `lane.pool` 注册的具名池）。认领泳道时将已跟踪文件重置为 `HEAD`，同时保留未跟踪的构建目录（`target/`、`node_modules/`），在保持分支隔离的同时避免冷编译惩罚。
10. **P10 · 数据红线与显式的每运行目标授权**
    - **数据红线**：读取、列出、stat、或复制真实业务数据（`*.xlsx`、`*.xls`、`*.csv`，或诸如 `/Volumes/WenshuSpace/**` 的涉密挂载）在提示层与扩展 `tool_call` 防火墙层都被严格禁止。
    - **决策归属**：`guide` 模式是默认（里程碑动作需要通过 `ygg.choice_selected` 给出人类菜单索引）。运行时切换到 `goal` 模式会把派发权移交给主 agent，要求在**每批盘点结算与退役**（常驻下 review.note 每批都走）之前先记录一条每运行的 `config.set goal.authorization=<note>`；若在授权注册之前就进入该模式，会立即告警（`[GOAL AUTH PENDING]`）。
11. **P11 · 宿主零语义（引擎/壳分界）**
    - 宿主（内核与 CLI/服务壳）不持有、不解析任何 Mod 领域概念；跨 Mod 复用的只有通用机制
      （存储、事件、路由、裸数据服务）。领域概念出现在宿主面即层次错位。
    - 来源：r8（serve 面板两级化——需求/泳道/门禁是特定 mod 的概念，宿主解析即限制 mod 自由度）；
> P11-P15——PLAY 不是工程原则的延续而是另一维度，同时不挤占未来工程原则编号。
> PLAY5 与 P10 数据红线同属「显式声明边界」家族；P11↔P2（零硬编码的宿主面投影）；

---

## 运行时台词与文案契约索引（r2 新增）

P2（零硬编码）的文档面投影：mod 的叙事与文案**知识权威**在 context 运行流程层四件，mod `data/texts/` 仅为运行时投影——

- [workflow-acts.md](workflow-acts.md)：五幕进行时台词（`scenes`/`prompts`/`<node>_common` 正文）+ 无 egg 执行章；
- [dispatch-ritual.md](dispatch-ritual.md)：派发仪式（`subagent_brief` 策略段、`config_opened` 面板语义）；
- [cleanup-policy.md](cleanup-policy.md)：盘点清理政策（会话/待办清理正文）；
- [copy-contract.md](copy-contract.md)：413 键句式契约表（错误码/回执/台账模板 + 协议前缀归属）。

改文案**先改上述文件再同步 mod**（单向同步纪律，见 [../wiki-governance.md](../wiki-governance.md)）；本节只放指针不搬正文，防双源漂移。

---

