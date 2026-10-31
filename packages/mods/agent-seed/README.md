# agent-seed · 双层状态机 + 提案验收的元工作流 Mod

M-Agent 泳道的 seed mod：把「一次完整的 Agent 工作」配置成一个可游玩的 Mod——协调器派活/验收，
每条需求各自跑 analyze → plan → 门禁 → develop → 交付 → 门禁。任何 Agent 装载它 + 注入任务描述
即可开工，**零定制**。怎么玩（构建 / 开局 / 单发 / 异步 / 结算）见仓库根 `AGENTS.md`；本文件只说
Mod 内部结构。

## 零、host 契约（**必填；宿主不含任何默认值**）

宿主（`crates/ygg-sandbox/examples/agent.rs`、`src/playtest.rs`）不含任何事件名字面量与回退默认值：
事件名、inbox 路由、结算口径、推进阈值**全部**从本 Mod 的 `pack.json` 与 `data/config.json` 读。
任何一段缺失 / 字段缺失 / 类型不符 / 非正整数 → **装载期失败**（非零退出或 `ok=false` 报告），
错误文案会点名缺失字段并给回本节的定位。旧 Mod 升级 = 补齐下面四段。

### 1. `pack.json` → `agent`（四个字段全部必填）

```json
"agent": {
  "entryEvent": "event.start",
  "choiceEvent": "ygg.choice_selected",
  "summaryEvent": "agent.summary",
  "endingPrefix": "【结局"
}
```

| 字段 | 含义 | 宿主用途 |
| --- | --- | --- |
| `entryEvent` | 新局入口事件 | 交互模式开局自动派发；`--event` 省略时的缺省 |
| `choiceEvent` | 玩家选择事件 | `--drive cmd` 每步派发；stdin 数字输入 |
| `summaryEvent` | Mod 自报摘要的只读动作 | 自动推进的唯一判定面（回执信封 `meta.summary`） |
| `endingPrefix` | 结局叙事行前缀 | 收口判据（回执 `ending` / 事件日志 / 结算） |

- `choiceEvent` 必须与**内核保留路由**同名（`ygg.choice_selected`）：内核只对该事件名做
  「序号在 `1..=选项数` 内」的域校验，改名即失去这层校验。
- `summaryEvent` 由 Mod 自己 `ygg.on(...)` 注册，回执里用 `ygg.meta("summary", {...})` 给结构化摘要。

### 2. `pack.json` → `inboxRoutes`（必填；不接受异步 inbox 时写 `{}`）

```json
"inboxRoutes": {
  "add": {
    "action": "requirement.add",
    "fields": ["title", "description", "external_id", "depends_on"],
    "write": { "flag": "requirement", "shape": "named" }
  }
}
```

| 键 | 必填 | 含义 |
| --- | --- | --- |
| 路由键（`add` …） | 是 | inbox 行的 `kind` 字面值 |
| `action` | 是 | 派发的内核动作名 |
| `fields` | 是 | wire format 的段序：按此序从记录取值，用 `|` 连接（`external_id` = 幂等键） |
| `write.flag` | 否 | 宿主 CLI 旗标名（`--requirement` → `requirement`）；不写则宿主不提供该写命令 |
| `write.shape` | 否 | `named`（写 `title`/`description` 字段）或 `payload`（整串进 `payload`） |

记录两种形态都认：**命名段**（记录带同名字段）与**预成形 wire**（记录只有 `payload`，原样透传）。
未声明的 `kind` = 坏行（跳过并前移 cursor，不阻塞队列）；未声明的写命令 = 明确拒绝（退出码 3）。

### 3. `data/config.json` → `settlement`（四个开关全部必填）

```json
"settlement": {
  "requireEvents": true,
  "requireNoFailedSteps": true,
  "requireEnding": true,
  "requireInboxConsumed": true
}
```

关掉的项不参与结算 `ok`（但仍逐项记入 `settlement.json` 的 `checks`：`{name, enabled, passed}`）。

### 4. `data/config.json` → `drive`（三项全部必填、必须正整数）

```json
"drive": { "maxSteps": 64, "stallLimit": 12, "stepsPerRequirement": 16 }
```

失败即装载期报错：`pack.json` 的 `agent` / `inboxRoutes`、`data/config.json` 的 `settlement` / `drive`。

## 一、协调器（`fsm.stage`，单机单事件 `stage.advance`）

```text
intake ──task.intake──▶ coordinate ──coordinate.submit──▶ review ──review.note──▶ done
                           ▲                                 │
                           └──────── verify.submit=false ─────┘（门禁失败打回）
```

- `coordinate.submit` 有硬门禁：全部叶子需求 `done` ∧ 0 待验收提案 ∧ 0 在跑 job ∧ 0 未解决冲突。
- `review` 里 `verify.submit=true` 只记录门禁证据（要求全绿时才允许之后收口），`review.note` 才收口。

## 二、需求级流水线（`req.<i>.status`）

```text
draft ─dispatch→ analyzing ─proposal→ analysis_review ─accept→ planning
      ─dispatch→ planning(job) ─proposal→ plan_review ─accept→ plan_gate
      ─gate.approve→ ready ─dispatch→ developing(lane) ─proposal→ dev_review
      ─accept→ accept(执行方交付) ─gate.approve→ done
```

| 状态 | 含义 |
| --- | --- |
| `draft` / `planning` / `ready` | 可派活（分别派 analysis / plan / development） |
| `analyzing` / `developing` | job 在跑（`req.<i>.job` 标记阶段；开发另占 `lane.<k>`） |
| `analysis_review` / `plan_review` / `dev_review` | 提案已到、待主 Agent 串行验收 |
| `plan_gate` | **强制门禁**：plan 批了才能 `ready` → 开发 |
| `accept` | **执行方已交付**（自检 + artifacts），不等于验收；等 accept_gate |
| `done` / `split` / `merged` / `dropped` | 终态（拆并/弃用保留可追溯性） |

另有标记：`conflict`（+`conflict_peer`，展示为 `blocked`）、`needs_amend`（待修订）、
`stale`（提案 rev 过期）、`gate_rev`（门禁绑定 revision）。

**两个强制门禁**（真实状态 + 动作约束，goal 也不能跳）：

1. `plan_gate`：`gate.approve` → `ready`；`gate.reject` → `planning`（重做）。
2. `accept` → accept_gate：`gate.approve` 要求 **artifacts 非空** → `done`；`gate.reject` → `ready`（重进开发队列）。
   两者都校验 `gate_rev == req.rev`，待批期间被 amend 即判 stale，不得批准。

## 三、提案协议（subagent 只建议，不改共享状态）

```text
proposal.submit  "r1|1|analysis|summary|artifacts[|changes[|external_id]]"
proposal.accept  "pN" / "rN" / ""      → 空 = requirement_id canonical 序取一条（串行验收）
proposal.reject  "pN|理由" / "rN|理由" / ""
```

- `stage ∈ analysis | plan | development`，必须与需求当前状态匹配（`analyzing` / `planning`(job) / `developing`）。
- `changes` 是**拟议变更**，验收时才应用：`split=a,b` / `merge=r5` / `amend=标题:描述` / `conflict=r5:说明`。
- 验收时 `rev ≠ req.<i>.rev` → 提案置 `stale`、需求置 `stale` 标记，可读拒绝（旧提案不得覆盖新决定）。
  **重定基**：直接提交同阶段、**当前 rev** 的新提案即可——`analysis_review` / `plan_review` /
  `dev_review` 在「该需求没有待验收提案」时接受；仍有正常 pending 提案时拒绝（`先 accept / reject`），
  不覆盖；重定基提案落地后 `stale` 自动清除。`external_id` 幂等（`【提案重放】`）。
- 多个需求可以同时在 analyze / plan / develop（job 上限 4），**验收一次只处理一条需求**。

## 四、主 Agent 的需求图命令（串行）

| 动作 | 载荷 | 说明 |
| --- | --- | --- |
| `requirement.split` | `父id\|子A\|子B[|…]` | 父置 `split`，子需求 `draft` 并**继承父依赖**（≥2 子） |
| `requirement.merge` | `id1,id2\|标题\|描述` | 源置 `merged`（`into` 指向新需求），新需求依赖取**并集** |
| `requirement.amend` | `id\|标题\|描述\|依赖` | rev+1；`draft/planning/ready` 直改、开发中只改说明、`done` **派生新需求** |
| `requirement.conflict` | `id1,id2\|说明` | 双向标 `blocked`；冲突需求不派活、不许收口 |
| `requirement.resolve` | `id1,id2\|keep_both\|keep_first\|revise` | 保留双方 / 放弃第二条 / 保留但置待修订 |

终态（`split`/`merged`/`dropped`）不再接受修订与冲突；`coordinate.submit` 会把拆并的**后继**一并要求 done。

## 五、job 槽位与开发泳道

- `tuning.job_capacity`（默认 4）：分析 / 规划 / 开发共用 job 槽位（`req.<i>.job` 非空即占用）。
- 开发额外占 lane（`lane.<k>.req`）；只有「`ready` ∧ 依赖全部满足 ∧ 无冲突 ∧ 不需要修订」的需求能进开发。
- 相互声明冲突的需求不可能同时 `developing`（冲突标记让两者都不可调度）。
- **内核仍单线程 dispatch + 单 autosave**：泳道只是宿主侧并行工作单元，结果一律经 inbox 回来，
  宿主按 `requirement_id` / `external_id` canonical 序合流。

## 六、宿主侧跨进程协议（Mod 不感知，但影响玩法）

Mod 本身无 IO，跨进程互斥全部在宿主的 run 目录里：

| 文件 | 作用 |
| --- | --- |
| `coordination.json` | `{phase: running\|closing\|finalized, writers, owner_pid, heartbeat_at}` |
| `coordination.lock` | 短临界区互斥（`create_new`；陈旧按 mtime 2s 抢占） |

- **写命令**（`--requirement` / `--proposal` / …）：先在 `running` 下登记 `writers`，行落盘后才摘除；
  `closing` → 非零拒绝；`finalized` → 非零拒绝（需开新局）。
- **收口**：goal 在最后的 `review.note` 前切到 `closing`，等在途写者归零，**同一保护下**最后一次 drain：
  有新需求就解除 closing 继续 loop，无新需求才 `review.note` + settle → `finalized`。
- **崩溃恢复**：closing 用心跳续命（10s），过期后写命令或 resume 会恢回 `running`；锁也按 mtime 抢占，
  不会永久卡死。

### 宿主读的摘要字段（`agent.summary` → `ygg.meta("summary", …)`）

宿主（`crates/ygg-sandbox/examples/agent.rs`）对「何时收口 / 有无冲突 / 有无待验收提案 / 是否有 job
空等」的认知**只来自本 Mod 自报的结构化摘要**，不读任何 `req.*` / `prop.*` / `fsm.stage` / `ending` 私有键：

```lua
ygg.on("agent.summary", function()
  ygg.meta("summary", { stage = ..., ending = ..., verifyPassed = ..., nextEndsRun = ...,
                        pendingProposals = ..., requirementCount = ...,
                        conflicts = { { id = ..., peer = ..., note = ... } },
                        awaiting = { req = ..., rev = ..., stage = ... } })  -- 无在跑 job 时缺省
  render()
end)
```

- 数据落在**回执信封**的 `meta.summary`（内核 `ygg.meta` 结构化回执通道），与人读叙事完全分离：
  叙事里**没有** `【摘要】` 前缀行，宿主也不从叙事文本里剖 JSON。
- 该动作**只读**：不推进状态、不改 KV、`pendingChoice` 保持不变（只重渲染一屏）。
- `meta` 是瞬态回执面，不进存档也不进状态哈希（调不调它不影响 `state_hash`）。
- 值面约束（内核把关）：标量 / 稠密序列 / 字符串键表；非有限值拒绝；空表读作 `[]`。

## 七、增量配置

- 静态 `data/config.json` + 动态 KV `settings.*`（`config.set`）：`play.mode`（guide/goal）、
  `gates.require_all_green`、`evolution.mode` / `evolution.max_per_run`。
- `tuning`：`req_capacity`（需求上限 32）、`job_capacity`（4）、`min_slices` / `max_slices`（历史遗留）、
  `default_score`。

## 八、文件

| 文件 | 作用 |
| --- | --- |
| `mod.lua` | Mod 本体：协调器 FSM / 需求流水线 / 提案与门禁 / 泳道 / 渲染 / 解析 |
| `data/scenes.json` | 文本资源（scenes / endings / prompts / options / beats / tuning） |
| `data/playbook.json` | 现场规则资源（`guide.show` 按 section id 取用） |
| `data/config.json` | 静态配置默认值 |
| `pack.json` / `manifest.json` | 打包声明与内容寻址清单（`cargo run --example pack` 生成） |
| `schemas/*.json` | 资源 schema（与 `packages/schema/schemas/` 单源一致） |

## 九、走查

```bash
# 默认全 1 贪心：intake → coordinate.submit → verify → review.note → 【结局
cargo run --example playtest -- .ygg/mods/agent-seed

# 多需求 multi-stage（外部提案；host 不再生成任何占位提案）：
#   .ygg/bin/ygg-agent --run-id t --runs-root .ygg/runs --requirement '需求A|描述'
#   .ygg/bin/ygg-agent --run-id t --runs-root .ygg/runs --proposal 'r1|1|analysis|结论|a.md'
#   .ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id t --runs-root .ygg/runs --resume

# 改完 data/ 或 schemas/ 必跑（manifest 内容寻址）
cargo run --example pack -- .ygg/mods/agent-seed
```
