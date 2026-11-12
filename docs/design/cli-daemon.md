# Yggdrasill · 宿主 CLI 与 Daemon JSON-RPC 线协议

> **定位**：egg 单发 CLI 参数、6 个 builtin 工具、daemon 行分隔 JSON-RPC 与 summary/HarnessAction Schema。
> **replaces**：DESIGN §3.5（L509–589）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[action-faces.md](action-faces.md) · [lanes.md](lanes.md) · [compaction.md](compaction.md)
> **搬运**：以下正文自 DESIGN.md L509–L589 逐字迁移，不重写语义。

### 3.5 宿主 CLI 与 Daemon JSON-RPC 线协议契约

> `op` 面的 8 个操作是状态推进之外的旁路端口；`op=vfs_put` 与 `event=vfs.write` 的语义差异（阶段守卫 / 内容上限 / 台账）见 §3.4。

#### 单次 CLI 与内置工具参数（`egg`）
- **运行执行**：`.ygg/bin/egg --mod <mod_dir> --run-id <id> --runs-root .ygg/runs [--resume] [--raw] --event <event> [--payload <json>]`
  - **回执渲染口径（r5）**：缺省为人读文本渲染；`--raw` = 单行 JSON 机器契约（与历史缺省逐字节一致）；解析链 `--raw` > `YGG_FORMAT`（text\|json）> 缺省 text；不改字段契约、退出码与 daemon 协议面。脚本 / CI 解析单发 stdout 必须加 `--raw`（或环境级 `export YGG_FORMAT=json`）。
- **单发派发的未注册事件（fail-closed，r11）**：`--event <name>`（或省略时的 `entryEvent`）不在 Mod registry 内时，派发在**零副作用**下拒绝（不进回放日志、不耗燃料、不动状态基线）：回执 `ok:false` + `error.code=VALIDATION`，`message` 三要素（事件名原样 + 已注册计数 + 已注册全集字典序），退出码 1，审计行 `ok=false` 照写。`ygg.*` 保留事件同样只认 registry（宿主不持事件名白名单）；空/空白事件名仍走参数面（usage / DAEMON 层），与本语义分层。
- **结算**：`.ygg/bin/egg --mod <mod_dir> --run-id <id> --runs-root .ygg/runs --settle`
- **内置宿主工具（`--builtin-tool <op>`）**：
  - `worktree-claim --lane <1..4> --req <rN> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;req=<rN>;branch=<br>;base=<sha>;clean=1;path=<wt_path>`（r1 分池：`--repo` 指向非运行仓的本地检出时，目录落位 `<pool-root>/<检出basename>-lane-<k>` —— Mod 侧经 `git.worktree_claim_pool` 池化工具声明携带该参）
  - `lane-probe --lane <1..4> --base <sha> --branch <branch> [--repo <path>]` $\to$ `lane=<k>;commits=<N>;clean=<0|1>;head=<sha>;files=<csv>`（同上，`git.lane_probe_pool` 携带 `--repo`）
  - `worktree-release --lane <1..4> [--repo <path>]` $\to$ `lane=<k>;released=1;idle=<0|1>`（同上，`git.worktree_release_pool` 携带 `--repo`）
  - `artifact-probe --path artifacts/<name>.md [--run-dir <dir>]` $\to$ `path=<p>;exists=<0|1>;bytes=<N>;sha256=<hex>`
  - `verify-suite [--mode fast|full] [--repo <path>]` $\to$ `passed=1;summary=<M>/<M>`（在回退到默认 Rust cargo 门禁前，尊重 `YGG_VERIFY_COMMANDS="cmd1 args;cmd2 args"` 环境变量覆盖）。
  - `archive-sessions --parent <id> --batch <run_id>` $\to$ `moved=<N>;remaining=0;dest=<path>`（r5/D-MA21：宿主 builtin 本体保留，但 **Mod 工具桥合同已摘除**——`harness.archive_sessions` 不再在 `pack.json`/`allowedTools` 声明，Mod 不再发起；子会话清理改提示词驱动 + `settlement.sessions` 声明制；builtin 物理退役归 N2 宿主决策）

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
        "brief": "Requirement r1 (rev 1, stage=development): ...\nMandatory references: read docs/README.md (wiki index) first, then docs/context/* (domain invariants) and docs/design/* (architecture & contracts) before proposing or editing.\n...",
        "deltaPrompt": "Resume r1 (rev 1) for stage `development` in worktree `...`; write `artifacts/r1-development.md` and submit.",
        "fullFallbackBrief": "Requirement r1 ...\nResume context: prior executor `agent-plan-1`.",
        "submitCommand": ".ygg/bin/egg --mod .ygg/mods/agent-seed --runs-root .ygg/runs --resume --event proposal.submit --payload '\"r1|1|development|<summary>|artifacts/r1-development.md||exec=<agent_id>\"'"
      }
    }
  ]
}
```

> ~~r1 自动收波看门狗契约（D-MA16）~~ 已废弃：扩展层删除（2026-11-12），看门狗随层移除；daemon 常驻由宿主 `--daemon`/`serve` 原生承担。

