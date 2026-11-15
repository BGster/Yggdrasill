# Yggdrasill · 宿主 CLI 与 Daemon JSON-RPC 线协议
> **id** host/cli-daemon · **模块** ② egg CLI 与 host · **触发** 改命令行 / daemon 协议 / 存储与版本兼容面之前
> **读不到代价** hard-refusal（存储后端不匹配 fail-closed，旧局打不开）

> **定位**：egg 单发 CLI 参数、6 个 builtin 工具、daemon 行分隔 JSON-RPC 与 summary/HarnessAction Schema。
> **replaces**：DESIGN §3.5（L509–589）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[action-faces.md](../mod/action-faces.md) · [lanes.md](lanes.md) · [compaction.md](../kernel/compaction.md)
> **搬运**：以下正文自 DESIGN.md L509–L589 逐字迁移，不重写语义。

### 3.5 宿主 CLI 与 Daemon JSON-RPC 线协议契约

> `op` 面的 8 个操作是状态推进之外的旁路端口；`op=vfs_put` 与 `event=vfs.write` 的语义差异（阶段守卫 / 内容上限 / 台账）见 §3.4。

#### 单次 CLI 与内置工具参数（`egg`）
- **运行执行**：新局 `.ygg/bin/egg new --mod <mod_dir> [--in .ygg/runs] [--raw] [--event <event> [--payload <json>]]`（自动 run-id，回执末行 run-root）；续跑 `.ygg/bin/egg open <run-root> [--at g<代>] [--raw] --event <event> [--payload <json>]`
  - **回执渲染口径（r5）**：缺省为人读文本渲染；`--raw` = 单行 JSON 机器契约（与历史缺省逐字节一致）；解析链 `--raw` > `YGG_FORMAT`（text\|json）> 缺省 text；不改字段契约、退出码与 daemon 协议面。脚本 / CI 解析单发 stdout 必须加 `--raw`（或环境级 `export YGG_FORMAT=json`）。
- **单发派发的未注册事件（fail-closed，r11）**：`--event <name>`（或省略时的 `entryEvent`）不在 Mod registry 内时，派发在**零副作用**下拒绝（不进回放日志、不耗燃料、不动状态基线）：回执 `ok:false` + `error.code=VALIDATION`，`message` 三要素（事件名原样 + 已注册计数 + 已注册全集字典序），退出码 1，审计行 `ok=false` 照写。`ygg.*` 保留事件同样只认 registry（宿主不持事件名白名单）；空/空白事件名仍走参数面（usage / DAEMON 层），与本语义分层。
- **结算**：`.ygg/bin/egg open <run-root> --settle`
- **存储选择与版本硬门**：默认 build 启用 `store-sqlite`（`crates/ygg-sandbox/Cargo.toml` `default=["store-sqlite"]`），新局默认 `<run-dir>/run.sqlite`。带 `autosave.json` / `events.jsonl` 的旧局自动检测并留在 JSON 后端，不迁移；`--store json` 仅为新局显式回退。`run.json` 的 `store` 一旦写入即权威，`--store` 只是开局断言，不匹配 fail-closed 且零写入。host 与 Mod 版本不匹配时装载立即失败，错误点名缺失字段与期望形状，绝不静默降级。`.ygg/bin/*`、`.ygg/runs/*` git-ignore，`.ygg/mods/` 入仓。
- **内置宿主工具（`--builtin-tool <op>`）**：
  - `worktree-claim --lane <1..4> --req <rN> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;req=<rN>;branch=<br>;base=<sha>;clean=1;path=<wt_path>`（r1 分池：`--repo` 指向非运行仓的本地检出时，目录落位 `<pool-root>/<检出basename>-lane-<k>` —— Mod 侧经 `git.worktree_claim_pool` 池化工具声明携带该参）
  - `lane-probe --lane <1..4> --base <sha> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;commits=<N>;clean=<0|1>;head=<sha>;files=<csv>`（同上，`git.lane_probe_pool` 携带 `--repo`）
  - `worktree-release --lane <1..4> [--repo <path>]` $\to$ `lane=<k>;released=1;idle=<0|1>`（同上，`git.worktree_release_pool` 携带 `--repo`）
  - `artifact-probe --path artifacts/<name>.md [--run-dir <dir>]` $\to$ `path=<p>;exists=<0|1>;bytes=<N>;sha256=<hex>`
  - `verify-suite [--mode fast|full] [--repo <path>]` $\to$ `passed=1;summary=<M>/<M>`（在回退到默认 Rust cargo 门禁前，尊重 `YGG_VERIFY_COMMANDS="cmd1 args;cmd2 args"` 环境变量覆盖）。
  - `archive-sessions --parent <id> --batch <run_id>` $\to$ `moved=<N>;remaining=0;dest=<path>`（r5/子会话清理：宿主 builtin 本体保留，但 **Mod 工具桥合同已摘除**——`harness.archive_sessions` 不再在 `pack.json`/`allowedTools` 声明，Mod 不再发起；子会话清理改提示词驱动 + `settlement.sessions` 声明制；builtin 物理退役归 N2 宿主决策）

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
  "play": "guide",
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
        "brief": "Requirement r1 (rev 1, stage=development): ...\nMandatory references: read docs/README.md, docs/context/acts/coordinate.md, docs/context/rituals/dispatch.md, and the requirement's named docs/design/<subsystem>.md before proposing or editing. Plan: follow playbook run_machine + actions and declare touched wiki files. Development: also follow gates + live_evidence and the touched subsystem chapter.\n...",
        "deltaPrompt": "Resume r1 (rev 1) for stage `development` in worktree `...`; write `artifacts/r1-development.md` and submit.",
        "fullFallbackBrief": "Requirement r1 ...\nResume context: prior executor `agent-plan-1`.",
        "submitCommand": ".ygg/bin/egg open <run-root> --event proposal.submit --payload '\"r1|1|development|<summary>|artifacts/r1-development.md||exec=<agent_id>\"'"
      }
    }
  ]
}
```

> ~~r1 自动收波看门狗契约（自动收波（已废弃））~~ 已废弃：扩展层删除（2026-11-12），看门狗随层移除；daemon 常驻由宿主 `--daemon`/`serve` 原生承担。

#### 收件箱（inbox）与跨进程协调协议

**文件**：`<runs-root>/<run-id>/inbox.jsonl`，一行一个对象 `{external_id, kind, title, description, payload, created_at}`，`kind ∈ add/amend/split/merge/conflict/resolve/proposal`。

**异步命令表**（只追加、不读写 autosave，因此不需要 `--mod`，可与活跃局并发执行）：

- `--requirement 'title|description'`（add）
- `--proposal 'rN|rev|stage|summary|artifacts[|changes]'`（proposal；external_id 自动作为第 7 段追加）
- `--amend 'rN|new title|new description|deps'`（rev+1）
- `--split 'rN|child A|child B'`
- `--merge 'r2,r3|title|description'`
- `--conflict 'r1,r2|note'`
- `--resolve 'r1,r2|keep_both'`

**消费点**：`cmd` 下每次自动推进前、`stdin` 下每次提示前、每次 single-shot dispatch 前——新行按 kind 排空并派发；**在局活跃期间到达的行永不可漏**。

**跨进程互斥（协议级，不是「再看一眼文件」）**：

- `coordination.lock`（`create_new` 排他；陈旧锁按 mtime 2s 接管）＋ `coordination.json` `{phase, writers, owner_pid, heartbeat_at}`，`phase ∈ running / closing / finalized`。
- **写命令先注册**：`running` → 在它的行落盘前算一个 writer；`closing`（心跳未过期）→ 命令自身失败退出 3；`finalized` → 非零拒绝（退出 3，需要新局）。
- **closing 协议**：自动推进在派发最终 `review.note` 前把 phase 置 `closing`，等在途 writer 归零，再在同一保护下做最后一次 drain——若有新的未 done 需求则撤销 closing 继续循环（不写 ending）；若没有新行则在同一保护下完成 `review.note` + settle 并置 `finalized`。因此「写命令成功了但局已越过最后一次 drain」在协议上不可能发生。
- **陈旧与崩溃恢复**：closing 用心跳（10s 过期）自保；过期后写命令或 `open`（attach）宣布它死亡并把 phase 拉回 `running`；锁同样按 mtime 接管——两者都不会因进程崩溃永久卡死。
- **`--settle` 在 closing 下同样走协议**：有未消费行时返回 `ok=false` 并回退 `running`（resume 可消费）；已经 `finalized` 时幂等返回旧报告，不改写任何东西。

**幂等与游标**：游标（`inbox.cursor` 的 `{consumed:N}`）只在派发成功且存档落盘之后推进；重放靠 `external_id`——add / proposal 由 Mod 按实体 ext 去重（`[REPLAY:REQ]` / `[REPLAY:PROPOSAL]`），amend / split / merge / conflict / resolve 用载荷尾部 `@ext=` 加 `inbox.last` 去重（`[REPLAY:ASYNC]`）。

**坏行与结算闸**：畸形行跳过并让游标越过它（否则会永久阻塞）。**硬结算闸**：`--settle` 发现未消费行即返回 `ok=false` + 非零退出（写完后会复查：结算期间到达的行会使报告作废）——「ending 之后才投递」永不会静默成功，必须开新局。进程退出之后的投递属于**新局**：inbox 保留这些行给下一局消费，结算报错。

#### run 目录文件契约

- `run.json` = `{run_id, mod, mod_signature, started_at, updated_at}`。
- `autosave.json`：逐步自动存档（实体表 + KV + 待决选择），可直接喂给 `load`。
- `events.jsonl`：每次 dispatch 一行 `{seq, event, payload, ok, narrative_count, timestamp, ending, ending_line?}`。
- `inbox.jsonl`：异步操作队列（append-only，由运行中的局消费；协议见上一节）。
- `coordination.json`：跨进程协议 `{phase, writers, owner_pid, heartbeat_at}`（见上一节）。
- `settlement.json`：只在 `--settle` 时写；`ok` = 事件日志非空 ∧ 无失败步 ∧ 出现 ending 叙述行 ∧ inbox 无未消费行（开关来自 Mod 契约的 `settlement` 段）；退出码镜像 `ok`；有未消费行时报告写「ending 之后还有 N 条未消费 inbox 行——需要新局」且 `ok=false`。
- `decide-input.json`：外部决策命令的输入——`--drive cmd:` 下，每次出现待决选择、在调用命令之前，把完整当前回执写进去。

**回执面（不落盘）**：

- summary 走内核的结构化回执通道 `ygg.meta("summary", {…})`，落在回执信封的 `meta.summary`（stage / ending / verifyPassed / nextEndsRun / pendingProposals / requirementCount / conflicts / awaiting / evolution{artifact,summary,candidates,classes}）；**宿主只读这个字段**，所以把同样内容写进 narrative 不会被误读；载荷 `"0"` 表示只读。
- **宿主永不读 Mod 私有 KV**：Mod 没有 `agent.summary` 时，宿主报「unknown」并安全暂停，不回落到 `ending` / `fsm.stage` / `prop.*` / `req.*`。
- **`meta` 是瞬时回执面**：不进存档、不进规范快照、不进 state hash——同一事件序列调用与否 `ygg.meta`，`state_hash` 相同。

