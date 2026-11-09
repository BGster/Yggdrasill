# Yggdrasill · 系统架构、行为语义与接口契约（`docs/DESIGN.md`）

> **定位**：`docs/DESIGN.md` 规定 Yggdrasill 的**产品形态、系统行为、状态机语义与规范接口契约**。领域词汇、愿景、架构原则与决策台账（`D1–D20`、`D-MA1..8`）见 [`docs/CONTEXT.md`](CONTEXT.md)。为使本文档在实现重构下保持持久，其契约定义在边界层（**JSON Schema、CLI/JSON-RPC 线协议、Lua 沙箱宿主 API、事件 Payload、状态迁移表**），而非内部源码函数名或行号。
>
> **术语速览**：领域术语的完整定义见 [CONTEXT §2 核心领域词汇](CONTEXT.md#2-核心领域词汇)，本文档不再重复维护。

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
│ Layer 1 · Playable Mod Package (.ygg/mods/pi-agent-seed, test-adventure)   │
│   - Executable Logic: Entry manifest (mod.lua) + pure modules (src/*.lua)  │
│   - Declarative Resources: data/scenes.json, data/config.json,             │
│     data/playbook.json, pack.json, manifest.json                           │
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

由于 Layer 1（`pi-agent-seed`）发出自包含的 `HarnessAction` 规格（`summary.jobs[i].subagentAction`），并经 Layer 0（`ygg.tool.call`）内联执行确定性仪式，任何编码 agent 宿主都能按其能力层级驱动 Yggdrasill：

| 层级 | 宿主环境 | 子代理执行与恢复 | 上下文遥测与压缩 | 集成机制 |
| --- | --- | --- | --- | --- |
| **Tier 1 · 原生扩展** | Pi Coding Agent（`ygg-pi-harness`） | 原生 `Agent({ resume })`，自动回退到 `fullFallbackBrief` | 实时 token 比例（`soft >= 60%`、`hard >= 75%`），自动 `artifacts/handoff-latest.md` + `compact()` | 持久化 `ygg-agent --daemon` stdio JSON-RPC 客户端 |
| **Tier 2 · CLI / Hook 运行器** | Claude Code、Codex CLI、Cursor | 子进程调用或原生任务工具，消费 `subagentAction.brief` 与 `submitCommand` | 在稳定检查点由 Hook 触发的 `handoff_compact` | `ygg-agent --daemon` 或单次 `--resume --event` CLI |
| **Tier 3 · 提示委派** | 通用 LLM 聊天 / 自定义 Agent 循环 | 协调者把 `subagentAction.brief` 复制进子代理上下文并转达 `proposal.submit` | 经 `vfs.write` 手动检查点交接 | 单次 CLI 回执（`narrative` + `pendingChoice`） |

---

## 2. 工作流状态机与行为语义

### 2.1 宏观 Coordinator 阶段机

Coordinator 在 4 个阶段上支配整体运行生命周期：

| 当前阶段 | 事件 | 守卫 / 前置条件 | 下一阶段 | 行为效果 |
| --- | --- | --- | --- | --- |
| `intake` | `task.intake` | 非空 `title\|description` payload | `coordinate` | 记录任务元数据并开启需求协调。 |
| `coordinate` | `coordinate.submit` | `pendingProposals == 0` $\land$ `jobs == 0` $\land$ `conflicts == 0` $\land$ 所有叶子需求 `done` | `review` | 重置 `verify.passed = false` 并清空本轮的 `settlement.*` 键。 |
| `review` | `verify.submit` / `gate.verify_suite` | `all_green=true`（或所有验证命令退出码为 `0`） | `review` | 锁存 `verify.passed = true`（`verify.source = "human"` 或 `"tool"`）。 |
| `review` | `verify.submit` | 当 `gates.require_all_green == true` 时 `all_green=false` | `coordinate` | 将运行退回 `coordinate` 以返工，并重置结算注册。 |
| `review` | `settlement.evolution` | `verify.passed == true` $\land$ 有效的候选/类别计数 | `review` | 注册演进摘要/产物，结算 `sessions=` 与 `todo=`，并触发内联 `harness.archive_sessions`。 |
| `review` | `review.note` | `verify.passed == true` $\land$ `settlement.*` 完整 $\land$（`play.mode != "goal"` $\lor$ `goal.authorization != ""`） | `done` | 将演进记录归档进 `review.evolution.*`，写入 `ending = "done"`，并发出 `[ENDING · run closed]`。 |

### 2.2 需求生命周期与强制门禁语义

每个需求 `rN` 用 5 个规范状态（`draft`、`planning`、`developing`、`done`，外加终态旁路状态 `split`、`merged`、`dropped`）与正交的状态属性（`plan_ok`、`job`、`gate`、`lane`、`rev`、`conflict`、`needs_amend`）存储：

| 存储状态与属性 | 派生展示阶段 | 触发事件 | 目标状态与属性 | 语义契约 |
| --- | --- | --- | --- | --- |
| `draft` | `draft` | `requirement.dispatch` | `planning`（`job="plan"`） | **阶段合并**：直接派发 `plan` 任务，无需单独的 `analysis` 回合。 |
| `planning`（`job="plan"`） | `planning` | `proposal.submit`（`stage="plan"`） | `planning`（`job="plan"`，待决提案 `pK`） | 子代理交付方案产物（`artifacts/rN-plan.md`）并经尾随 `exec=<agent_id>` 绑定 `executor.rN.plan`。 |
| `planning`（待决 `pK`） | `plan_review` | `proposal.accept` | `planning`（`job=""`，`gate="plan"`，`gate_rev=rev`） | 释放任务槽并升起强制 `plan_gate`。 |
| `planning`（`gate="plan"`） | `plan_gate` | `gate.approve` | `planning`（`gate=""`，`plan_ok=true`）$\to$ **响应式自动派发** $\to$ `developing`（`job="development"`，`lane=k`） | 批准 `plan_gate` 置 `plan_ok=true`；当泳道槽空闲且无冲突/门禁阻塞时，响应式钩子立即认领 `lane-k` 并在**同一回合**派发 `development`。 |
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
   - `code`（可选）：`{ "entry": "mod.lua", "modules": ["src/util.lua", ...] }`。当存在时，`entry` 不得出现在 `modules` 中，且每个模块 chunk 顶层由 `ygg_modules::effects` 强制为纯。
   - `entries`：将逻辑资源 ID（`config`、`scenes`、`playbook`）映射到 `{ path, schema }`。
   - `agent`：8 个必填字段（`startEvent`、`summaryEvent`、`choiceEvent`、`endingPrefix`、`rejectionPrefixes`、`configEntry`、`guideEntry`、`guideEvent`）。
   - `inboxRoutes`：声明 `idField`、`externalIdField`、`kindField`，以及每 kind 的 `{ event, fields[] }` 映射，用于异步 JSONL 收件箱摄入。
   - `tools`（可选，**Layer 1 能力白名单**）：将 `<namespace>.<name>` 映射到 `{ kind: "cli" | "http", program, args[], timeoutMs, maxOutputBytes, readOnly, resultEvent }`。占位符用 `{{arg}}` 表示调用参数，用 `$ENV{VAR}` 表示宿主侧密钥解析。
2. **`data/config.json`（运行时默认值与 Layer 2 发布白名单）**：
   - 依据 `packages/schema/schemas/agent-config.json` 校验。
   - 声明 `play.mode`（`"guide"` | `"goal"`）、`evolution`（`mode`、`allow_paths`、`max_per_run`）、`gates.require_all_green`、`drive`（`maxSteps`、`stagnationLimit`、`decideTimeoutMs`）、`settlement`（`requireNonEmptyEvents`、`requireZeroFailedSteps`、`requireEnding`、`requireInboxConsumed`）、`allowedTools`（**Layer 2 发布白名单**，必须是 `pack.json` `tools` 键的超集），以及可选的 `harness` 适配器元数据。
3. **`manifest.json`（内容寻址完整性锁）**：
   - 依据 `packages/schema/schemas/manifest.json` 校验。为每个代码文件与数据资源记录 `{ path, sha256, byteLength }`。经 `./scripts/sync-seed-mod.sh` 维护。

### 3.3 事件 Payload 线协议契约

派发到 `pi-agent-seed` 的所有事件使用管道分隔（`|`）的 UTF-8 字符串 payload（在支持处附带可选尾随 `@ext=<id>` 幂等键）：

| 事件名 | 线协议 Payload 格式 | 契约与校验规则 |
| --- | --- | --- |
| `event.start` | `"0"` 或 `""` | 初始化默认值并渲染 `intake` 阶段回执。 |
| `task.intake` | `"<title>\|<description>"` | 从 `intake -> coordinate` 迁移。 |
| `requirement.add` | `"<title>\|<description>[\|<ext_id>[\|<dep1,dep2>]]"` | 在 `draft` 中添加 `rN`。校验 DAG 无环与 `ext_id` 幂等。 |
| `requirement.dispatch` | `""` \| `"0"` \| `"rN"` | 派发就绪需求，最多达 `job_capacity`（4）。`draft` 行进入 `planning`；`ready`（`planning` + `plan_ok`）行认领泳道并进入 `developing`。 |
| `proposal.submit` | `"<req_id>\|<rev>\|<stage>\|<summary>\|<artifacts>\|[<changes>]\|[<ext_id>][\|exec=<agent_id>[:<reason>]]"` | 提交针对 `analysis`、`plan` 或 `development` 的提案。`exec=` 必须是最后一段，并在同一事务中绑定执行者台账。 |
| `proposal.accept` | `""` \| `"0"` \| `"pN"` \| `"rN"` | 接受头部（或指定的）待决提案。在 `guide` 模式下需要菜单票据。 |
| `proposal.reject` | `"<pN_or_rN>[\|<reason>]"` | 拒绝待决提案并将需求返回其阶段队列。 |
| `gate.approve` | `""` \| `"0"` \| `"rN[\|<note>]"` | 通过 `plan_gate`（$\to$ `ready` + 响应式自动派发）或 `accept_gate`（$\to$ `done` + 内联泳道释放）。要求 `gate_rev == req.rev`，且对 `accept_gate` 要求非空 `artifacts`。 |
| `gate.reject` | `"<rN>[\|<feedback>]"` | 拒绝 `plan_gate`（`plan_ok=false`）或 `accept_gate`（保留 `plan_ok`），递增 `rev`，并返回 `planning`。 |
| `requirement.amend` | `"<req_id>\|<title>\|<description>\|<deps>"` | 空段保留当前值；`deps="-"` 清空依赖。递增 `rev`。 |
| `requirement.split` | `"<parent_id>\|<child_title_1>\|<child_title_2>[\|...]"` | 将一个开发前需求拆分为 $\ge 2$ 个 `draft` 子需求。 |
| `requirement.merge` | `"<rA,rB>\|<merged_title>\|<merged_desc>"` | 将 $\ge 2$ 个开发前需求合并为一个带并集依赖的新 `draft` 需求。 |
| `requirement.conflict` | `"<rA,rB>\|<note>"` | 将两个需求标记为 `blocked`，直到解决。 |
| `requirement.resolve` | `"<rA,rB>\|<keep_both\|keep_first\|revise>[\|<note>]"` | 清除冲突并应用所选解决策略。 |
| `coordinate.submit` | `""` \| `"0"` | 当所有关闭前置条件满足时，从 `coordinate -> review` 推进。 |
| `verify.submit` | `"all_green=<true\|false>[;report=<text>]"` | 在 `review` 中记录验证结果。 |
| `settlement.evolution` | `"<artifact>\|<summary>[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/0/<dest>][\|todo=0/<verified_by>]"` | 在一个事务中注册演进筛选并结算会话/todo 清理。 |
| `review.note` | `"<lesson>\|<score_1_to_5>"` | 将 `settlement.evolution.*` 归档进 `review.evolution.*` 并关闭运行（`[ENDING`）。 |
| `config.set` | `"<key>=<value>"` | 更新运行时 `settings.<key>`（`play.mode`、`goal.authorization`、`executor.<rN>.<stage>`、`evolution.mode`、`gates.require_all_green`、`review.evolution.*`）。 |
| `vfs.write` | `"artifacts/<name>.md\|<content>"` | 经 `ygg.fs.write` 将内容写入 `run.sqlite`（`vfs_files`），并在 `artifact.*` 中记录 SHA-256 + 字节长度。 |

### 3.4 宿主 CLI 与 Daemon JSON-RPC 线协议契约

#### 单次 CLI 与内置工具参数（`ygg-agent`）
- **运行执行**：`.ygg/bin/ygg-agent --mod <mod_dir> --run-id <id> --runs-root .ygg/runs [--resume] --event <event> [--payload <json>]`
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
- **响应**：`{ "id": <number|string>, "ok": true, "result": <object> }` 或 `{ "id": <number|string>, "ok": false, "error": "<message>" }`

#### 结构化 Summary 与 `HarnessAction` Schema（`result.summary` 与 `result.receipt`）
```json
{
  "stage": "coordinate",
  "ending": false,
  "verifyPassed": false,
  "nextEndsRun": false,
  "pendingProposals": 0,
  "requirementCount": 1,
  "pendingTools": 0,
  "conflicts": [],
  "gates": [],
  "jobs": [
    {
      "id": "r1",
      "rev": 1,
      "stage": "development",
      "status": "developing",
      "phase": "developing",
      "lane": 1,
      "branch": "feature/r1",
      "base": "7a40830",
      "worktreePath": "/Users/.../tmp/worktrees/Yggdrasill-lane-1",
      "worktreeStatus": "ready:/Users/.../tmp/worktrees/Yggdrasill-lane-1@feature/r1",
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
        "submitCommand": ".ygg/bin/ygg-agent --mod .ygg/mods/pi-agent-seed --runs-root .ygg/runs --resume --event proposal.submit --payload '\"r1|1|development|<summary>|artifacts/r1-development.md||exec=<agent_id>\"'"
      }
    }
  ]
}
```

### 3.5 上下文压缩与检查点交接契约

长时间的多需求运行通过 Layer 2 中的确定性压缩水位线来防护上下文腐化：
- **有效窗口上限**：默认 `min(modelWindow, 200_000)` token（可经 `YGG_EFFECTIVE_CONTEXT_WINDOW` 覆盖）。
- **水位线**：
  - `ratio < 0.60` $\to$ `normal`
  - `0.60 <= ratio < 0.75` $\to$ `soft_brake`（先让当前活跃子代理波在其门禁处落地，再写交接并压缩）
  - `ratio >= 0.75` $\to$ `hard_brake`（不再派生新子代理；立即写交接并压缩）
- **稳定检查点谓词**：仅当 `summary.available == true` $\land$ `pendingTools == 0` $\land$（`gates.length > 0` $\lor$ `jobs.length == 0` $\lor$ `conflicts.length > 0`）时才允许压缩，除非经 `handoff_compact` 显式强制。
- **交接产物（`artifacts/handoff-latest.md`）**：在压缩前持久化进 `run.sqlite` VFS 并在压缩后重新注入，包含运行 ID、阶段、token 比例、任务表（含 `executor` / `planExecutor`）、待决门禁、未解决冲突、VFS 产物清单，以及精确的下一步 `ygg_session` / `ygg_run_lanes` 命令。

---

## 4. 活文档闭环

`docs/CONTEXT.md` 与 `docs/DESIGN.md` 在每次运行中都作为闭环契约强制执行：

1. **执行前注入**：Mod 的 `subagent_brief` 模板（`data/scenes.json`）与扩展的 brief 构建器（`buildSubagentBrief`）向每个 `plan` 与 `development` 子代理 brief 注入一条强制指令：*"在提议或编辑之前，先阅读 `docs/CONTEXT.md`（领域不变量）与 `docs/DESIGN.md`（架构与契约）。"*
2. **结算前同步**：每当某个需求修改了领域概念、架构决策（`D1+`）、模块边界、状态机迁移或线/API 契约，执行者都必须在同一特性分支中、在 `accept_gate` 与最终 `review` 结算之前更新 `docs/CONTEXT.md` 和/或 `docs/DESIGN.md`。
