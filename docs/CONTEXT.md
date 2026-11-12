# Yggdrasill · 领域语境、愿景、决策与核心原则（`docs/CONTEXT.md`）

> **定位**：`docs/CONTEXT.md` 是 Yggdrasill 仓库**始终最新的领域知识、愿景、原则与架构决策台账（Architectural Decision Ledger）权威文档**——记录的是**当前生效状态**，不是流水账：决策变更就原地修订对应行、概念演进就就地改写对应条目，任何变更叙事与历史都归 git 与结算产物，不进本文档。覆盖范围是**整个仓库**（内核 crates / egg CLI / Mod / serve / 工具链），按章节对应各子系统，不局限于 Mod。它与 [`docs/DESIGN.md`](DESIGN.md)（负责规定产品形态、系统行为与接口契约）共同取代了 `docs/` 下所有遗留的碎片化文档。每一次规划（`plan`）与实现（`development`）任务，都必须在提议或修改代码之前先查阅 `docs/CONTEXT.md` 与 `docs/DESIGN.md`；任何架构或契约变更都必须在同一次迭代中、在同一特性分支上同步更新这两份文档的对应章节，之后才能进入最终 `review` 结算。

---

## 术语速览（Glossary）

高频术语的中英对照速览，完整定义见 [§2 核心领域词汇](#2-核心领域词汇)：

| 术语 | 中文释义 |
| --- | --- |
| `Run` | 运行实例 |
| `stage`（Coordinator Stage） | 协调者宏观阶段（`coordinate` → `review` → `done`；r2 rev2 B 裁决后 boot 直落 `coordinate`，`intake` 阶段已注销） |
| `Requirement`（`r1..rN`） | 需求（交付单元） |
| `Lane`（Persistent Lane Pool） | 持久化泳道工作树池 |
| `Proposal`（`p1..pM`） | 提案（不可变交付提交） |
| `Gate`（`plan_gate` / `accept_gate`） | 强制门禁 |
| `Executor Ledger` | 执行者台账 |
| `HarnessAction`（`subagentAction`） | 治理引擎动作指令 |
| `Stable Checkpoint` | 稳定检查点 |

---

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

## 2. 核心领域词汇

| 术语 | 领域定义 |
| --- | --- |
| **Run（运行实例）** | 由 `run_id` 标识的单个隔离执行实例，位于 `.ygg/runs/<run_id>/` 下，持久化于单个 SQLite 数据库（`run.sqlite`），内含事件日志、状态快照、收件箱队列与虚拟文件系统产物。r9 起运行是**常驻自循环**：运行级终局不再由 review.note 自动触发，而是周期性批结算 + 显式退役（`done` 仅经 `run.retire` 武装可达）；升级不迁移旧局（装载失败即开新局）。 |
| **Coordinator Stage（`stage`，协调者宏观阶段）** | 一次 Run 的宏观生命周期（r2 rev2 B 裁决：`intake` 阶段与 `task.intake` 事件已注销，冷启动章程由 `event.start` 启动载荷承载，boot 直落 `coordinate`）：`coordinate`（常驻主态：派发泳道、接受提案、通过门禁）$\to$ `review`（批盘点相位：验证套件、演进筛选、会话/todo 清理）$\to$ 批收口回 `coordinate`（`run.batch` +1）；`done`（退役终态，仅 `run.retire` 武装后的 review.note 可达）。需求级 `done` 与运行退役态只是字面量同形（语义正交）。 |
| **Requirement（`r1..rN`，需求）** | 由状态机跟踪的一个交付单元。以 5 个规范状态（`draft`、`planning`、`developing`、`done`，外加终态旁路状态 `split` / `merged` / `dropped`）存储，并通过 `plan_ok`、`job`、`gate`、`conflict` 属性投影为细粒度展示阶段（`draft`、`planning`、`plan_review`、`plan_gate`、`ready`、`developing`、`dev_review`、`accept`、`done`、`blocked`）。 |
| **Persistent Lane Pool（`lane-1..4`，持久化泳道池）** | 一个有界池，最多 4 个可复用的 Git worktree（`~/tmp/worktrees/<repo>-lane-<k>`）。认领泳道时将其分支重置为 `HEAD` 但不删除未跟踪的构建缓存（`target/`、`node_modules/`）；释放泳道时将其停放在 detached `HEAD` 上，从而永不锁死分支。泳道分支名执行强制形态 `feature/{4位uuid}-{语义词段}`（`lane.claim` 与 `git.worktree_claim` 两条入口同规则，非法名可读拒绝；默认名生成机制已删除）——契约细节见 DESIGN §2.4。 |
| **Proposal（`p1..pM`，提案）** | 由执行者子代理针对特定 `(req_id, rev, stage)` 元组产出的不可变交付提交（`proposal.submit`），引用一个已归档的 VFS 产物（`artifacts/<req>-<stage>.md`）并绑定执行者身份（`exec=<agent_id>`）。提案在协调者串行执行 `proposal.accept` 或 `proposal.reject` 之前不会改变需求状态。 |
| **Mandatory Gates（`plan_gate`、`accept_gate`，强制门禁）** | 每个需求的两个不可跳过的质量检查点。`plan_gate` 批准技术方案（`planning` $\to$ `ready`）并响应式地自动派发开发。`accept_gate` 要求非空产物证据（`artifacts`）与匹配的修订号（`gate_rev == req.rev`），之后才将需求标记为 `done` 并释放其泳道。 |
| **Executor Ledger（`executor.<rN>.<stage>`，执行者台账）** | 一条自声明的审计轨迹，记录哪个子代理 ID 执行了某个需求的某个阶段（`analysis`、`plan`、`development`）。跨阶段交接或同一阶段更换为不同执行者 ID 需要显式理由（`<new_id>\|<reason>`）。占位 ID（`pi-ygg-harness`、`main`、`default`、`unregistered`）在恢复决策中被过滤掉。r3 起，台账 id 与提案的宿主 `actor=` 章在 `requirement.list` trace 上做三态互证显示（`[verified]` / `[mismatch:actor=…]` / `[unverified]`，识别不拒绝）——账本仍是权责权威，互证只是显示 join。 |
| **Profile Registry（`profile.<handle>.*`，身份注册表）** | 一张**自报身份卡**（self-reported registry，不是凭据）：`config.set profile.<handle>.<key>` 逐键注册的 `settings.profile.*` 平铺 KV（保留键 `harness` / `session_id` / `handle` / `role` / `assignments`，自由键 `^[a-z0-9_]{1,32}$`）。回答「这个句柄是谁」（harness / 原生 session id / 角色自报分工快照）；与执行者台账**互链不冗余**（渲染层按 `handle == executor id` 字符串 join，权责仍以账本为准）。投影：`map.show full` 的 `[PROFILES]` 行与 `agent.summary` 的 `profiles` 数组（≤8 张，handle 升序）。 |
| **In-Archive VFS（`artifacts/*`，归档内虚拟文件系统）** | `run.sqlite` 内的一张虚拟文件系统表（`vfs_files`），通过 `ygg.fs.*` 暴露给 Lua 沙箱，通过 `vfs.write` / JSON-RPC `vfs_put` 暴露给外部工具。存储方案、开发报告、演进审查与压缩交接，带 SHA-256 完整性。 |
| **Synchronous Tool Bridge（`ygg.tool.call`，同步工具桥）** | 一个可选加入的宿主能力，允许 Mod 在同一次 dispatch 回合内同步调用白名单内的确定性宿主仪式（`git.worktree_claim`、`git.lane_probe`、`git.worktree_release`、`fs.artifact_probe`、`gate.verify_suite`、`harness.archive_sessions`），消除多回合异步回调往返。 |
| **HarnessAction（`subagentAction`，治理引擎动作指令）** | Mod 在 `summary.jobs[i].subagentAction` 内发出的一条结构化机器可读指令（`subagent.spawn` 或 `subagent.resume`），携带渲染后的 `brief`、`deltaPrompt`、`fullFallbackBrief`、`worktreePath`、`artifactPath` 与 `submitCommand`，使任何上层治理引擎适配器都能执行泳道而无需重新实现工作流逻辑。 |
| **Stable Checkpoint（稳定检查点）** | 一个静止的运行状态（`available == true` $\land$ `pendingTools == 0` $\land$（`gates > 0` $\lor$ `jobs == 0` $\lor$ `conflicts > 0`）），在此状态下上下文压缩与结构化交接（`artifacts/handoff-latest.md`）可以安全发生而不丢失在途子代理输出。 |

---

## 3. 架构与运营原则（工程面 P 列表 + 游玩面 PLAY 节）

1. **P1 · 纯函数式迁移与事务性存档点（`(state, event) -> state'`）**
   - 每一次 dispatch 都是一次全有或全无的原子迁移。若某个 Lua 处理器出错或耗尽指令燃料，该次 dispatch 期间对实体表与 KV 存储的全部修改都会被完全回滚。在 SQLite 持久化模式下，事件日志行与状态快照在单个数据库事务中提交。
2. **P2 · 零硬编码 — 数据与契约驱动（`D11/D15`）**
   - 业务规则、状态迁移、叙述模板与提示节拍都归属于 Mod 脚本（`src/**`，按 core/domain/view/runtime/handlers 五层组织）与 JSON 资源（`data/*.json`），绝不硬编码在 Rust 宿主代码或 TypeScript 扩展代码中。所有提示模板、子代理 brief 与叙述行都存放在 `data/texts/` 片段（由 `mod.lua` 装配回 TEXTS）。
3. **P3 · 契约驱动的通用宿主**
   - Rust 宿主（`ygg-agent`）与内核（`ygg-sandbox`）对特定 Mod 的事件名、阶段名或叙述前缀一无所知。一切都在 `pack.json`（`agent`、`inboxRoutes`、`tools`、`code`）与 `data/config.json`（`drive`、`settlement`、`allowedTools`）中声明。缺失或畸形的契约字段在加载时即失败关闭（fail closed），零回退默认值。
4. **P4 · 少即是多 — 每个机制一条规范路径**
   - 每个能力都有唯一的规范机制。冗余的校验块、重复的 Mod 副本、或重叠的多回合异步仪式，都被合并为单源辅助函数与 1 回合内联仪式。
5. **P5 · 单一事实来源（Single Source of Truth）**
   - **Schema**：`packages/schema/schemas/` 是 JSON Schema 的唯一权威（由 Rust 与 TypeScript 共享）。
   - **Mod**：`.ygg/mods/agent-seed` 是唯一的物理 Mod 目录；`packages/mods/agent-seed` 是指向其的相对符号链接（`../../.ygg/mods/agent-seed`）。
   - **运行状态**：`<run-dir>/run.sqlite` 是新运行的单一存储文件。
   - **文档**：`docs/CONTEXT.md` 与 `docs/DESIGN.md` 是 `docs/` 下仅有的两份架构/设计文档。
6. **P6 · 模块顶层纯度（`YGG-MOD-EFFECT`）**
   - 在多文件 Mod 中，`mod.lua` 是唯一的装配入口。支撑模块（`src/**` 五层）在顶层只能导出纯函数与常量；在模块顶层调用有副作用的宿主 API（`ygg.on`、`ygg.emit`、`ygg.narrate`、`ygg.choice`、`ygg.kv.set`、`ygg.entity.*`、`ygg.fsm.define`）会在静态阶段（`ygg-lint`）与加载阶段（`ygg-sandbox`）被双重拒绝。这保证了处理器注册顺序（进而 dispatch 顺序）由 `mod.lua` 显式控制，而从不取决于 `require` 顺序。
7. **P7 · 响应式收敛效应先于选择（Choice）**
   - 一旦调用 `ygg.choice`，内核就终止当前迁移并丢弃任何仍排队的内置事件。因此，所有响应式副作用（`hooks.useEffect`，如 `auto_dispatch`、`tools.receipt`、`tools.reactive`）必须在 `render()` 顶部、在发出 `ygg.choice` 之前先冲刷并收敛（`MAX_PASSES = 4`）。
8. **P8 · 可选加入的能力降级（Wasm 纯度 vs 本地宿主能力）**
   - Lua 沙箱默认严格隔离（无 I/O、无时钟、确定性 PRNG、指令燃料上限），使 Mod 在浏览器 Wasm 中运行结果完全一致。本地宿主能力（`ygg.fs` VFS 与 `ygg.tool.call` 同步工具运行器）通过可选的宿主 trait 注入；当未挂载时，Mod 优雅降级而不破坏状态语义。
9. **P9 · 保留构建缓存的持久化泳道工作区**
   - 并行子代理泳道跨需求与跨运行复用持久化 worktree 目录（`<repo>-lane-1..4`）。认领泳道时将已跟踪文件重置为 `HEAD`，同时保留未跟踪的构建目录（`target/`、`node_modules/`），在保持分支隔离的同时避免冷编译惩罚。
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

## 4. Monorepo 模块拓扑与依赖白名单

```text
Yggdrasill/
├── crates/
│   ├── ygg-core/        # 纯确定性内核：实体表、事件总线、迁移驱动器（零内部依赖；仅 serde/serde_json/thiserror）
│   ├── ygg-modules/     # 纯多文件 Lua 模块路径解析与副作用禁用清单（零依赖叶 crate）
│   ├── ygg-schema/      # Manifest SHA-256 内容寻址与 JSON Schema 校验（仅依赖 ygg-core）
│   ├── ygg-store/       # 运行持久化端口：SQLite 默认（run.sqlite + VFS）与遗留 JSON 后端（仅作为 ygg-sandbox 示例的 dev 依赖）；observe.rs = serve 只读观察面
│   ├── ygg-sandbox/     # 确定性 Lua 5.4 VM（mlua）、宿主 API 白名单、ygg stdlib、ModBundle 加载器、宿主 CLI（examples/agent/，含 serve.rs 只读观察台与 assets/panel 内置两级列表页）
│   ├── ygg-lint/        # 静态 AST 检查器（full_moon）：YGG-R1/R2/R3、YGG-MOD-EFFECT、YGG-PARSE（仅依赖 ygg-modules，零内核/VM 依赖）
│   └── ygg-wasm/        # 浏览器 WebAssembly 绑定（wasm-bindgen 薄壳，位于 ygg-core + ygg-sandbox 之上）
├── packages/
│   ├── schema/          # JSON Schema 单一事实来源（manifest、pack、story、agent-config）
│   ├── mods/
│   │   ├── agent-seed -> ../../.ygg/mods/agent-seed         # 指向规范工作流 Mod 的符号链接
│   │   └── test-adventure/                                  # 参考可玩文字冒险 Mod
│   ├── client-web/      # React + TypeScript 浏览器播放器
│   └── pipeline/        # TypeScript AI Mod 生成与校验循环
├── clients/
│   └── godot/           # Godot 4 原生 gdext 客户端（M2.5）
├── .ygg/
│   └── mods/agent-seed/      # 规范多文件 Agent 治理 Mod（mod.lua、src/** 五层、data/texts|playbook|config、schemas/）
├── .pi/
└── docs/
    ├── CONTEXT.md       # 领域知识、愿景、原则与架构决策台账（本文件）
    └── DESIGN.md        # 系统架构、状态机语义与接口契约
```

### 硬性依赖不变量
- `ygg-core` 只能依赖 `serde`、`serde_json` 与 `thiserror`。任何 VM、Schema 校验器、I/O 或平台 API 都不得进入 `ygg-core`。
- `ygg-modules` 是被 `ygg-sandbox` 与 `ygg-lint` 共享的零依赖叶 crate。
- `ygg-lint` 绝不依赖 `ygg-core`、`ygg-schema` 或 `ygg-sandbox`（保证静态 AST 检查快速且免去 `mlua` 编译）。
- `ygg-store`（及 `rusqlite`）只被原生宿主 CLI（`crates/ygg-sandbox/examples/agent/main.rs`，经 `[dev-dependencies]`）链接，确保 `ygg-wasm` 与 `ygg-lint` 的依赖树 100% 不含 SQLite 与文件系统依赖。

---

## 5. 关键架构决策台账（`D1–D21` 与 `M-Agent` 演进）

> 任何新的项目级架构决策，或对既有决策的修订，都必须在实现它的同一次提交中记录到本表（**原地修订对应行**，编号顺延但决策行只改内容不追加变更叙事——流水账不进本文档，历史归 git 与结算产物）。

### 5.1 核心引擎与平台决策（`D1–D21`）

| ID | 决策领域 | 裁决与理由 | 状态 |
| --- | --- | --- | --- |
| **D1** | 初始运行时目标 | **Web 优先**：通过 `ygg-wasm` 实现纯浏览器可玩，表现与逻辑严格分离；微信小游戏与 Godot 作为后续目标。 | 已确认 |
| **D3** | 内核规格 | **从第 1 天起即为完整 RPG 级规则内核**（实体、属性、标签、事件总线、FSM、Lua 条件），使文字冒险通过增加内容/子系统演进为 RPG，而非重写内核。 | 已确认 |
| **D4** | 首个里程碑策略 | **先引擎 + 手写参考 Mod（`test-adventure`）**，待内核语义得到证明后再接 AI 生成流水线（`M1`）。 | 已确认 |
| **D6** | 游戏引擎集成 | **Godot 原生链接（`gdext`）** 排期于 M2.5（原生链接 Rust 库，无 Wasm 开销）；保持表现/逻辑分离。 | 已排期（Spike GO） |
| **D7** | 内核语言 | **单一 Rust 实现**：编译为浏览器 Wasm（`ygg-wasm`）、原生链接进 Godot（`gdext`），并在权威服务器上原生运行（`ygg-agent` / Axum）。保证无 GC 抖动的确定性回放。 | 已确认 |
| **D8** | Web 表现技术栈 | **React + TypeScript**（`packages/client-web`），用于以文本为中心的叙事 UI 与成熟的 AI 工具生态。 | 已确认 |
| **D9** | 平台服务（M2+） | **Rust + Axum**，与内核直接共享 `ygg-schema` 校验逻辑。 | 已计划 |
| **D10** | 内容流水线技术栈 | **TypeScript**（`packages/pipeline`），用于 I/O 密集的 LLM 编排、结构化输出解析与重试循环。 | 已确认 |
| **D11** | Mod 契约 | **Lua 脚本作为唯一可执行接口**：叙事流、效果与条件以 Lua 编写；静态内容存放于 JSON 资源文件，经 `ygg.data(...)` 加载。取代更早的声明式 JSON 条件/对话树草案。 | 已确认 |
| **D12** | Crate 边界 | **`crates/ygg-sandbox`** 封装 `mlua`、宿主 API 白名单与内嵌的 `ygg` Lua 标准库；`crates/ygg-lint` 提供独立的 AST 静态分析。 | 已确认 |
| **D13** | 沙箱安全边界 | **零 I/O、零网络、零时钟、指令燃料上限、内存上限（`16 MiB`）、API 白名单（`safe_stdlib`）与确定性种子 PRNG**。内容审查简化为 JSON Schema 校验 + `ygg-lint` + `playtest`。 | 已确认 |
| **D14** | 持久化状态权威 | **持久化状态仅存在于 Rust（`ygg.kv` / `ygg.entity`）**。Lua 局部/全局变量在单次 dispatch 内是临时的；`ygg-lint`（`YGG-R2`）标记未持久化的实体句柄。 | 已确认 |
| **D15** | 外部数据与 Schema 单一来源 | **资源 JSON 文件 + `packages/schema/schemas/` 中的单一来源 JSON Schema**，由 Rust（`jsonschema`）与 TypeScript（`ajv`）做完全一致的校验。 | 已确认 |
| **D16** | Lua 方言 | **标准 Lua 5.4**（经内置 `mlua`），带指令计数钩子燃料、加固的标准库（移除 `os`/`io`/`debug`/`package`/`dofile`/`loadfile`，`require` 替换为纯沙箱加载器），以及确定性的 `luai_makeseed`。 | 已确认 |
| **D17** | 并发与分片模型 | **单实例同步 FIFO 确定性内核 + 宿主侧分片**：每个 `KernelFacade` 严格单线程 FIFO；水平扩展使用相互隔离的 `KernelFacade` 分片，仅通过规范的 tick 边界消息通道通信。已在 1 万实体上验证亚毫秒 dispatch（`probe_determinism`）。 | 已确认 / 保留 |
| **D18** | LLM 提供方抽象 | **双协议适配器（`openai-chat` 默认 + `anthropic-messages` 回退）**，严格通过 git-ignored 的 `.env` 键（`YGG_LLM_*`）配置。 | 已确认 |
| **D19** | 试玩验证契约 | `playtest` 产出的**机器可读 8 键 JSON 报告**（贪心 option-1 遍历、自定义 `--choices`、`--max-steps`），通过 Mod 声明的 `agent.endingPrefix` 将结局归类为 `ending` / `choice` / `limit` / `deadend`。 | 已确认 |
| **D20** | 资源模板单一来源 | **`scene-texts.json` 与 `agent-config.json` 的 Schema 规范地存放于 `packages/schema/schemas/`**；LLM 填充数据实例，绝不自创 Schema 定义。 | 已确认 |
| **D21** | egg 全局化与全局唯一 serve | **链式软链**：`~/.local/bin/egg` → `<repo>/.ygg/bin/egg`（安装副本为唯一事实源，重建后重跑 `scripts/install-egg.sh` 即同步；cargo clean 无感、免 sudo、零墓碑）；**单实例按端口成立**：启动前只读探针 `GET /api/runs`（零锁文件）——自家实例幂等复用退出 0 / 根不被覆盖拒绝退出 1（不热改根）/ 旧代单根实例拒绝并提示重启 / 外姓占用由 bind 失败兑住；**根集三层显式解析链**：可重复 `--runs-root` > `~/.config/ygg/serve.json` > `./runs`，**拒绝文件系统爬扫**（根集随文件系统静默漂移违反 P10 显式声明边界；面板不得出现未声明过的根）。 | 已确认 |

### 5.2 Agent 治理（`M-Agent`）架构决策

| ID | 决策领域 | 裁决与理由 | 状态 |
| --- | --- | --- | --- |
| **D-MA1** | 工作流即 Mod 架构 | 通过 `.ygg/mods/agent-seed` 协调软件工程任务（常驻自循环：`boot(event.start 载荷章程) -> coordinate ⇄ review 批循环 -> (退役) done` + 每个需求的 `draft -> planning -> plan_gate -> ready -> developing -> accept -> done`，详见 D-MA10）。所有产物都要求先有状态机事件。 | 已确认 |
| **D-MA2** | 多文件 Mod 纯度（`YGG-MOD-EFFECT`） | `pack.json` 的 `code.{entry, modules[]}` 显式枚举所有 Lua 模块。只有 `mod.lua` 可以在顶层注册处理器或改变状态；`src/**` 模块导出纯函数（`M.install(deps)`）并携带真实的 `@src/<file>.lua:line` 诊断信息。 | 已确认 |
| **D-MA3** | 单文件 SQLite 运行存储与 VFS | 新运行默认使用 `<run-dir>/run.sqlite`（`ygg-store`），原子提交 `events`、`snapshots`、`inbox`、`kv_projection` 与 `vfs_files`（`artifacts/*`）。既有的 JSON 运行（`autosave.json` + `events.jsonl`）保持自动检测，不隐式迁移。 | 已确认 |
| **D-MA4** | 响应式钩子与省回合压缩 | `src/core/hooks.lua`（`useMemo`、`useEffect`、`flush_effects`）在 `render()` 发出 `ygg.choice` 之前运行。实现 3 条省回合规则：① `draft -> planning` 阶段合并，② 在 `proposal.submit` 上内联 `exec=<agent_id>` 注册，③ `gate.approve(plan)` 之后的响应式 `auto_dispatch`。 | 已确认 |
| **D-MA5** | 同步工具桥（`ygg.tool.call`） | 当 `ygg-agent` 挂载 `HostToolRunner` 时，白名单工具（`git.worktree_claim`、`git.lane_probe`、`git.worktree_release`、`fs.artifact_probe`、`gate.verify_suite`、`harness.archive_sessions`）在 1 回合内联执行并立即把结果折叠进 KV，同时为未挂载宿主保留异步 `tool.call`/`tool.result` 回退。 | 已确认 |
| **D-MA6** | 持久化泳道工作树池 | `worktree-claim` 管理持久化池 `<repo-parent>/worktrees/<repo>-lane-1..4`，通过 `git checkout -B <branch> HEAD` 重置分支（保留如 `target/` 的未跟踪构建缓存），并将释放的泳道停放在 `--detach HEAD` 上而非删除目录。 | 已确认 |
| **D-MA7** | 通用治理引擎适配器（`subagentAction`） | Mod 计算 `summary.jobs[i].subagentAction`（`subagent.spawn` / `subagent.resume`），携带渲染后的 brief（`data/texts/` 模板片段）与实时的 `worktreeStatus` / `subagentStatus` / `probeStatus`，使外部执行壳（如 Claude Code / Codex / Cursor 适配器）能充当薄执行壳。 | 已确认 |
| **D-MA8** | 活文档整合 | 将所有架构、领域与契约文档整合进 `docs/CONTEXT.md`（领域与决策）与 `docs/DESIGN.md`（系统行为与接口契约），通过子代理 brief 中的强制必读引用与最终审查时的同步检查来执行。 | 已确认 |
| **D-MA9** | 动作面三层分层（`event` / `op` / `action`） | **取向：部分收敛（Partial Convergence）。** `event` 是状态推进的唯一货币（全集计数不写死，口径与复算命令见 `docs/DESIGN.md` §3.4 末注；`guide.show` 已随 r1 物理注销），`op` 是状态之外的宿主端口（8 个），`action` 是适配器的语义入口（9 个，值域随 harness 可变）。**禁止把 `action` 理解为 `event` 的第二份清单**：9 个 `action` 里只有 `approve_gate` / `reject_gate` 与 `event` 重叠，且是**有意保留的位次正确性别名**（`reqId` 必填可防「批错需求」，判据为「能防止一类可命名的 payload 错位才配拥有别名」）；`vfs_write` 与 `event=vfs.write` **语义不等价**（事件路径有阶段守卫、64 KiB 上限、`artifact.*` 台账与 `[VFS LEDGER ONLY]` 降级，`op=vfs_put` 全都没有），故不得互替；`kickoff` / `settle_close` 是多步编排（顺序契约在适配器侧），`dispatch` 是唯一的事件直通口。零增量同义别名（原 `step`）**已从值域退场**，不做兼容残留。`dispatch` **不得附带压缩等回合后动作**：压缩只经 `action=handoff_compact` 与 `ygg_run_lanes` 编排层刹车两个显式入口。契约正文见 `docs/DESIGN.md` §3.4。 | 已确认 |
| **D-MA10** | map/guide 语义收口（r1 rev3） | `map.show` 单一视图：操作视图即完整视图，payload 一律忽略（payload 无关性由语义测试钉死：任意两个 payload 的叙事逐行相同），设计视图整体退出 CLI。`guide.show` **物理注销**：注册面、菜单挂载、文案键、playbook 引用、孤儿代码零残留，不做墓碑兼容；直派旧名不再有静默假成功（内核对未注册事件 fail-closed，r11 起为可读 VALIDATION 拒绝）。删除即报错、零残留退场（P13）。 | 已确认 |
| **D-MA11** | 迭代分支命名强制形态（r2） | 泳道分支必须满足 `feature/{4位uuid}-{语义词段}`：小写 4 位 uuid 段 + 语义词段（≥3 连续字母、非纯数字、8–16 位纯 hex 拒绝、总长 ≤48、前缀大小写敏感）。`lane.claim` 与 `git.worktree_claim` 两条入口同规则、拒绝通道不同（前者 `flow.refuse` 可读拒绝且 `reject.*` 留痕，后者叙事拒绝、账本与 reject 台账不动）；默认分支名生成机制整体删除，未自报名 = 可读拒绝——标识符 = 机器唯一 + 人类意图（P14）。 | 已确认 |
| **D-MA12** | 提交者身份模型（r3：宿主章互证 + profile 注册 + opener） | **双通道一致性可核对，不是身份认证**。① 解析链 `--actor` > `YGG_ACTOR` > daemon 请求字段（`actor_session` 对称），宿主为声明路由注入 `\|actor=` / `\|actor_session=` wire 段，调用方自带保留段 = `ACTOR_FORGED` 拒绝（格式红线，不是密码学）；② mod 三态互证（`exec=` vs `actor=`：verified / mismatch / unverified，识别不拒绝，裁决权在人 / 门禁）；③ 双 id 命名空间不可混用（Agent 工具句柄 vs 原生 session id——后者只作审计，永不参与判定）；④ profile 注册 = 纯 `config.set` KV 约定不新增事件（不推进状态的事件违背 L1；`config.set` 天然落事件行可审计、幂等、不占门禁），自报卡与账本互链不冗余（权责看账本）；⑤ opener = run.json 首条带身份成功写先到先得不可变（失败步不认领）。诚实威胁模型：同机进程下 env/flag/profile 可伪造，本机制防误记与张冠李戴、让冒用可见可审计，不做密码学不引入 token。契约正文见 `docs/DESIGN.md` §3.7。 | 已确认 |
| **D-MA13** | 种子 Mod 更名 `agent-seed`（r4） | 物理目录 `.ygg/mods/pi-agent-seed` → `.ygg/mods/agent-seed`，`packages/mods/agent-seed` 相对软链重建（单一事实源不变，见 P5）；宿主内置 legacy 别名（存在性优先）翻译旧路径，服务存量局续局——更名不承诺旧路径永续，只保证干净的翻译层与可读报错。**别名翻译层已退役（2026-11-12，裁决与细则见 D-MA19）**。 | 已确认 |
| **D-MA14** | CLI 回执文本渲染（r5 v3） | 单发回执**缺省人读文本**（自然语言版式），`--raw` 为唯一机器旗标（输出单行 JSON 信封，与历史缺省逐字节一致）；解析链 `--raw` > `YGG_FORMAT`（text\|json）> 缺省 text，非法 env 值 fail-closed；字段契约、退出码与 daemon 协议面零改动。人读缺省是原则，机器面由显式旗标保护（PLAY4）。 | 已确认 |
| **D-MA15** | Mod 片段化与 src 五层（r6） | 大 JSON 按域拆片：`data/texts/`（scene-texts-fragment 若干片，`mod.lua` 装配回 TEXTS）与 `data/playbook/`（agent-playbook-fragment 若干片），`pack.json` `entries` 数组逐片声明；`src/**` 重组为 core/domain/view/runtime/handlers 五层（24 模块纯移动）。宿主不感知 Mod 内部布局——拆分是 Mod 自由度，不是内核契约（P11）。 | 已确认 |
| **D-MA16** | 自动收波看门狗（r1） | 触发 = 扩展内进程内定时器轮询 daemon `summary`（`YGG_AUTO_INTERVAL_MS` 钳位 1s–60s，缺省 5s；批量入口禁令下唯一合法形态）；落点 = 每波原样复用 `ygg_run_lanes` 全链，天然停在门禁/待决提案（可收波谓词：available ∧ `lifecycle=="resident"` ∧ 无冲突 ∧ ∃ plan/development 作业未 `delivered:*` 未停车）；回执 = 事件账与手发同构 + `artifacts/auto-wave-<NNNN>.md` 波次日志（narrative 全量不截断）+ 交接单重写；保活 = 每 tick 一次 `summary()` 心跳，意外退出由 daemon 客户端既有单次重启自愈（spike 实证：`ctx.executeTool` 在回合外 handler ctx 上不可用 → 降级为 `pi.sendMessage(triggerTurn:true)` 确定性自唤醒回合，主代理代跑 `ygg_run_lanes`，每波多耗一个模型回合，语义与回执不变）；预算 = per `(req,rev,stage)` 连续 2 次整波失败停车（rev 变化自动复位）+ 全局 3 波全败自关（fail-closed）；在场唤醒 = goal 模式限定 opt-in（`YGG_AUTO_WAKE=on`），guide 模式永不唤醒；压缩边界 = 看门狗不是第三个压缩入口（硬水位只抑制派波）。宿主零改动；mod 仅 `summary_fields` 加性 `play` 投影。契约正文见 `docs/DESIGN.md` §3.5。 | **已废弃**（扩展层删除 2026-11-12，看门狗随层移除；daemon 常驻由宿主原生承担） |
| **D-MA17** | 观察台展示投影（面板默认局详情视图） | 内置面板页允许对 agent-seed 工作流做**只读展示投影**：Rust 服务面保持零语义（零新端点、零解析——P11 字面边界不动），领域词汇（`fsm.stage`/`req.*`/`prop.*`/`lane.*` 键族、`req_phase_name` 派生名、决策事件族）的解析只发生在 `assets/panel` 展示层，且对无 `req.*` 键族的 mod **无害退化**（详情退化为通用摘要、零报错）。理由：面板=纯前端资产是「同 r3」需求约束，mod 侧自建 panel 违反该约束且引入第二套 UI（违 P4）；投影不限制 mod 自由度——任何 mod 的局仍可被观察，r8 的「宿主解析限制 mod 自由度」担忧只落在 Rust 面。判据=投影失败必须无害退化（字段缺失显「—」、非 seed 局只显摘要段）；权威仍是 mod 回执/CLI，面板仅为投影。 | 已确认 |
| **D-MA18** | intake 阶段注销与章程载荷化（r2 rev2 B 裁决） | 注销 `intake` 宏观阶段与 `task.intake` 事件（零残留：菜单/渲染/地图/文案/playbook 链句全清，事件全集 33→32 见 DESIGN §3.3 末注）；冷启动章程改由 `event.start` 启动载荷承载（`title\|description`，复用 `parse_intake` 校验；`""`/`"0"` 同判无载荷；coordinate 内再带载荷 = 幂等补写不过段）； 局保护：`gate_problem` 增两条款（章程非空 ∧ 需求数 ≥1），`coordinate.submit`/`verify.submit`/`review.note` 共用可读拒绝，同时堵 `req_all_done` 0 需求空真洞。理由：冷启动两步仪式是 E3 筛查确认的隐藏出边——intake 阶段可立案后直接派活、 局可空收口，治理面外泄；载荷化后 boot 一步落 coordinate，章程与收口前置由状态机强制。判据=全 Mod grep intake 除 `[deprecated-ref]`/`parse_intake` 标识符/pack 钉死路径外归零；变异（重注册 task.intake）下 VALIDATION 拒绝集与 33 事件计数断言必红。 | 已确认 |
| **D-MA19** | 旧名别名机制退役（r3 rev1，用户裁决放行） | `pi-agent-seed` → `agent-seed` 的双向别名翻译层（D-MA13 的 `legacy_seed_alias`/`with_legacy_seed_alias`，含 main 入口归一与 serve 调用点）整体删除，解析单义化：`resolve_mod_dir` 仅对「路径缺席 ∧ 父目录名 `mods` ∧ 末段精确 `pi-agent-seed`」给专名拒绝（点名同父规范路径、宣布退役日期、历史 run.json **不迁移**——开新 run-id 新局）；其余缺席维持通用缺路径错误；serve 面板对同形状缺席给专名降级理由（观察面不炸，§4.1）。断点：D-MA13 的翻译承诺就此终止，存量局 fail-closed + 可读指引；机检护栏 = AGENTS.md §1.7「Mod text hygiene」+ `agent_seed_semantics::mod_text_fragments_carry_no_retired_paths`。 | 已确认 |
| **D-MA20** | r1 归档防护契约补记（r5 补录） | r1（主会话保护）交付时漏记台账，本批补记：宿主 `archive-sessions` 按「文件名排除 + 全文 contains(parent)」选文件，parent 回落 `"default"` 时可把主会话 rename 进 trash（合成探针三场景实证）；Mod 侧 fail-closed——缺 `run.id`/`run.parent` 凭证不发归档请求、不伪造 `remaining=0`；review 相位撤销静默自动归档，改 `[SESSIONS UNSETTLED]` 可读提示；宿主语义测试块（原 `synchronous_tool_bridge` 第 4 步）重钉为新安全契约（无 sessions 段不归档）。用户授权的有界宿主测试修改自此成为先例。 | 已确认 |
| **D-MA21** | 提示词驱动子会话清理（r5） | 用户核心需求修正：「删除 subagent 会话记录」必须可用且**跨 harness**——动作走提示词而非宿主代码。清理提示词 beat（`[SESSIONS CLEANUP PENDING]`）指引用主会话自身环境原生能力删除本局 subagent 会话记录（仅限本局产物，主会话与无关会话绝不碰），完成后声明 `settlement.sessions`（`moved/remaining/destination`，`moved=0` 合法；缺声明 = 诚实未清理，`review.note` 保持阻断）。`harness.archive_sessions` 从 Mod 工具桥整体退役（pack.json + allowedTools 同批摘除保 superset 一致、REACTORS 条目与死代码 `auto_archive_sessions` 删除、beats 去专名）；机检护栏 = `agent_seed_semantics::beats_cleanup_prompt_carries_no_harness_literals`；宿主 builtin 的物理退役（含子串匹配危险与 dry-run 缺失）归 N2 宿主决策。 | 已确认 |
| **D-MA22** | 集成职责再分配：执行者集成 feature→develop，主流程只做 develop→master | 原 `integration.serial-closeout` 把全部集成串行分配给主代理，且状态机无集成事件——集成退化为批次末 集中合并，需求量大时冲突集中爆发（2026-11-12 局实证：4 个 feature 批末一起合 develop）。现按用户设计意图翻转：迭代点集成（feature→develop：对齐 origin/develop → `--no-ff` 合并 → 集成态门禁 → push develop）由需求开发执行者在自己 lane 收口时完成，验收 artifacts 必带 `develop@<sha>` 集成证据；同仓库集成经 `origin/develop` 自串行（push 被拒=别人先落，重对齐重试；栈叠 lane 按基线序）；轮次收口（develop→master、用户触发）与部署仍为主代理串行职责。落地：ch2-delivery §integration 五条改写、ch3-evidence 翻转、subagent_brief 模板补集成职责行。 | 已确认 （2026-11-12 用户修订：`develop` 与 `master` 的同步一律走 `git reset --hard master` + `--force-with-lease` push，反向合并 master→develop 禁止——见 playbook `integration.round.reset-develop`。）|

