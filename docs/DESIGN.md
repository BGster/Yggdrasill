# Yggdrasill · 系统架构、行为语义与接口契约（`docs/DESIGN.md`）

> **定位**：`docs/DESIGN.md` 规定 Yggdrasill 的**产品形态、系统行为、状态机语义与规范接口契约**。领域词汇、愿景、架构原则与决策台账（`D1–D20` 与 `D-MA` 系列台账，上界随台账顺延不写死）见 [`docs/CONTEXT.md`](CONTEXT.md)。为使本文档在实现重构下保持持久，其契约定义在边界层（**JSON Schema、CLI/JSON-RPC 线协议、Lua 沙箱宿主 API、事件 Payload、状态迁移表**），而非内部源码函数名或行号。
>
> **术语速览**：领域术语的完整定义见 [CONTEXT §2 核心领域词汇](CONTEXT.md#2-核心领域词汇)，本文档不再重复维护。
>
> **动作面分层**：Layer 2 的工具参数面契约（`ygg_session.action` 与 `event` / `op` 的分层、三类落点与硬不变量）见 [§3.4](#34-三层动作面契约event--op--action)。

---

## 1. 产品形态与三层系统边界

Yggdrasill 将确定性规则执行、声明式工作流语义与宿主环境集成分离为三个严格有界的层：

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ Layer 2 · Presentation & Harness Adapter Shell                             │
│   - Game Clients: Web Browser (React + ygg-wasm), Godot 4 (gdext)          │
│   - Agent Adapters: Pi Extension (.pi/extensions/ygg-pi-harness),          │
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
| **Tier 1 · 原生扩展** | Pi Coding Agent（`ygg-pi-harness`） | 原生 `Agent({ resume })`，自动回退到 `fullFallbackBrief` | 实时 token 比例（`soft >= 60%`、`hard >= 75%`），自动 `artifacts/handoff-latest.md` + `compact()` | 持久化 `ygg-agent --daemon` stdio JSON-RPC 客户端 |
| **Tier 2 · CLI / Hook 运行器** | Claude Code、Codex CLI、Cursor | 子进程调用或原生任务工具，消费 `subagentAction.brief` 与 `submitCommand` | 在稳定检查点由 Hook 触发的 `handoff_compact` | `ygg-agent --daemon` 或单次 `--resume --event` CLI |
| **Tier 3 · 提示委派** | 通用 LLM 聊天 / 自定义 Agent 循环 | 协调者把 `subagentAction.brief` 复制进子代理上下文并转达 `proposal.submit` | 经 `vfs.write` 手动检查点交接 | 单次 CLI 回执（缺省人读文本；`--raw` 显式旗标输出单行 JSON 信封）（`narrative` + `pendingChoice`） |

### 1.2 Adapter 一致性契约（任何 harness 都必须满足的五条义务）

三层架构的薄弱点在第 2 层：`subagentAction` 只是 `meta.summary` 里的**数据**，dispatcher 完全由 adapter 自行实现。于是「Mod 产出的 brief 被消费」曾只写成注释约定，离开 Pi 的 `Agent` 工具后就静默丢失。本节把这件事变成**可断言的义务**，全部落在 `.pi/extensions/ygg-pi-harness/`（与 Pi API 无关，可移植到任何 harness），每条都有纯函数、失败信号与可跑的命令。

| # | 义务 | 纯函数 / 位置 | 违反时的可见症状 |
| --- | --- | --- | --- |
| **C1** | 派发给 subagent 的 prompt 必须含 `Mandatory references:` 与 `Submit:` 两条**行首锚线** | `checkSubagentActionConsumption`（`orchestrator.ts`） | 回执 `[ADAPTER CONTRACT] <rN> <stage>: …`，**不 spawn**，结果进 `degraded` |
| **C2** | `deltaPrompt` 若被单独使用，必须可判定；不可判定时**不得消费**（fail-closed） | 同 C1，策略表 `DELTA_ASSESSABLE` | `delta-unassessable`；adapter 退回 `brief` 并写 `[ADAPTER NOTE]` |
| **C3** | 任何一次派发都必须把**实际使用的 prompt 逐字节**落进 `artifacts/<rN>-<stage>-brief.md` | `briefArtifactPathFor` + `writeBriefArtifact` | `[BRIEF REFUSED] <path>：<原因>` 进回执，并计入 `degraded` |
| **C4** | 归档结论必须是「查过且查空」或「没有库可查」，**不得**以「工具跑过」充当 `0/0` | `analyzeArchiveVerdict`（`archive.ts`） | 见下文 §1.2.1 三态表；非证据态写 `sessions=0/0/unsettled:<kind>` |
| **C5** | 每一份 brief 都以 `## 续跑承接（Resume Handover）` 段收尾 | `ensureResumeHandoverSection` | `brief-missing-resume-handover`；无 `agentId` 时 `resume` 无法等价 |

**锚线为什么必须是「行首前缀」而不是 `includes`**：`job.description` 是多行需求正文，正文里完全可能出现 `Mandatory references: …` 这几个字。用任意 `includes` 判定会被一句话绕过——那是自指类断言的经典空转（`harness.test.ts` 有一个用例专门喂这种输入）。`Submit:` 额外要求**后续非空**（`^Submit:\s*\S`），这样「有这条命令」与「命令不是空的」被同时钉住。

**两条路必须同形状**：Core 的 `subagent_brief` 模板（`data/texts/` 片段装配的 TEXTS）带这两行；adapter **本地拼装**的 brief 也必须带。否则「旧 Mod 兼容路径」会自己违反 C1——`harness.test.ts` 的 `本地拼装路径也满足两条锚线` 用例咬住这一点。

**`preferCoreBrief` 不是绕过开关**：`preferCoreBrief: false` 是**显式的旧 Mod 兼容降级**，不是静默默认——此时回执写 `[ADAPTER NOTE]` 且 `OrchestrationResult.degraded` 非空，让上层看见；而 Core 明明给了 `subagentAction` 却被关掉**不构成** `no-core-action` 的豁免理由（那正是本契约要消灭的路径）。

**`resume` 的等价物是制品，不是这一段**：`## 续跑承接（Resume Handover）` 记录 `basis` / 前任执行者 / 未落地清单 / 硬约束 / 已归档制品，作用是让**下一个执行者**（可能是另一个 harness、另一个上下文窗口）知道上一任为什么停在门外。它本身**不是** `resume` 的等价物——等价物是 `artifacts/<rN>-<stage>.md` 加上门禁证据。`artifacts/handoff-latest.md` 的下一步会显式点名这条 `continuation` 路径。

#### 1.2.1 未知 harness 下 `archive-sessions` 的行为边界

会话库落点被硬编码在宿主（`agent.rs` 的 `harness_session_dirs` 只认 `pi` / `claude-code` / `codex`），`auto` 探测在含 `.pi/` 的仓库里必然判成 `pi`。因此**不知道落点时唯一诚实的做法是改判结论，而不是猜落点**。归档结论只允许以下四种，且必须可区分：

| 结论 | 宿主输出 | 是否算证据 | Mod 侧 `settlement.sessions` |
| --- | --- | --- | --- |
| ① `swept` | `moved=N;remaining=0;dest=<本局声明的库根下>` | ✅ 是 | adapter 不写 `sessions=` 段，由 Mod 的 `[SESSIONS SETTLED]` 机证接管 |
| ② `no-store` | `dest=generic:no-session-store:<batch>` | ❌ **不是** | `sessions=0/0/unsettled:no-store`（可收口，但台账自证「没有库」） |
| ③ `unavailable` | 非零退出 / `builtin_usage_error` / **外域 `dest`** | ❌ 不是 | `sessions=0/0/unsettled:unavailable` |
| ④ `unsettled` | 工具没回可判定结果（空叙事 / `remaining>0` / 三元组不全） | ❌ 不是 | `sessions=0/0/unsettled:unsettled` |

**禁止第五种**：`dest` 非空但**没有任何 harness 认领**（典型形态：在 DeepSeek Harness 局里出现 `dest=~/.pi/agent/sessions-trash/<batch>`）。那条路径确实存在，但它归档的不是本局会话——把它写成「查空了」就是谎报。识别完全在扩展侧，不需要新契约：`dest` 的 trash 根必须落在**本局声明的 adapter** 对应的根下（`archive.ts` 的 `FOREIGN_SESSION_ROOTS` + 可选的 `destRoot`）。默认判据是「宁可说没查，不谎报查空了」。

> **给宿主维护者的一句话**：要让 `archive-sessions` 真正覆盖未知 harness，需要在 `agent.rs` 增加第五个适配器及其会话库约定。本仓库把 `crates/ygg-sandbox/examples/agent/main.rs` 当红线，所以这里只保证**不谎报**，不自行修改宿主。

---

## 2. 工作流状态机与行为语义

### 2.1 宏观 Coordinator 阶段机

Coordinator 在 4 个阶段上支配整体运行生命周期。r9 起生命周期是**常驻自循环**：`coordinate` 是常驻主态，
`review` 是批盘点相位，`review.note` 的常态落点是**批收口**（回 `coordinate`，批计数 +1）；运行级终局
`done` 只经**显式退役**（`config.set run.retire=true` 武装后的终局变体）可达。需求级 `done` 保留
（accept_gate 产物，与运行退役态只是字面量同形，语义正交）：

| 当前阶段 | 事件 | 守卫 / 前置条件 | 下一阶段 | 行为效果 |
| --- | --- | --- | --- | --- |
| `intake` | `task.intake` | 非空 `title\|description` payload（仅冷启动；新任务经 `requirement.add` / inbox 进料） | `coordinate` | 记录任务元数据并开启需求协调。 |
| `coordinate` | `coordinate.submit`（**批边界**） | `pendingProposals == 0` $\land$ `jobs == 0` $\land$ `conflicts == 0` $\land$ 所有叶子需求 `done` | `review` | 重置 `verify.passed = false` 并清空本轮的 `settlement.*` 键。 |
| `review` | `verify.submit` / `gate.verify_suite` | `all_green=true`（或所有验证命令退出码为 `0`） | `review` | 锁存 `verify.passed = true`（`verify.source = "human"` 或 `"tool"`）。 |
| `review` | `verify.submit` | 当 `gates.require_all_green == true` 时 `all_green=false` | `coordinate` | 将本批退回 `coordinate` 以返工，并重置结算注册。 |
| `review` | `settlement.evolution` | `verify.passed == true` $\land$ 有效的候选/类别计数 | `review` | 注册演进摘要/产物，结算 `sessions=` 与 `todo=`，并触发内联 `harness.archive_sessions`。 |
| `review` | `review.note`（未武装，常态） | `verify.passed == true` $\land$ `settlement.*` 完整 $\land$（`play.mode != "goal"` $\lor$ `goal.authorization != ""`） | `coordinate` | **批收口**：将演进记录归档进 `review.evolution.*`（滚动单槽），`run.batch` +1，复位 `verify.passed` 与 `settlement.*`，发出 `[BATCH <n> SETTLED]`（绝无 `[ENDING` 前缀）。 |
| `review` | `review.note`（`run.retire` 武装，唯一终局） | 同上（goal 授权在每批盘点与退役前都查） | `done` | **退役**：归档 + 写入 `ending = "done"`，发出 `[ENDING · run retired]`。 |

### 2.2 需求生命周期与强制门禁语义

门禁裁决权随 `play.mode` 划分（硬约束，与 AGENTS.md §1.8 同源）：`guide` 模式下门禁动作只收菜单票，由**人**裁决；`goal` 模式下主代理读完整回执后自行裁决（`goal.authorization` 逐局登记，证据标准不变）。机制上表现为：`proposal.accept` / `gate.approve` 等里程碑动作在 `guide` 下拒绝非票据直发（见 §3 动作表）。

每个需求 `rN` 用 5 个规范状态（`draft`、`planning`、`developing`、`done`，外加终态旁路状态 `split`、`merged`、`dropped`）与正交的状态属性（`plan_ok`、`job`、`gate`、`lane`、`rev`、`conflict`、`needs_amend`）存储：

| 存储状态与属性 | 派生展示阶段 | 触发事件 | 目标状态与属性 | 语义契约 |
| --- | --- | --- | --- | --- |
| `draft` | `draft` | `requirement.dispatch` | `planning`（`job="plan"`） | **阶段合并**：直接派发 `plan` 任务，无需单独的 `analysis` 回合。 |
| `planning`（`job="plan"`） | `planning` | `proposal.submit`（`stage="plan"`） | `planning`（`job="plan"`，待决提案 `pK`） | 子代理交付方案产物（`artifacts/rN-plan.md`）并经尾随 `exec=<agent_id>` 绑定 `executor.rN.plan`。 |
| `planning`（待决 `pK`） | `plan_review` | `proposal.accept` | `planning`（`job=""`，`gate="plan"`，`gate_rev=rev`） | 释放任务槽并升起强制 `plan_gate`。 |
| `planning`（`gate="plan"`） | `plan_gate` | `gate.approve` | `planning`（`gate=""`，`plan_ok=true`）$\to$ **响应式自动派发** $\to$ `developing`（`job="development"`，`lane=k`） | 批准 `plan_gate` 置 `plan_ok=true`；当该需求自身无在跑任务、无自身待决提案、无自身冲突（门禁按需求局部化判据，r12：他需的事态不再阻塞本需派发）且全局开发槽空闲时，响应式钩子立即认领 `lane-k` 并在**同一回合**派发 `development`。 |
| `planning`（`gate="plan"`） | `plan_gate` | `gate.reject` | `planning`（`gate=""`，`plan_ok=false`，`rev=rev+1`） | 使方案失效，`rev` 递增，并返回规划队列。 |
| `developing`（`job="development"`，`lane=k`） | `developing` | `proposal.submit`（`stage="development"`） | `developing`（待决提案 `pM`，内联 `git.lane_probe`） | 绑定 `executor.rN.development`，同步探测 `lane-k`（`commits`、`clean`、`head`、`files`），并运行跨泳道冲突雷达。 |
| `developing`（待决 `pM`） | `dev_review` | `proposal.accept` | `developing`（`job=""`，`gate="accept"`，`gate_rev=rev`） | 释放活动任务槽并升起强制 `accept_gate`（泳道在 `accept_gate` 通过前保持绑定）。 |
| `developing`（`gate="accept"`） | `accept` | `gate.approve` | `done`（`gate=""`，`lane=0`，内联 `git.worktree_release`） | 要求非空 `artifacts` 且 `gate_rev == rev`。在**同一回合**将需求标记为 `done` 并把 `lane-k` 停放在 detached `HEAD`。 |
| `developing`（`gate="accept"`） | `accept` | `gate.reject` | `planning`（`gate=""`，`rev=rev+1`，保留先前的 `plan_ok`） | 若未修订（`plan_ok=true`），展示为 `ready` 重新进入开发；若已修订（`plan_ok=false`），展示为 `planning` 重新规划。 |

#### 旁路与异常迁移
- **`requirement.amend`**：递增 `rev` 并使 `plan_ok` 失效（除非已处于 `developing`，此时保留进度）。若有门禁挂起，`gate_rev != rev` 使该门禁呈现为 `stale` 并阻塞 `gate.approve`。修订一个 `done` 需求会派生一个带 `origin=rN` 的新 `draft` 需求。
- **`requirement.split` / `requirement.merge`**：仅当需求处于 `draft` / `analyzing` / `planning` 且无活动任务、门禁、泳道或冲突时有效。父/源需求进入终态 `split` / `merged`，而子/目标需求从 `draft` 起步。
- **`requirement.conflict` 与冲突雷达**：在两个需求之间升起双向冲突（`req.i.conflict`、`req.i.conflict_peer`），暂停派发并关闭，直到经 `requirement.resolve`（`keep_both`、`keep_first` 或 `revise`）解决。

### 2.3 响应式单后继推进与效应收敛

在发出 `pendingChoice` 之前，Mod 按确定性的注册顺序冲刷已注册的响应式效应（`useEffect`）（最多 `MAX_PASSES = 4`；不收敛即为硬错误）。每个响应式效应必须遵守四条不变量：

1. **唯一声明后继**：迁移是确定性的，且只在一处声明。
2. **零外部判断**：无需人类批准或未挂载的外部 I/O。
3. **绝不绕过门禁**：里程碑放行动作（`proposal.accept`、`gate.approve`、`coordinate.submit`、`verify.submit`、`review.note`）绝不由效应触发。
4. **无渲染的效应**：效应可改变 KV 并调用 `ygg.narrate`，但绝不调用 `render()` 或 `ygg.choice`。

### 2.4 持久化泳道池与实时进度投影

- **非破坏性认领（`worktree-claim`）**：分配 `<repo-parent>/worktrees/<repo>-lane-<k>`（$k \in 1..4$）。若 worktree 已存在且其已跟踪工作树是干净的，则运行 `git checkout -B <branch> HEAD`，使未跟踪构建缓存（`target/`、`node_modules/`）跨任务存活。
- **非破坏性释放（`worktree-release`）**：验证 worktree 干净并运行 `git checkout --detach HEAD`，为 Coordinator 合并解锁 `feature/<req>` 而不删除目录。
- **强制分支命名（`branch_problem`，r2）**：泳道分支必须满足 `feature/{4位uuid}-{语义词段}`——小写 4 位 uuid 段 + 语义词段（≥3 连续字母、非纯数字、8–16 位纯 hex 拒绝、总长 ≤48、前缀大小写敏感）。`lane.claim` 与 `git.worktree_claim` 两条入口同规则、拒绝通道不同：前者走 `flow.refuse` 可读拒绝（`reject.count/last` 留痕），后者走叙事拒绝（账本与 reject 台账不动）；自动认领不再生成默认名（未自报名 = 可读拒绝，展示层未断言分支显示 `-`）。
- **执行者身份投影（r3）**：`requirement.list` trace 并列三态互证后缀（`[verified]` / `[mismatch:actor=…]` / `[unverified]`，判定只看 `exec=` vs 宿主 `actor=` 章）；`agent.summary.profiles` 数组（≤8 张，handle 升序）投影 profile 注册表（自报身份卡，与账本互链不冗余）。
- **实时状态投影**：`requirement.list` 与 `summary.jobs[]` 中的每一行需求投影出三条从 KV 派生的实时状态字符串：
  - `worktreeStatus`：`"n/a"` | `"pending"` | `"ready:<path>@<branch>"` | `"bound:lane-<k>@<branch>"`
  - `subagentStatus`：`"idle"` | `"pending"` | `"dispatched:<agent_id>"` | `"delivered:p<N>(<agent_id>)"`
  - `probeStatus`：`"n/a"` | `"pending"` | `"verified:commits=<N>,clean=<0|1>"`

---

## 3. 接口契约（规范线协议与 API 面）

### 3.1 Lua 沙箱宿主 API 契约（`ygg.*`）

Lua 5.4 沙箱只暴露安全标准库（`base`、`table`、`string`、`math`、`utf8`、`coroutine`）外加冻结的 `ygg` 全局表：

| API 面 | 签名与返回契约 | 副作用类别（`YGG-MOD-EFFECT`） |
| --- | --- | --- |
| `ygg.on(event_name, handler_fn)` | 注册事件处理器 `function(payload)`。`mod.lua` 加载完成后即封存；在运行时或子模块顶层调用即为错误。 | **模块顶层禁用**（仅入口 `mod.lua` 或 `M.install`） |
| `ygg.emit(event_name, payload)` | 入队一个内部派生事件（FIFO，受 `max_cascade_depth` 上限约束）。若在同一处理器中 `ygg.choice` 之后调用则被丢弃（`YGG-R1`）。 | **模块顶层禁用** |
| `ygg.narrate(text)` | 向当前迁移的临时叙述回执缓冲区追加一行。 | **模块顶层禁用** |
| `ygg.choice(prompt, options_array)` | 将 `pending_choice` 写入状态并**立即**通过哨兵错误终止当前处理器。玩家选择以新外部事件 `ygg.choice_selected`（1 基整数索引）到达。 | **模块顶层禁用** |
| `ygg.meta(key, json_value)` | 将结构化元数据（`summary`、`tool_calls`、`harness_actions`）写入当前 dispatch 回执的临时 `meta` 映射。 | **模块顶层禁用** |
| `ygg.data(resource_id)` | 纯读取，返回 `pack.json` `entries` 中声明的、经 Schema 校验的 JSON 资源的深度 Lua 表副本。 | 模块顶层允许（纯读取） |
| `ygg.kv.get(key)` / `ygg.kv.set(key, val)` | 读取或写入持久化键值状态（`nil`、`boolean`、`number`、`string` 或可 JSON 序列化的表）。处理器失败时回滚。 | `get` 允许；`set` **模块顶层禁用** |
| `ygg.entity.*` | `spawn()`、`exists(id)`、`destroy(id)`、`get_attribute(id, k)`、`set_attribute(id, k, v)`、`add_tag(id, t)`、`remove_tag(id, t)`、`has_tag(id, t)`。返回数字实体 ID（`YGG-R2` 要求将 ID 持久化到 `ygg.kv`）。 | 查询允许；变更 **模块顶层禁用** |
| `ygg.fsm.*` / `ygg.narrative.*` | 内嵌 Lua 标准库辅助，用于扁平有限状态机（`fsm.define`、`fsm.current`、`fsm.on_event`）与故事节点（`narrative.node`、`narrative["goto"]`；点形式 `narrative.goto` 是 Lua 5.4 语法错误，由 `YGG-R3` 捕获）。 | `define` / `node` / `goto` **模块顶层禁用** |
| `ygg.fs.{write,read,stat,list}` | 可选宿主注入的 VFS 接口，严格限定于 `artifacts/*.md`。`write(path, content)` 返回 `{ bytes, sha256 }`。 | 变更调用 **模块顶层禁用** |
| `ygg.tool.call(tool_id, args_table)` | 可选同步宿主工具桥。挂载时内联执行白名单工具并返回 `{ ok = true, data = { ... } }` 或 `{ ok = false, error = "..." }`。未挂载时返回 `{ ok = false, unmounted = true }`，使 Mod 回退到异步 `meta.tool_calls`。 | **模块顶层禁用** |

### 3.2 Mod 包与多文件纯度契约（`pack.json`、`manifest.json`、`data/*`）

一个 Mod 包目录包含：

1. **`pack.json`（编写与宿主契约声明）**：
   - `id`、`version`、`engineVersion`（semver 范围）、`seed`、`limits`（`maxFuelPerDispatch`、`maxCascadeDepth`、`maxMemoryBytes`）。
   - `code`（可选）：`{ "entry": "mod.lua", "modules": ["src/core/util.lua", ...] }`。当存在时，`entry` 不得出现在 `modules` 中，且每个模块 chunk 顶层由 `ygg_modules::effects` 强制为纯。
   - `entries`：声明式资源清单（数组，每项 `{ id, kind, schema, path }`），逻辑资源按域拆片声明：一个 `agent-config`（`data/config.json`）+ 若干 `scene-texts-fragment`（`data/texts/*.json`，由 `mod.lua` 装配回 TEXTS）+ 若干 `agent-playbook-fragment`（`data/playbook/*.json`）；片段数随内容演进，以 `pack.json` 为准。
   - `agent`：8 个必填字段（`entryEvent`、`choiceEvent`、`summaryEvent`、`endingPrefix`、`metaKey`、`rejectionPrefixes`、`replayPrefix`、`defaultPayload`）。
   - `inboxRoutes`：声明 `idField`、`externalIdField`、`kindField`，以及每 kind 的 `{ event, fields[] }` 映射，用于异步 JSONL 收件箱摄入；需要宿主注入提交者身份的动作可声明 `actor` / `actor_session` 布尔（r3，缺省 false = 功能关闭）。
   - `tools`（可选，**Layer 1 能力白名单**）：将 `<namespace>.<name>` 映射到 `{ kind: "cli" | "http", program, args[], timeoutMs, maxOutputBytes, readOnly, resultEvent }`。占位符用 `{{arg}}` 表示调用参数，用 `$ENV{VAR}` 表示宿主侧密钥解析。
2. **`data/config.json`（运行时默认值与 Layer 2 发布白名单）**：
   - 依据 `packages/schema/schemas/agent-config.json` 校验。
   - 声明 `play.mode`（`"guide"` | `"goal"`）、`evolution`（`mode`、`allow_paths`、`max_per_run`）、`gates.require_all_green`、`drive`（`maxSteps`、`stagnationLimit`、`decideTimeoutMs`）、`settlement`（`requireNonEmptyEvents`、`requireZeroFailedSteps`、`requireEnding`、`requireInboxConsumed`）、`allowedTools`（**Layer 2 发布白名单**，必须是 `pack.json` `tools` 键的超集），以及可选的 `harness` 适配器元数据。
3. **`manifest.json`（内容寻址完整性锁）**：
   - 依据 `packages/schema/schemas/manifest.json` 校验。为每个代码文件与数据资源记录 `{ path, sha256, byteLength }`。经 `./scripts/sync-seed-mod.sh` 维护。

### 3.3 事件 Payload 线协议契约

派发到 `agent-seed` 的所有事件使用管道分隔（`|`）的 UTF-8 字符串 payload（在支持处附带可选尾随 `@ext=<id>` 幂等键）：

| 事件名 | 线协议 Payload 格式 | 契约与校验规则 |
| --- | --- | --- |
| `event.start` | `"0"` 或 `""` | 初始化默认值并渲染 `intake` 阶段回执。 |
| `task.intake` | `"<title>\|<description>"` | 从 `intake -> coordinate` 迁移。 |
| `requirement.add` | `"<title>\|<description>[\|<ext_id>[\|<dep1,dep2>]]"` | 在 `draft` 中添加 `rN`。校验 DAG 无环与 `ext_id` 幂等。 |
| `requirement.dispatch` | `""` \| `"0"` \| `"rN"` | 派发就绪需求，最多达 `job_capacity`（4）。`draft` 行进入 `planning`；`ready`（`planning` + `plan_ok`）行认领泳道并进入 `developing`。 |
| `proposal.submit` | `"<req_id>\|<rev>\|<stage>\|<summary>\|<artifacts>\|[<changes>]\|[<ext_id>][\|exec=<agent_id>[:<reason>]][\|actor=<actor_id>][\|actor_session=<session_id>]"` | 提交针对 `analysis`、`plan` 或 `development` 的提案。保留尾段链固定相对序 `exec → actor → actor_session`（各自可独立缺席）：`exec=` 必须在链中 `actor=` 之前并同事务绑定执行者台账；`actor=` / `actor_session=` 由**宿主注入**（声明路由 + `--actor` / `YGG_ACTOR` / daemon 请求字段），调用方自带同名段在宿主侧即被拒（`ACTOR_FORGED`），mod 侧错位段逐段点名拒绝；三态互证（verified / mismatch / unverified）识别不拒绝，`session` 是纯审计字段永不参与判定（见 §3.7）。 |
| `proposal.accept` | `""` \| `"0"` \| `"pN"` \| `"rN"` | 接受头部（或指定的）待决提案。在 `guide` 模式下需要菜单票据。 |
| `proposal.reject` | `"<pN_or_rN>[\|<reason>]"` | 拒绝待决提案并将需求返回其阶段队列。 |
| `gate.approve` | `""` \| `"0"` \| `"rN[\|<note>]"` | 通过 `plan_gate`（$\to$ `ready` + 响应式自动派发）或 `accept_gate`（$\to$ `done` + 内联泳道释放）。要求 `gate_rev == req.rev`，且对 `accept_gate` 要求非空 `artifacts`。 |
| `gate.reject` | `"<rN>[\|<feedback>]"` | 拒绝 `plan_gate`（`plan_ok=false`）或 `accept_gate`（保留 `plan_ok`），递增 `rev`，并返回 `planning`。 |
| `requirement.amend` | `"<req_id>\|<title>\|<description>\|<deps>"` | 空段保留当前值；`deps="-"` 清空依赖。递增 `rev`。 |
| `requirement.split` | `"<parent_id>\|<child_title_1>\|<child_title_2>[\|...]"` | 将一个开发前需求拆分为 $\ge 2$ 个 `draft` 子需求。 |
| `requirement.merge` | `"<rA,rB>\|<merged_title>\|<merged_desc>"` | 将 $\ge 2$ 个开发前需求合并为一个带并集依赖的新 `draft` 需求。 |
| `requirement.conflict` | `"<rA,rB>\|<note>"` | 将两个需求标记为 `blocked`，直到解决。 |
| `requirement.resolve` | `"<rA,rB>\|<keep_both\|keep_first\|revise>[\|<note>]"` | 清除冲突并应用所选解决策略。 |
| `coordinate.submit` | `""` \| `"0"` | 当所有关闭前置条件满足时，从 `coordinate -> review` 推进（**批边界**）。 |
| `verify.submit` | `"all_green=<true\|false>[;report=<text>]"` | 在 `review` 中记录验证结果。 |
| `settlement.evolution` | `"<artifact>\|<summary>[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/0/<dest>][\|todo=0/<verified_by>]"` | 在一个事务中注册演进筛选并结算会话/todo 清理（**每批一次**）。 |
| `review.note` | `"<lesson>\|<score_1_to_5>"` | 将 `settlement.evolution.*` 归档进 `review.evolution.*`；未武装 = 批收口回 `coordinate`（`run.batch` +1，无 `[ENDING`），`run.retire` 武装 = 退役终局（`[ENDING · run retired]`）。 |
| `config.set` | `"<key>=<value>"` | 更新运行时 `settings.<key>`（`play.mode`、`goal.authorization`、`run.retire`（`true`/`""` 退役武装/解除）、`executor.<rN>.<stage>`、`evolution.mode`、`gates.require_all_green`、`review.evolution.*`）。 |
| `vfs.write` | `"artifacts/<name>.md\|<content>"` | 经 `ygg.fs.write` 将内容写入 `run.sqlite`（`vfs_files`），并在 `artifact.*` 中记录 SHA-256 + 字节长度。 |

### 3.4 三层动作面契约：`event` / `op` / `action`

Yggdrasill 的动作面被严格分成三层，任何一层都不得越权承担另一层的职责。这三个词在本文档后文一律按此定义使用，不许混用。

| 层 | 载体 | 值域 | 权威来源 | 职责边界 |
| --- | --- | --- | --- | --- |
| **L1 `event`** | `ygg.on(event_name, handler)` 注册的 handler（`stage.advance` 由 `ygg.fsm.define` 转手注册） | 33 个事件名（全集见本节末注，payload 位次见 §3.3；`guide.show` 已随 r1 注销） | Mod（`.ygg/mods/agent-seed/src/**`（core/domain/view/runtime/handlers 五层）、`mod.lua`） | **唯一的状态推进货币**：`(state, event) -> state'`，全有或全无（见 CONTEXT `P1`）。事件之外不存在任何改状态的路径。 |
| **L2 `op`** | daemon JSON-RPC 请求的 `op` 字段 | `ping` / `summary` / `dispatch` / `vfs_put` / `vfs_get` / `vfs_list` / `settle` / `shutdown`（8 个） | Rust 宿主（`crates/ygg-sandbox/examples/agent.rs`）；线协议见 §3.5 | **状态推进之外的旁路能力**：制品读写、摘要查询、结算报告、进程控制。`op=vfs_put` 与 `event=vfs.write` 是**两条平行端口**，互不经过对方，且语义不等价（见下文对照表）。 |
| **L3 `action`** | `ygg_session` 工具的 `action` 参数（`StringEnum`） | `kickoff` / `dispatch` / `approve_gate` / `reject_gate` / `vfs_write` / `vfs_read` / `vfs_list` / `handoff_compact` / `settle_close`（9 个） | Layer 2 适配器（`.pi/extensions/ygg-pi-harness`） | **面向模型/人的语义入口**：它只决定「用哪个动词说出意图、由谁拼 payload」，**不决定状态**。值域随适配器可变，**不是**跨 harness 契约。 |

一句话边界：**`event` 决定状态能不能变；`op` 决定旁路能力能不能用；`action` 只决定模型怎么用一句话说出它想干什么。** 因此 `action` 的值域永远可以变，`event` 与 `op` 的值域基本不能变——重叠发生在哪一层，就只在哪一层可能是冗余。

#### 三条硬不变量

1. **I1 · 推进唯一性**：任何 `action` 都**不得**绕过 `op=dispatch` → Mod handler 去改状态。适配器里没有第二套状态机，也没有第二份状态投影。
2. **I2 · 状态不可写性**：适配器**绝不**直接写 `run.sqlite` / `settings.*` / `req.*` / `stage`。`stage` 由 `ygg.fsm` 独占写；其余领域键只由 Mod handler 写（`ygg.kv.set` 的全部领域键都落在 `src/handlers/**` 内）。
3. **I3 · 拒绝不可洗白**：Mod 的回执一旦带 `agent.rejectionPrefixes` 命中的行（`[REJECTED:*]` / `[BLOCKED:*]`），适配器必须**原样回贴**并停止该步的后续编排，绝不复述为成功，也绝不重试到看起来通过。

#### `action` 面的四类落点（每个 `action` 必须可归入且只归入一类）

| 类别 | `action` | 落点 | 允许的 dispatch 次数 |
| --- | --- | --- | --- |
| **A1 · 多步编排** | `kickoff`、`settle_close` | 多个 `event`（`settle_close` 另含 `op=vfs_put` 与 `op=settle`） | $\ge 2$，且**任一步被拒即停** |
| **A2 · 单 event 便利别名** | `approve_gate` → `gate.approve`、`reject_gate` → `gate.reject` | 单个 `event` | 1 |
| **A3 · 事件直通** | `dispatch` | 任意 `event`，payload 由调用方给 | 1 |
| **A4 · 非 event 旁路** | `vfs_write` / `vfs_read` / `vfs_list` → `op=vfs_put` / `op=vfs_get` / `op=vfs_list`；`handoff_compact` → 纯适配器本地（`op=vfs_put` + 宿主压缩） | 0 个 `event` | 0 |

A1 的正确性依赖适配器对 Mod 内部语义的理解（例如 `kickoff` 里 `goal.authorization` 必须先于 `play.mode=goal`，`settle_close` 里 `verify.submit` → `settlement.evolution` → `review.note` 的顺序与 `sessions=` 段的幂等判断）——这条顺序契约**写在适配器里**，Mod 侧不会替它兜底。

#### A2 的准入判据

**当且仅当一个 `action` 能防止一类具体的、可命名的 payload 错位时，它才配拥有一个别名**，并须在适配器的契约测试里以断言钉住。

`approve_gate` / `reject_gate` 命中此判据：`reqId` 必填可防止「漏 `reqId` 时 Mod 退化去批门禁队列里的下一个待批门」——那是**批错需求**的正确性问题，不是省 token；两者把 payload 拼成 `"rN|<note>"`，空 `note` 也必须产出 `"rN|"` 而不是 `"rN"`（位次见 §3.3）。除此之外的 `action` 都没有第二条等价 `event` 路径，因此**不存在可消除的功能重叠**。同一条判据也否掉了零增量别名：一个连「能防哪类具体错误」都答不出来的 `action`（只是另一个名字的同义重复）应当退场，而不是无限期挂着当兼容残留。

#### `dispatch` 与压缩的边界

`dispatch` 的语义严格是：**派发一次事件，返回该次派发的完整回执**。它**不得**附带任何「回合后动作」，尤其不得触发上下文压缩——`ctx.compact()` 会 abort 当前回合，而 §1 与 CONTEXT 都要求「回执整段可见」，两者不可能同时成立。

- 因此 `dispatch` 分支对上下文压力**只报告、不压缩**：回执里给出水位、稳定节点判定与「需要时显式调 `handoff_compact`」的提示。
- 压缩只允许在两个**显式入口**发生：① `action=handoff_compact`（强制，见 §3.6）；② `ygg_run_lanes` 的编排层刹车（编排层已经落下本波并写好交接单之后）。
- 自动触发的水位判断最多只能**报告**，不能自己压。

#### `op=vfs_put` 与 `event=vfs.write` 的不等价（不得互相替代）

同一份字节可以经两条路进 VFS，但只有事件路径带语义守卫与台账：

| 维度 | `event=vfs.write`（走 `action=dispatch`） | `op=vfs_put`（走 `action=vfs_write`） |
| --- | --- | --- |
| 阶段守卫 | `stage == done` 时**拒绝** | **无**（收口后仍可写） |
| 内容上限 | `MAX_VFS_CONTENT` = **64 KiB**（`65536` 字节），超出即 `reject_payload` | 宿主 `validate_artifact_content` = **256 KiB** |
| 路径狱 | `artifacts/<name>.md`、$\le 128$ 字节、无 `..`、无空白（`artifact_path_ok`） | 宿主 `validate_artifact_path`：`artifacts/` 前缀、$\le 128$ 字节、只允许 `[A-Za-z0-9._/-]`、不得含 `..`、不得有空路径段；**不强制 `.md`** |
| `artifact.*` 台账 | **写**（`record_artifact(path, bytes, sha, source="vfs")`，供 `fs.artifact_probe` 与收口核验读取） | **不写**（只 `artifact_put` 落库 + `vfs_load` 灌进内核 `ygg.fs`） |
| `ygg.fs` 缺席时 | `[VFS LEDGER ONLY]` 降级：照样记台账、照样叙事，不声称落盘 | 由宿主直接落库并 `vfs_load`，不存在这次降级 |

结论：凡需要进入 `artifact.*` 台账、需要阶段守卫或 64 KiB 上限的制品，**必须**经 `event=vfs.write`；`op=vfs_put` 只用于「不参与台账」的宿主侧归档（例如交接单 `artifacts/handoff-latest.md` 与 `settle_close` 的进化复盘）。这两个名字长得像，但**不是**同一个原语的两种写法。

#### 本节末注 · `event` 全集（33 个）

`agent.summary`、`task.intake`、`requirement.add` / `requirement.list` / `requirement.dispatch` / `requirement.amend` / `requirement.split` / `requirement.merge` / `requirement.conflict` / `requirement.resolve`、`lane.claim` / `lane.release`、`proposal.submit` / `proposal.accept` / `proposal.reject`、`coordinate.submit`、`verify.submit`、`settlement.evolution` / `settlement.sessions` / `settlement.todo`、`review.note`、`gate.approve` / `gate.reject`、`config.set` / `config.open` / `config.close`、`map.show`、`ygg.choice_selected`、`event.start`、`tool.call` / `tool.result`、`vfs.write`、`stage.advance`。

计数口径（可复核）：前 32 个由 Mod 直接调用 `ygg.on(...)` 注册；`stage.advance` 是第 33 个，它由 `ygg.fsm.define`（`crates/ygg-sandbox/stdlib/fsm.lua`）转手注册，**不在** `ygg.on` 的字面出现处。可执行复算：`grep -rn 'ygg\.on("' .ygg/mods/agent-seed/ | sed 's/.*ygg\.on("\([^"]*\)".*/\1/' | sort -u` 得到 32 项，再加上 `mod.lua` 的 `ygg.fsm.define` 声明里那一项 `event = "stage.advance"`，合计 33。这 33 个事件构成状态机的完整原子转换集合；`action` 面**从不**表示「这 33 个之外还有别的状态转换」。运行期 registry = 这 33 个 + stdlib 内置注册面（`ygg.narrative.goto`，合计 34）；registry 之外的事件名（含拼错的名字与未注册的 `ygg.*`）派发一律 **fail-closed**——外部名在门面预检零副作用拒绝（可读 VALIDATION，文案带已注册全集），派生名（handler `ygg.emit`）使整次转换失败回滚。registry 是运行时唯一事实，宿主不持任何事件名白名单。

### 3.5 宿主 CLI 与 Daemon JSON-RPC 线协议契约

> `op` 面的 8 个操作是状态推进之外的旁路端口；`op=vfs_put` 与 `event=vfs.write` 的语义差异（阶段守卫 / 内容上限 / 台账）见 §3.4。

#### 单次 CLI 与内置工具参数（`ygg-agent`）
- **运行执行**：`.ygg/bin/ygg-agent --mod <mod_dir> --run-id <id> --runs-root .ygg/runs [--resume] [--raw] --event <event> [--payload <json>]`
  - **回执渲染口径（r5）**：缺省为人读文本渲染；`--raw` = 单行 JSON 机器契约（与历史缺省逐字节一致）；解析链 `--raw` > `YGG_FORMAT`（text\|json）> 缺省 text；不改字段契约、退出码与 daemon 协议面。脚本 / CI 解析单发 stdout 必须加 `--raw`（或环境级 `export YGG_FORMAT=json`）。
- **单发派发的未注册事件（fail-closed，r11）**：`--event <name>`（或省略时的 `entryEvent`）不在 Mod registry 内时，派发在**零副作用**下拒绝（不进回放日志、不耗燃料、不动状态基线）：回执 `ok:false` + `error.code=VALIDATION`，`message` 三要素（事件名原样 + 已注册计数 + 已注册全集字典序），退出码 1，审计行 `ok=false` 照写。`ygg.*` 保留事件同样只认 registry（宿主不持事件名白名单）；空/空白事件名仍走参数面（usage / DAEMON 层），与本语义分层。
- **结算**：`.ygg/bin/ygg-agent --mod <mod_dir> --run-id <id> --runs-root .ygg/runs --settle`
- **内置宿主工具（`--builtin-tool <op>`）**：
  - `worktree-claim --lane <1..4> --req <rN> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;req=<rN>;branch=<br>;base=<sha>;clean=1;path=<wt_path>`
  - `lane-probe --lane <1..4> --base <sha> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;commits=<N>;clean=<0|1>;head=<sha>;files=<csv>`
  - `worktree-release --lane <1..4> [--repo <path>]` $\to$ `lane=<k>;released=1;idle=<0|1>`
  - `artifact-probe --path artifacts/<name>.md [--run-dir <dir>]` $\to$ `path=<p>;exists=<0|1>;bytes=<N>;sha256=<hex>`
  - `verify-suite [--mode fast|full] [--repo <path>]` $\to$ `passed=1;summary=<M>/<M>`（在回退到默认 Rust cargo 门禁前，尊重 `YGG_VERIFY_COMMANDS="cmd1 args;cmd2 args"` 环境变量覆盖）。
  - `archive-sessions --parent <id> --batch <run_id>` $\to$ `moved=<N>;remaining=0;dest=<path>`

#### Daemon 模式（`--daemon`）行分隔 JSON-RPC 协议
每个请求是 `stdin` 上的一行 UTF-8 JSON；每个响应是 `stdout` 上的一行 UTF-8 JSON：
- **请求**：`{ "id": <number|string>, "op": "<ping|summary|dispatch|vfs_put|vfs_get|vfs_list|settle|shutdown>", ...params }`
- **响应**：`{ "id": <number|string>, "ok": true, "result": <object> }` 或 `{ "id": <number|string>, "ok": false, "error": { "code": "<DAEMON|VALIDATION|…>", "message": "<text>" } }`（订正：`error` 是**对象形**，与实现一致；`dispatch` 的失败应答是步回执形——附 `op:"dispatch"` 与 `summary`，错误码随内核信封；`op` 面自身错误用 `DAEMON`）
- **提交者身份（r3）**：`op=dispatch` 请求可带 `"actor"` / `"actor_session"` 字段（Agent 工具句柄 / 原生会话 id），仅在旗标（`--actor` / `--actor-session`）与环境变量（`YGG_ACTOR` / `YGG_ACTOR_SESSION`）都缺席时兑底（解析链见 §3.7）；形状非法 → 可读错误应答。派发回执恒回显 `actor` / `actor_session`（None → null，加性键）；inbox 写命令回执另回显 `opener`（仅携带身份时）。

#### 结构化 Summary 与 `HarnessAction` Schema（`result.summary` 与 `result.receipt`）
```json
{
  "stage": "coordinate",
  "ending": false,
  "verifyPassed": false,
  "nextEndsRun": false,
  "lifecycle": "resident",
  "batch": 1,
  "batchDrained": false,
  "pendingProposals": 0,
  "requirementCount": 1,
  "pendingTools": 0,
  "conflicts": [],
  "gates": [],
  "profiles": [
    { "handle": "agent-1", "harness": "pi", "session_id": "01a1c961-…", "role": "plan-executor", "assignments": ["r1:plan", "r1:development"] }
  ],
  "jobs": [
    {
      "id": "r1",
      "rev": 1,
      "stage": "development",
      "status": "developing",
      "phase": "developing",
      "lane": 1,
      "branch": "feature/a3f9-map-single-gone",
      "base": "7a40830",
      "worktreePath": "/Users/.../tmp/worktrees/Yggdrasill-lane-1",
      "worktreeStatus": "ready:/Users/.../tmp/worktrees/Yggdrasill-lane-1@feature/a3f9-map-single-gone",
      "subagentStatus": "pending",
      "probeStatus": "pending",
      "executor": "",
      "planExecutor": "agent-plan-1",
      "title": "...",
      "description": "...",
      "subagentAction": {
        "action": "subagent.resume",
        "reqId": "r1",
        "rev": 1,
        "stage": "development",
        "lane": 1,
        "resumeAgentId": "agent-plan-1",
        "worktreePath": "/Users/.../tmp/worktrees/Yggdrasill-lane-1",
        "artifactPath": "artifacts/r1-development.md",
        "brief": "Requirement r1 (rev 1, stage=development): ...\nMandatory references: read docs/CONTEXT.md (domain invariants) and docs/DESIGN.md (architecture & contracts) before proposing or editing.\n...",
        "deltaPrompt": "Resume r1 (rev 1) for stage `development` in worktree `...`; write `artifacts/r1-development.md` and submit.",
        "fullFallbackBrief": "Requirement r1 ...\nResume context: prior executor `agent-plan-1`.",
        "submitCommand": ".ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --runs-root .ygg/runs --resume --event proposal.submit --payload '\"r1|1|development|<summary>|artifacts/r1-development.md||exec=<agent_id>\"'"
      }
    }
  ]
}
```

### 3.6 上下文压缩与检查点交接契约

> 压缩是**显式**的：它只在 `action=handoff_compact` 与 `ygg_run_lanes` 的编排层刹车两个入口发生；`dispatch` 只报告水位，不触发压缩（见 §3.4）。

长时间的多需求运行通过 Layer 2 中的确定性压缩水位线来防护上下文腐化：
- **有效窗口上限**：默认 `min(modelWindow, 200_000)` token（可经 `YGG_EFFECTIVE_CONTEXT_WINDOW` 覆盖）。
- **水位线**：
  - `ratio < 0.60` $\to$ `normal`
  - `0.60 <= ratio < 0.75` $\to$ `soft_brake`（先让当前活跃子代理波在其门禁处落地，再写交接并压缩）
  - `ratio >= 0.75` $\to$ `hard_brake`（不再派生新子代理；立即写交接并压缩）
- **稳定检查点谓词**：仅当 `summary.available == true` $\land$ `pendingTools == 0` $\land$（`gates.length > 0` $\lor$ `jobs.length == 0` $\lor$ `conflicts.length > 0`）时才允许压缩，除非经 `handoff_compact` 显式强制。r9 常驻语义下，**批边界是天然稳定检查点**（批收口后 `stage=coordinate ∧ jobs=0 ∧ gates=0 ∧ conflicts=0`，谓词常真）；常驻长局推荐在批边界做压缩交接。
- **r9 常驻批排空暂停（drive 泵）**：`summary.lifecycle == "resident"` $\land$ `summary.batchDrained` $\land$ inbox 无未消费行 → 打印等待文案后安全暂停（exit 0）：空批绝不连点 `coordinate.submit`；`lifecycle` 缺失（旧 mod）不触发。
- **交接产物（`artifacts/handoff-latest.md`）**：在压缩前持久化进 `run.sqlite` VFS 并在压缩后重新注入，包含运行 ID、阶段、token 比例、任务表（含 `executor` / `planExecutor`）、待决门禁、未解决冲突、VFS 产物清单，以及精确的下一步 `ygg_session` / `ygg_run_lanes` 命令。

---

### 3.7 serve 观察面契约（只读观察台 + 两级列表 + mod 面板 iframe 托管）

> r8 · rev2。`serve` 是继单发 `--event` / 交互 / `--daemon` 之后的**第四模式**（加性）。
> 不改既有单发 CLI 与 daemon 的任何行为；面板默认只读，gate/accept 类人类门禁
> （`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note`）
> 在 serve 面上**不存在写路径**——路由表只登记 GET，非 GET 一律 `405`（物理不可绕过），
> 人类门禁仍走 CLI 票据 / 适配器路径。

#### 3.7.1 CLI 形态

```text
agent [--runs-root <目录>] serve [--port <1..65535>] [--panel-dir <目录>] [--poll-ms <ms>]
```

- `--runs-root` 缺省 `runs`；目录不存在 → 报错退出 1（fail-closed，不自动创建）。
- `--port` 缺省 7800；恒绑定 **`127.0.0.1` 常量**（无旗标）；被占用 → 报错退出 1（不自动递增）。
- `--poll-ms` 缺省 700，钳制 [200, 5000]（watcher 变更检测周期）。
- `--panel-dir`：内置两级列表页的盘上覆盖（开发期改 HTML 刷新即生效；缺失文件 404）。
- 与其它全部模式互斥（`usage_error`，退出 2）；`--run-id` / `--mod` / `--store` 不存在
  （观察单位 = runs-root，不绑定单局、不装 Mod）。
- 先起 serve 再开局**合法**（一级列表动态出现新局）；局收口后继续提供终局数据；
  run 目录被外部删除 → 相关 API 404 + watcher 摘除，serve 不退出。

#### 3.7.2 四级形态与宿主/mod 边界

| 级 | 归属 | 内容 | 数据源 |
| --- | --- | --- | --- |
| 一级：mod 列表 | egg 原生 | 按 `meta.mod` 分组的实例枚举（组按组内最新 `updated_at` 降序） | runs-root 扫描 + 存储探针（前端对 `/api/runs` 分组推导，serve 零改动） |
| 二级：mod 实例列表 | egg 原生 | 该 mod 的 runs 枚举 | 同上（前端按 key 过滤） |
| 三级：局内存档 | egg 原生 | 该 run 的存档列表 | `ygg_store::observe` 只读观察面 |
| 四级：存档视图 | **mod 自行维护** | mod 自带 UI | 存档数据 API（裸数据） |

宿主对第四级**仅托管静态文件 + 裸数据 API**，零解析零渲染任何 mod 领域语义（P11）。

hash 路由（纯前端，URL=状态、可收藏可刷新）：`#/` = mod 列表；`#/mods/<encodeURIComponent(meta.mod)>` = 实例列表；`#/runs/<id>` = 局内存档。返回导航由前端按当前路由推导（`#nav-back` 空容器；根视图无返回入口；三级父级解析失败——截断/已删——降级为「← mod 列表」），不由点击历史推导——渲染即有意义，结构上不可能 no-op。`meta.mod` 缺失/解析失败的行归键 `-` 的「未知 mod」组，诚实呈现、不做客户端别名合并（别名归宿主 serve 侧，只服务面板托管，不服务分组）。

#### 3.7.3 API 契约总表（全部 GET；非 GET → 405；未知路径 → 404）

| 路由 | 语义 | 响应形状（关键字段） |
| --- | --- | --- |
| `GET /` | 内置两级列表页 `index.html` | text/html |
| `GET /assets/<name>` | 内置页静态资产（白名单 `app.js` / `style.css`） | 按扩展名映射 + nosniff |
| `GET /api/runs` | **第一级**：runs 枚举 | `{version, generatedAt, runsRoot, truncated?, runs:[{runId, meta:<run.json 原样>, coord:<原样\|null>, backend, probe:{generation,latestId,latestCreatedAt,count}, panel:{available,url?,reason?}, metaError?}]}`（updated_at 降序；封顶 200） |
| `GET /api/runs/<id>/saves?limit=` | **第二级**：存档列表 | `{runId, backend, generation, saves:[{id,generation,seq,createdAt,bytes,latest}], panel:{available,url?}}`（limit 缺省 50 封顶 200；JSON 局恒单行 `latest`——单档后端的物理事实） |
| `GET /api/runs/<id>/saves/<sid>` | **存档数据 API**：原始快照字节 | `200 application/json` + `X-Ygg-Run/X-Ygg-Save-Id/X-Ygg-Generation/X-Ygg-Seq/X-Ygg-Created-At/X-Ygg-Bytes`；body = 存档原字节（零包装零重序）；`sid` ∈ `latest \| [0-9]+`（否则 400） |
| `GET /api/runs/<id>/events?since=&limit=` | 事件日志分页（seq 升序） | `{runId, generation, latest, since, records:[{seq,event,payload,ok,timestamp,ending}]}`；`limit` 缺省 200 封顶 500；`payload` 原文透传（`actor=`/`actor_session=` 段随 r3 自动出现） |
| `GET /api/runs/<id>/artifacts` | 当前代制品元信息 | `{artifacts:[{path,bytes,sha256,updatedAt}]}`（path 升序） |
| `GET /api/runs/<id>/artifacts/<path..>` | 制品原文 | 原字节（`validate_artifact_path` 门复用；读侧封顶 1 MiB → 413） |
| `GET /api/stream` | SSE 变更流 | `text/event-stream`（§3.7.5） |
| `GET /panel/<run-id>/[rel]` | mod 面板托管 | 文件字节（§3.7.4；未声明/不可达 → 404 JSON 带 reason） |

错误统一 `{"ok":false,"error":{"code":…,"message":…}}`；响应一律 `Cache-Control: no-store` +
`X-Content-Type-Options: nosniff`。

#### 3.7.4 pack.json `panel` 段契约（挂进 §3.2 的 pack.json 契约旁）

- 声明：`"panel": { "entry": "panel/index.html" }`（**可选顶层段，v1 全部面 = 单字段 `entry`**；
  未来字段只能向后兼容式增量）。语法门 = 不以 `/` 开头、不含 `..`、以 `.html` 结尾；
  违规 → `panel.available=false` + 具体reason，**不是装载错误**（面板声明损坏不能炸 mod 装载）。
- 兼容性：pack.json 解析是宽松 Value 读取，旧宿主忽略未知顶层段；旧 mod 无段 → serve 侧
  `panel.available=false`（降级矩阵五态见实现测试）。面板文件**不进** `entries[]`（内核资源面）
  也**不进** `watched_files`（签名/热载面）——改面板刷新浏览器即生效。
- 托管：运行期现读 + 双守卫（`canonicalize` + 前缀校验，拦 `..` 穿越与 symlink 逃逸）。
- **信任模型（诚实声明）**：mod 面板 JS 与内置页同源（127.0.0.1 同端口）执行，信任级别 = mod
  本身（mod 代码本就持 daemon 工具执行权限）。serve 不消毒不沙箱；防护边界 = 路径双守卫 +
  只读 + localhost 绑定 + nosniff。
- **宿主不提供 SDK/桥**：mod UI 的取数契约 = 同源 `fetch('/api/runs/<id>/saves/<sid>')`（`X-Ygg-*`
  头带元数据）+ `fetch('/api/runs/<id>/events?since=..')` + `new EventSource('/api/stream')`
  （过滤 `frame.run === 本局` 后自行重取）。iframe 传参 = query（`?save=<sid>`，可收藏可刷新）。

**内核存档信封（mod UI 作者依赖面）**：存档本体 = `{engine_version, entities, format_version,
kv:{entries,revision}, pending_choice}`。mod 自己的状态在 `kv.entries`；`format_version` 判版本。
`meta.summary`（`contract.summaryEvent` 派发面）是**内存态派生产、不持久化**——serve 不装载内核故
不供；需要 profiles 的 mod UI 应从自己的存档状态取数（mod 自己写入 kv 的注册表副本）。

#### 3.7.5 SSE 帧协议

- 单一端点 `/api/stream`；watcher 是唯一变更源（轮询 runs-root：名单比对 + 每 run 的
  run.json/coordination.json stat + `save_probe`）。检测到变更 → 全局 `version += 1` → 广播。
- 帧：`event: change` + `data: {"version":<n>,"run":"<runId>"}`（根级变更 `"run":null`）。
  **没有数据载荷**——客户端收帧后重取自己正在看的端点；每 poll 每 run 至多一帧（天然合流）。
- 保活：`: ping` 每 15s；断线自动重连 + `onerror` 降级 2s 轮询（同一批 API）。无
  `Last-Event-ID`（变更帧无连续性语义）。通知 ≤ 一个 poll 周期（最终一致通知，读恒为当下事实）。

#### 3.7.6 安全边界与线程模型

- 绑定 `127.0.0.1` 常量（单测断言字面量）；GET-only（非 GET → 405，请求体不读）；无凭据面；
  解析健壮性（32 KiB 头上限 / 5s 超时 / 64 连接封顶超出 503 / 路径白名单 / query 数值钳制 /
  run-id 语法门）；无 CORS 头；`/panel` 双守卫 + nosniff；观察面零写（`query_only` + 无 migrate +
  先探测后连接，`observe_creates_nothing` / `observe_writes_nothing` 测试锁）。
- **零内核**：serve 不装载 ModBundle/kernel/mlua、不构造 `Session`——数据面只剩文件 stat 与轻量
  SQL（毫秒级），发布/订阅快照机制失去存在理由；mlua 非 Send 约束以「进程无 Lua」最强成立。
  物理锁：`serve_source_has_no_write_path`（生产面源扫描禁写路径）。
- 线程编排：主线程 accept（每连接一线程，封顶 64）；watcher 线程纯变更检测；SSE 每连接
  `wait_timeout` 等广播。唯一共享可变状态 = `version` 广播对（Mutex + Condvar）。

---

## 4. 活文档闭环

`docs/CONTEXT.md` 与 `docs/DESIGN.md` 在每次运行中都作为闭环契约强制执行：

1. **执行前注入**：Mod 的 `subagent_brief` 模板（`data/texts/ 片段装配的 TEXTS`）与扩展的 brief 构建器（`buildSubagentBrief`）向每个 `plan` 与 `development` 子代理 brief 注入一条强制指令：*"在提议或编辑之前，先阅读 `docs/CONTEXT.md`（领域不变量）与 `docs/DESIGN.md`（架构与契约）。"*
2. **结算前同步**：每当某个需求修改了领域概念、架构决策（`D1+`）、模块边界、状态机迁移或线/API 契约，执行者都必须在同一特性分支中、在 `accept_gate` 与**每批的 `review` 盘点结算**（常驻局的批收口或退役）之前更新 `docs/CONTEXT.md` 和/或 `docs/DESIGN.md`。
