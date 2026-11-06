# pi-agent-seed · 双层状态机 + 提案验收的元工作流 Mod

M-Agent 泳道的 seed mod：把「一次完整的 Agent 工作」配置成一个可游玩的 Mod——协调器派活/验收，
每条需求各自跑 analyze → plan → 门禁 → develop → 交付 → 门禁。任何 Agent 装载它 + 注入任务描述
即可开工，**零定制**。怎么玩（构建 / 开局 / 单发 / 异步 / 结算）见仓库根 `AGENTS.md`；本文件只说
Mod 内部结构。

## 零、host 契约（**必填；宿主不含任何默认值**）

宿主（`crates/ygg-sandbox/examples/agent.rs`、`src/playtest.rs`）不含任何事件名字面量与回退默认值：
事件名、inbox 路由、结算口径、推进阈值**全部**从本 Mod 的 `pack.json` 与 `data/config.json` 读。
任何一段缺失 / 字段缺失 / 类型不符 / 非正整数 → **装载期失败**（非零退出或 `ok=false` 报告），
错误文案会点名缺失字段并给回本节的定位。旧 Mod 升级 = 补齐下面四段。

### 1. `pack.json` → `agent`（八个字段全部必填）

```json
"agent": {
  "entryEvent": "event.start",
  "choiceEvent": "ygg.choice_selected",
  "summaryEvent": "agent.summary",
  "endingPrefix": "【结局",
  "metaKey": "summary",
  "rejectionPrefixes": ["【动作被拒】", "【载荷被拒】", "【需求待办】"],
  "replayPrefix": "【需求重放】",
  "defaultPayload": "0"
}
```

| 字段 | 含义 | 宿主用途 |
| --- | --- | --- |
| `entryEvent` | 新局入口事件 | 交互模式开局自动派发；`--event` 省略时的缺省 |
| `choiceEvent` | 玩家选择事件 | `--drive cmd` 每步派发；stdin 数字输入 |
| `summaryEvent` | Mod 自报摘要的只读动作 | 自动推进的唯一判定面 |
| `endingPrefix` | 结局叙事行前缀 | 收口判据（回执 `ending` / 事件日志 / 结算） |
| `metaKey` | 摘要结构化字段在回执信封 `meta` 里的键名 | 宿主只读 `meta.<metaKey>`，不读 Mod 私有 KV |
| `rejectionPrefixes` | 「动作未被接受」的叙事行前缀集合（≥1 条） | 宿主据此判定**未生效**（不推 cursor、不算成功） |
| `replayPrefix` | 「幂等重放」的叙事行前缀 | 宿主据此判定重放（算成功消费，只是没新增） |
| `defaultPayload` | 入口事件首次派发的载荷（JSON 标量文本；可为空串） | 开局派发 / `--payload` 缺省 / `event` 命令省略载荷 |

- `choiceEvent` 必须与**内核保留路由**同名（`ygg.choice_selected`）：内核只对该事件名做
  「序号在 `1..=选项数` 内」的域校验，改名即失去这层校验。
- `summaryEvent` 由 Mod 自己 `ygg.on(...)` 注册，回执里用 `ygg.meta("<metaKey>", {...})` 给结构化摘要。
- `rejectionPrefixes` 必须是 **Mod 真实在用的叙事前缀**：写错了宿主就把被拒派发当成已生效
  （早前宿主硬编码的第三条 `【需求待排】` 就是这种失效——本 Mod 实际是 `【需求待办】`）。

### 2. `pack.json` → `inboxRoutes`（必填；三个保留键与 kind 同层）

```json
"inboxRoutes": {
  "externalIdField": "external_id",
  "payloadField": "payload",
  "extMarker": "@ext=",
  "add": {
    "action": "requirement.add",
    "fields": ["title", "description", "external_id", "depends_on"],
    "write": { "flag": "requirement", "shape": "named" }
  }
}
```

工具结果走同一条 inbox 通道（kind 由 Mod 声明），所以 `tool_result` 只是一条普通路由：

```json
"tool_result": {
  "action": "tool.result",
  "fields": ["call_id", "tool_id", "status", "code", "output", "external_id"],
  "write": { "flag": "tool-result", "shape": "payload" }
}
```

| 键 | 必填 | 含义 |
| --- | --- | --- |
| `externalIdField` | 是 | inbox 记录里**幂等键**的字段名（也用于 `fields` 里标记幂等键段） |
| `payloadField` | 是 | inbox 记录里**预成形载荷**的字段名（整串 wire，宿主不解析） |
| `extMarker` | 是 | 幂等键拼到预成形载荷末尾时的**后缀标记** |
| 路由键（`add` …） | 是 | inbox 行的 `kind` 字面值（**不得与三个保留键同名**） |
| `action` | 是 | 派发的内核动作名 |
| `fields` | 是 | wire format 的段序：按此序从记录取值，用 `\|` 连接；值等于 `externalIdField` 的段填幂等键 |
| `write.flag` | 否 | 宿主 CLI 旗标名（`--requirement` → `requirement`）；不写则宿主不提供该写命令 |
| `write.shape` | 否 | `named`（前两个非幂等键段依次填「首段 / 余下」，典型即标题与描述）或 `payload`（整串进 `payloadField`） |

- 三个保留键**必填**，但**允许一个 kind 都不声明**（= 本 Mod 不支持异步 inbox；宿主写命令退出码 3）。
- 保留键与 kind 同层：kind 名不得与它们同名（同名会被当成保留键消费，形不成路由）。
- `write.shape = "named"` 要求 `fields` 里至少有一个非幂等键段（否则写出来的记录无载荷，装载期报错）。

记录两种形态都认：**命名段**（记录带同名字段）与**预成形 wire**（记录只有 `payloadField` 指名的载荷，原样透传）。
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

失败即装载期报错：`pack.json` 的 `agent` / `inboxRoutes` / `tools`、`data/config.json` 的 `settlement` / `drive`。

### 5. `data/config.json` → `allowedTools`（可选段）

```json
"allowedTools": ["git.head_short", "git.status"]
```

工具两层白名单的 **Layer 2**（Layer 1 = `pack.json` 的 `tools`，宿主装载期校验）：

- 缺失 / 非数组 / 元素非法 = **一个也不放行**（fail-closed）；逐项按集合语义处理，重复项允许。
- 必须 **⊇** `pack.json` 的 `tools` 键（宿主装载期硬校验，缺一个即拒绝装载）。
- Mod **读不到** `pack.json`（`ygg.data` 只读 `entries` 声明的资源），所以本 Mod 只认这一层；
  跨层一致性由 `agent_seed_semantics` 的声明一致性用例钉住（两侧 `tools` 深相等、`allowedTools` 与
  `tools` 同集合、19 个 `tool_*` beat 齐全）。
- **不是**安全边界，而是**防漂移**门禁：能改 `data/config.json` 的人也能改 `pack.json`；真正的边界是
  宿主的 Layer 1 校验（程序白名单 / 无 shell / 无绝对路径 / 无 `..`）加上沙箱本身没有 IO。

## 一、协调器（`fsm.stage`，单机单事件 `stage.advance`）

```text
intake ──task.intake──▶ coordinate ──coordinate.submit──▶ review ──review.note──▶ done
                           ▲                                 │
                           └──────── verify.submit=false ─────┘（门禁失败打回）
```

- `coordinate.submit` 有硬门禁：全部叶子需求 `done` ∧ 0 待验收提案 ∧ 0 在跑 job ∧ 0 未解决冲突。
- `review` 里 `verify.submit=true` 只记录门禁证据；随后必须先 `settlement.evolution` 登记，`review.note` 才单源归档并收口。

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
2. `accept` → accept_gate：`gate.approve` 要求 **artifacts 非空** → `done`；`gate.reject` 保留 `plan_ok`
   （未 amend → `ready` 重进开发队列；待批期间被 amend 则 `plan_ok` 已作废 → 停在 `planning` 强制重规划）。
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
| `requirement.split` | `父id\|子A\|子B[\|…]` | 父置 `split`，子需求 `draft` 并**继承父依赖**（≥2 子） |
| `requirement.merge` | `id1,id2\|标题\|描述` | 源置 `merged`（`into` 指向新需求），新需求依赖取**并集** |
| `requirement.amend` | `id\|标题\|描述\|依赖` | rev+1；`draft/planning/ready` 直改、开发中只改说明、`done` **派生新需求** |
| `requirement.conflict` | `id1,id2\|说明` | 双向标 `blocked`；冲突需求不派活、不许收口 |
| `requirement.resolve` | `id1,id2\|keep_both\|keep_first\|revise` | 保留双方 / 放弃第二条 / 保留但置待修订 |

终态（`split`/`merged`/`dropped`）不再接受修订与冲突；`coordinate.submit` 会把拆并的**后继**一并要求 done。

## 五、人类动作门禁与二级菜单页（展示 / 防自批两层，不改状态机）

### 5.1 人类动作门禁

引擎不区分「谁派发的」——`--event 动作` 与 TTY 按键走同一套通道。所以 Mod 自己把这层
从隐式变成显式：**guide 下「会产生通过效果」的动作只能经 `ygg.choice_selected` 路由进入**，
单发直派被可读拒绝（`【动作被拒】`，落在 `pack.json.agent.rejectionPrefixes` 内——宿主因此
判为「未生效」，不前移 cursor、不记失败步，`ok` 仍为 `true`）：

| 受门禁的动作 | 理由 |
| --- | --- |
| `proposal.accept` | 验收判断（自批 = 伪造验收结论） |
| `gate.approve` | 两个强制门禁 |
| `coordinate.submit` | 收口进 review |
| `verify.submit` | 门禁自检结论 |
| `review.note` | 结局评分 / 经验 + settlement 单源归档（`lesson|score`） |

- 打回类（`gate.reject` / `proposal.reject`）**故意不在内**：打回不产生任何通过效果，自打回
  只会让自己返工；而它是唯一的纠错通道（菜单本就没有带理由的 reject 项）。
- 机制是**一次性票据**：`ygg.emit` 是入队而非同步派发，所以不能用调用栈深度表达「这一步来自
  菜单」——菜单路由在 emit 前盖章，对应 handler 取证即消费；非人类动作不盖章，票据不会泄到
  下一个直派。
- `goal` 模式同样走 `ygg.choice_selected`，但是否派发由外部决策命令决定（`HUMAN:…` 即安全
  暂停），不会默认批准。**所以 `--drive cmd` 的决策脚本等于「人」**——它的判断力由脚本自负。

### 5.2 二级菜单页

顶层菜单只留一个 `config` 入口，三项设置进子页：运行模式 / 演进偏好 / 门禁。进化声明的预登记键
（`review.evolution.artifact` / `summary` / `candidates` / `classes`）**不占菜单项**——菜单没有自由文本输入，
所以它们走 `config.set` 直派，写 `settings.review.evolution.*`；verify 通过后的 `settlement.evolution`
菜单项读取这些值构造载荷并集中校验（见 §八）。

| 动作 | 载荷 | 作用 |
| --- | --- | --- |
| `config.open` | `""` | 进子页（KV `ui.page=config`、`ui.page.stage=<当前阶段>`） |
| `config.close` | `""` | 回阶段菜单 |

- 纯展示层：**不参与状态机**，只改 `ui.page` / `ui.page.stage` 两个展示键；不进摘要、不影响
  派活，也**不占顶层首项**（主前进动作永远是 `pendingChoice` 第 1 项，`playtest` 全 1 照旧通关）。
- 清空时机：阶段变化（`advance()` 清、`render()` 再兜一次）、选了任何非 `config.*` 的菜单项、
  以及**切换运行模式**——驱动者换了人，子页处境已失效；不这样做，goal / `--drive cmd` 会在
  子页首项上来回翻模式直到步数上限。

## 六、job 槽位与开发泳道（含账本）

- `tuning.job_capacity`（默认 4）：分析 / 规划 / 开发共用 job 槽位（`req.<i>.job` 非空即占用）。
- 开发额外占一条泳道，账本是 `lane.<k>.{req,branch,base,since,clean}`（`k` = 1..`job_capacity`；
  **单仓无 `repo` 层**——多仓版是 `lane.<repo>.<k>.*`，本 Mod 不移植那一层，泳道号即全局槽位号）：

| 字段 | 含义 |
| --- | --- |
| `req` | 占用者需求 id（`""` = 空闲）。**兼容读**旧存档的数字 index 写法，故不需要一次性迁移 |
| `branch` / `base` | 物理 worktree 的分支与 base（由 `lane.claim` 记）；`requirement.dispatch` 自动挂牌时留空 = **未断言** |
| `since` | 挂牌序号（单调递增，KV `lane.seq`）。沙箱没有墙钟，`since` 只表达先后；真实时间由主 Agent 写在 worktree 根的 `.lane` |
| `clean` | `"1"` = 主 Agent 已断言工作区干净（**mod 不能跑 git**，只认这个断言；空 = 未断言） |

- 只有「`ready` ∧ 依赖全部满足 ∧ 无冲突 ∧ 不需要修订」的需求能进开发；`requirement.dispatch`
  在**同一次事务**里挂牌（写五键 + `req.<i>.lane`）。若主 Agent 已用 `lane.claim` 预挂牌，
  dispatch **沿用**那条，不覆盖已记的 `branch` / `base` / `clean`。
- 释放：执行方交付（进 `accept`）、门禁吊销、拆并 / 弃用都走同一处 → `LANES.release`
  （五键一起清，且只在账本确实记着它时才清，不误伤别人）。
- `map.show` 的账本块逐条显示 `lane-<k> ← <需求>（阶段）｜分支｜base｜since #N｜clean`；
  空字段显示 `—`（未断言的部分不假装知道）。
- 相互声明冲突的需求不可能同时 `developing`（冲突标记让两者都不可调度）。
- **内核仍单线程 dispatch + 单 autosave**：泳道只是宿主侧并行工作单元，结果一律经 inbox 回来，
  宿主按 `requirement_id` / `external_id` canonical 序合流。

| 泳道动作 | 载荷 | 作用 |
| --- | --- | --- |
| `lane.claim` | `"泳道号\|需求id\|分支\|base\|clean"` | 挂牌 / 补记。`clean` 必须是 `"1"`（脏树禁挂牌）；泳道被别人占 / 本条已持有另一条 / 需求终态 → 可读拒绝 |
| `lane.release` | `"泳道号"` | 摘牌（幂等：本就空闲也算成功） |

两者都**不进**人类动作门禁：只记账本，不产生「通过」效果。

## 七、宿主侧跨进程协议（Mod 不感知，但影响玩法）

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
                        evolution = { artifact = ..., summary = ..., candidates = ..., classes = ... },
                        awaiting = { req = ..., rev = ..., stage = ... } })  -- 无在跑 job 时缺省
  render()
end)
```

- 数据落在**回执信封**的 `meta.summary`（内核 `ygg.meta` 结构化回执通道），与人读叙事完全分离：
  叙事里**没有** `【摘要】` 前缀行，宿主也不从叙事文本里剖 JSON。
- 该动作**只读**：不推进状态、不改 KV、`pendingChoice` 保持不变（只重渲染一屏）。
- `meta` 是瞬态回执面，不进存档也不进状态哈希（调不调它不影响 `state_hash`）。
- `evolution` 四个子键是**声明回显**（`review.evolution.*` 的同一份值，按 artifact / summary /
  candidates / classes 顺序缺省 `""` / `""` / `"0/0"` / `""`）：
  它可以被断言「声明了没有」，不能拿来断言建议质量或复核独立性。
- 值面约束（内核把关）：标量 / 稠密序列 / 字符串键表；非有限值拒绝；空表读作 `[]`。
- **回执面有两个独立键**：`meta.summary`（人/宿主的处境摘要）与 `meta.tool_calls`（工具请求队列，
  稠密数组 `[{id, tool, args?}]`，只在 `tool.call` / `tool.result` 两条 dispatch 上出现，终态后为 `[]`）。
  两者是 `ygg.meta` 的同层 `BTreeMap` 两个槽位（`host_api.rs` 的按键插入），互不覆盖；`meta` 仍是瞬态面
  （不进存档 / 不进状态哈希）。摘要里的 `pendingTools` 就是 `tool.pending` 的只读回显。

## 八、增量配置

- 静态 `data/config.json` + 动态 KV `settings.*`（`config.set`）：`play.mode`（guide/goal）、
  `gates.require_all_green`、`evolution.mode` / `evolution.max_per_run`（**仅叙事偏好**：不自动修订、
  不登记修订请求、不参与任何判定）、`review.evolution.artifact` / `review.evolution.summary` /
  `review.evolution.candidates` / `review.evolution.classes`（进化候选预登记，见下）。
- 顶层菜单只暴露一个 `config` 入口，三项常用设置走二级页（见 §5.2）；其余键仍可直接
  `config.set "<键>=<值>"` 派发。**配置写一律不设人类动作门禁**（不产生「通过」效果）；`play.mode` 的切换是
  决策权移交，由 AGENTS §8 约束（用户当次授权下可自行切，goal 局收口前要交代授权），不走票据闸。
- `goal.authorization`（run 级 goal 授权，见 §8.2）与 `executor.<rN>.<stage>`（执行者账本，见 §8.3）同样走
  `config.set` 直派，不占菜单项。
- `tuning`：`req_capacity`（需求上限 32）、`job_capacity`（4）、`min_slices` / `max_slices`（历史遗留）、
  `default_score`。

### 8.1 进化候选登记（`settlement.evolution`）与归档（`review.note`）

```text
settlement.evolution "artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]"
review.note "lesson|score"
```

| 段 | 形状 | 缺省 | 归档到 |
| --- | --- | --- | --- |
| `artifact` | 可空；非空须为 `artifacts/<file>.md`，禁 `..` 与空白 | 空 | `review.evolution.artifact` |
| `summary` | 去空白后必填，不能含 `|` | 菜单用中性默认文案 | `review.evolution.summary` |
| `c` | 两个非负整数，`kept ≤ screened` | 先回落预登记，再 `0/0` | `review.evolution.candidates` |
| `cls` | `<kind>:<n>` 逗号分隔，kind ∈ `long_term` / `unused` / `one_off` / `temp_tool` / `redundant` / `no_scene` | 先回落预登记，再空 | `review.evolution.classes` |

- **登记前件**：只在 review 且 `verify.passed=true` 后可用，每轮 done 前都必须登记。成功写
  `settlement.evolution.{artifact,summary,candidates,classes}`；`c=0/0` 允许 artifact 为空，避免零候选造空文档。
- **预登记与完整性**：guide 可先 `config.set review.evolution.artifact|summary|candidates|classes=<值>`
  （空 summary = 清除），逐键校形状；跨字段完整性只在 `settlement.evolution` 校。`screened > 0` 时
  artifact 与 `cls` 必填，`cls` 总和 = `screened`，保留类（`long_term` + `unused` + `redundant`）总和 = `kept`。
  可选段只接受 `c=` / `cls=`；`ev=`、重复段与未知段均【载荷被拒】。
- **菜单不变式**：verify 已通过且未登记时首项是 `settlement.evolution`，载荷由 settings 预登记构造；
  未预登记时给空 artifact、中性 summary 与 `c=0/0`。登记后首项才是 `review.note`，保持 `nextEndsRun`
  契约；默认零候选载荷保证全 1 走查可按「登记 → 收口」通关，不引入 disabled 项。
- **单源归档**：`review.note` 只接受 `lesson|score`；旧 `ev=` / `c=` / `cls=` 段一律拒绝并提示先
  `settlement.evolution`。它只把本轮 settlement 四键归档到 `review.evolution.*`，不再读载荷或 settings
  重算。`coordinate.submit` 重入 review、以及 `verify.submit=false` 打回时清空本轮 settlement 四键，
  但保留 settings 预登记。
- **声明 ≠ 证明**：沙箱读不到文件，Mod 只校声明形状与计数自洽；不校验 artifact 内容、也不知道复核是否独立执行。
  `agent.summary.evolution` 回显归档的 artifact / summary / candidates / classes 四键，随存档与
  `settlement.json.final_state.kv` 自然可见。

### 8.2 goal 授权（run 级自声明）

```text
config.set goal.authorization=<单行说明>      # trim ≤200、无换行；空值 = 显式清除
```

- 落点 `settings.goal.authorization`；**只有运行期进入 goal**（`settings.play.mode=goal`）的局在 `review.note`
  收口前校验：缺记录则旁白阻断（不写 `ending`、不推进），登记后即可收口；resume 保留；切回 guide 不校验。
- 静态 `data/config.json` 预设 goal 视为仓库所有者常设配置，**不在校验范围**（否则全 1 冒烟会断）；若静态 goal 的局
  中途切过一次模式（`settings.play.mode` 被写入），之后收口就会要求授权——语义是「运行期重新进入 goal 要说清谁授权」。
- 它只证明「登记过一句话」，不是身份认证，也不进 `agent.summary`；配置页会回显摘要或「未登记」。

### 8.3 执行者账本（自声明）

```text
config.set executor.<rN>.<analysis|plan|development>=<agent_id>[|<handoff_reason>]
config.set executor.<rN>.<stage>=            # 空值 = 显式解绑
```

| 情形 | 结果 |
| --- | --- |
| 首次绑定（且与前一阶段 id 相同/无前序） | 写 `settings.executor.<rN>.<stage>`；旁白；**不写 hist** |
| 同一 (req,stage) 同 id 重绑 | 幂等：旁白「执行者未变」，不改值、不写 hist |
| 同一 (req,stage) 换不同 id | **必须带原因**（`=<id>|<原因>`），否则【需求待办】拒绝、不写 KV |
| 跨阶段换人（plan ≠ analysis；development ≠ plan，无 plan 时比 analysis） | **同样必须带原因**；不得以「首次绑定该 stage」静默覆盖 |
| 换人成功（含跨阶段） | 写 `id|原因`，并向 `req.<i>.hist` 追加一条（滚动上限沿用，不另建 history/handoff 键） |
| `amend` | 绑定不变（账本不带 rev） |
| `split` / `merge` | 子需求 / 合并目标**不继承**绑定；父/源终态只作历史可读 |
| `proposal.submit` | **不做硬校验**：外部提案与同 ext 重放照旧；未登记只在 `requirement.list` 显示「未登记」 |

- `requirement.list` 的 trace 会以紧凑 clause 列出各阶段执行者（`〔exec analysis=S1 plan=S1〕`）；当前 `job` 对应阶段
  未登记时显示 `〔exec <stage>=未登记〕`。
- id 形状：trim 非空、≤64、无空白、不含 `|`；`external` 可作为「用户/外部提案人」的约定 token（Mod 不特殊处理）。
- 同样是**自声明**：它只证明主 Agent 登记了什么，不证明宿主身份或真实执行者，不得包装成身份认证。

## 九、文件

| 文件 | 面 | 作用 |
| --- | --- | --- |
| `mod.lua` | 骨架 | **装配入口**：资源快照 → require 22 模块 → install → `render.register()` → `tools.register()` → `fsm.define`（≈115 行） |
| `src/*.lua` | 骨架 | 12 个纯导出模块：util / config / requirements / lanes / proposals / gates / guide / menus / render / wire / tools / flow |
| `src/handlers/*.lua` | 骨架 | 10 个注册模块：actions / lanes / proposals / review / requirement_ops / gate_actions / config / views / choice / lifecycle |
| `data/scenes.json` | 实例 | 文本资源（scenes / endings / prompts / options / beats / tuning） |
| `data/playbook.json` | 实例 | 现场规则资源（`guide.show` 按 section id 取用） |
| `data/config.json` | 实例 | 静态配置默认值 |
| `pack.json` / `manifest.json` | 骨架 + 生成物 | 打包声明（`code` 段属骨架，`agent`/`inboxRoutes` 属实例）与内容寻址清单 |
| `schemas/*.json` | 骨架 | 资源 schema（与 `packages/schema/schemas/` 单源一致） |

骨架面（`mod.lua` + `src/**` + `schemas/` + `pack.json` 的 `code` 段）与
`packages/mods/pi-agent-seed/` 逐字节一致，由 `scripts/sync-seed-mod.sh --from project` 同步；
模块清单、依赖方向与三条硬规则见种子侧 README 的「模块结构」一节。
`src/tools.lua` 是唯一带 `register()` 的 `src/` 模块（与 `render.lua` 同构：纯导出 + 装配期注册），
也是 `tool.*` 的**唯一写方**。
本实例新增的 KV 前缀：`reject.count` / `reject.last`（`flow.refuse` 的拒绝留痕，只读查询用）。
`data/` 与 `pack.json` 的 `agent`/`inboxRoutes` 属实例面，不参与同步。

## 十、工具桥（`tool.call` / `tool.result`）

Mod **永不做 IO**（沙箱无 `io` / `os`，也没有 process / 网络 API）：一个工具调用是**请求**，不是副作用。

```text
① Mod 登记请求：ygg.meta("tool_calls", { { id = "t1", tool = "git.head_short", args = { rev = "HEAD" } } })
② 宿主执行（Layer 1 形状 + Layer 2 放行），把结果作为**普通事件**回投
③ tool.result "t1|git.head_short|ok|0|abc1234|ext-1"
```

- **请求 wire**（`tool.call`）：`<tool_id>[|<k=v;...>][|<call_id>]`（位次固定；只要 call_id 就写 `git.status||t2`）。
  `tool_id` 必须已列在 `allowedTools`；`call_id` 自动分配为 `t1`、`t2`…（也可显式给，保留字
  `count` / `pending` / `last` / `notified` 不可用）。
- **结果 wire**（`tool.result`）：`<call_id>|<tool_id>|<ok|err>|<code>|<output>[|<external_id>]`
  （5 或 6 段；也可用 `|@ext=` 后缀形态）。`output` 内不得含竖线，CR/LF 折成 ` / `，超过
  `tuning.tool_output_max` 截断并附 `…[truncated N chars]`。
- **`meta.tool_calls`**：稠密数组、序 = 分配序、**只列 `status = "pending"` 的行**；无参数时
  **省略** `args` 键（Lua 空表只能序列化成 `[]`）。终态后为 `[]`（不是缺键）。
- **幂等**：请求侧 `@ext=` 已应用 → `[REPLAY:TOOL]`；结果侧同一 `external_id` 重投 → `[REPLAY:TOOL]`，
  不改状态、不减计数、不重出 `[TOOL SETTLED]`。一个 call 只有一个终态（换状态是冲突，不静默覆盖）。
- **KV 面**（`src/tools.lua` 是唯一写方）：`tool.count`（生命周期计数）/ `tool.pending`（未终态计数，
  摘要字段 `pendingTools` 读它）/ `tool.last` / `tool.seq.<i>`（分配序索引，`flush_meta` 的遍历面）/ 
  `tool.<id>.{id,tool,args,status,code,output,ext,res_ext,truncated}` / `tool.notified.<id>`（回执闩锁）/
  `tool.reacted.<id>`（响应式闩锁）。响应式子系统另写 `lane.*` / `req.<i>.conflict*` / `artifact.*` /
  `verify.{passed,report,source}` / `settlement.sessions.*`（见 §十一；逐键登记在 `src/tools.lua` 头注释）。
- **回执 effect**：`hooks.useEffect("tools.receipt")` 只置闩锁 + 一行叙事，不 render、不 choose、不改业务状态；
  因此 `[TOOL SETTLED]` 一定与 `[TOOL RESULT]` 在**同一回合**、且一生只有一次。**领域折叠在另一条 effect**
  （`tools.reactive`，见 §十一）：两条 effect 同处一次 `flush_effects`，但边界不混。
- **两个场景**（宿主侧）：`inlineWaitMs > 0` 且结果及时到位 = **场景 1：同进程内就地回投**（用户可见：
  一步之内看到结果）；`inlineWaitMs = 0` 或超时 = **场景 2：脱体 worker + `--tool-result` inbox 回写**，
  回投走 `tool_result` 这条路由，Mod 侧看到的事件形状**完全相同**。
- **凭据红线**：`pack.json` 的 `args` / `headers` 可写 `$ENV{VAR}`，宿主按
  `进程环境 > <mod_dir>/.env.local > <mod_dir>/.env` 解析；解析后的值**绝不**进载荷、回执或 notice
  （报错只出现变量名）。Mod 资源、配置与报告只出现**键名**。

## 十一、响应式后置效应与主动子系统

`tool.result` 落库为 `ok` + `code 0` 时，`hooks.useEffect("tools.reactive")` 把 `output`（`k=v;k=v` 平坦映射）
折叠成领域状态。它与 `tools.receipt` 同处 `render()` 入口的**同一个 `flush_effects`**（同回合、产出 choice 之前），
但职责严格分层：回执行只说「结果到了」，反应器才改领域 KV。闩锁 `tool.reacted.<call_id>` 保证一生只折叠一次
（重投结果不会第二次抬泳道、第二次抬冲突）。

| 能力 | 触发面 | 写的 KV | 读 / 判 |
| --- | --- | --- | --- |
| `git.worktree_claim` | `tool.result` | `lane.<k>.{req,branch,base,since,clean}` + `req.<i>.lane`（走 `LANES.bind`，与 `lane.claim` **同函数同键**） | `clean=1` / 泳道空闲 / 需求未绑定 / 未终态，任一不满足 → `[WORKTREE CLAIM REFUSED]` 且账本不动 |
| `git.worktree_release` | `tool.result` | 清 `lane.<k>.*` + `req.<i>.lane`（走 `LANES.release_lane(k)`） | 入参是**泳道号**（`LANES.release` 收的是 req index，别混）；`released≠1` 不动作 |
| `git.lane_probe` | `tool.result` | `lane.<k>.probe.{commits,clean,head,files,call}`；有交集时经 `req_do_conflict(...,"radar")` 抬 `req.<i>.conflict*` | `commits=0` / `clean≠1` → `[LANE PROBE FAILED]`（**警告面**，不阻塞收口）；输出被截断 → 跳过雷达 |
| `fs.artifact_probe` | `tool.result` | `artifact.<path>.{bytes,sha256,source}` + `artifact.count` / `artifact.seq.<n>` | `exists≠1` / `bytes=0` / 路径不合形状 → 不落库 |
| `gate.verify_suite` | `tool.result` | `verify.passed` / `verify.report` / `verify.source` | 只在 `review` 阶段、只在 `passed=1`、且 `gate_problem()==nil` 时写绿 |
| `harness.archive_sessions` | `tool.result` | `settlement.sessions.{moved,remaining,destination,source}` | 只在 `review`、只在 `remaining=0` 且 `dest≠""` 时登记 |
| `vfs.write` | 事件 handler | `artifact.<path>.{bytes,sha256,source="vfs"}` | 载荷 = 路径 + 竖线 + 内容（首个竖线前是路径，其后全部是内容；内容可含竖线） |

**三条硬边界**（`data/playbook.json` 的 `tools.reactive-effect-scope` / `subsystems.probe-is-warning` /
`subsystems.machine-evidence-parity`）：

1. **反应器只写 KV + 叙事**，绝不 `render()` / `ygg.choice` / `flow.refuse`——拒绝行会被宿主记成失败步，
   而「探针失败」是事实报告，不是 Mod 拒绝。
2. **探针失败不接门禁**：`[LANE PROBE FAILED]` 只叙事 + 留 `lane.<k>.probe.*` 可查；接进 `gate_problem()` 会让
   一次脏树探针把收口永久卡死（没有「撤销探针」的动作）。**冲突雷达例外**：它走 `req.<i>.conflict`，因此
   自动进 `req_conflict_total` → 阻塞派活与收口，解除只走 `requirement.resolve`。
3. **机证与人证同级不等强**：`gate.verify_suite` / `harness.archive_sessions` 写的是与 `verify.submit` /
   `settlement.sessions` **同一批 KV**，只省一个回合；`verify.source` / `settlement.sessions.source` 保留溯源，
   汇报时不得声称机证比自声明更强。

**`vfs.write` 与 `ygg.fs`**：Mod 仍不做 IO，写盘一律委托宿主中介的 `ygg.fs`（run 相对 + 路径狱）。宿主没装
`ygg.fs` 时**不假装写过**：账本照样记（同一条 `record_artifact`），叙事改成 `[VFS LEDGER ONLY]`，
所以「KV 与叙事」在两种宿主下都言行一致。`artifacts/../x.md`、含空白、超过 128 B、非 `artifacts/*.md`
一律在调 `ygg.fs` **之前**拒绝。

## 十二、走查

```bash
# 默认全 1 贪心：intake → coordinate.submit → verify → settlement.evolution → review.note → 【结局
cargo run --example playtest -- .ygg/mods/pi-agent-seed

# 多需求 multi-stage（外部提案；host 不再生成任何占位提案）：
#   .ygg/bin/ygg-agent --run-id t --runs-root .ygg/runs --requirement '需求A|描述'
#   .ygg/bin/ygg-agent --run-id t --runs-root .ygg/runs --proposal 'r1|1|analysis|结论|a.md'
#   .ygg/bin/ygg-agent --mod .ygg/mods/pi-agent-seed --run-id t --runs-root .ygg/runs --resume

# 改完 data/ 或 schemas/ 必跑（manifest 内容寻址）
cargo run --example pack -- .ygg/mods/pi-agent-seed
```
