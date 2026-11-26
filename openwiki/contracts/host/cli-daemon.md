---
type: contract
title: CLI 与常驻契约
description: egg 单发 CLI 参数、六个内置工具、daemon 行分隔 JSON-RPC、结构化 summary、inbox 跨进程协议、run 目录文件契约与 drive 决策协议。
tags: [host, cli, daemon, json-rpc, inbox, contract]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-4c2ab33dcfd357289041ac6f
    resource: repo://crates/ygg-sandbox/Cargo.toml
  - id: openwiki-source-5f49e9ecfdbd1d2e19a9b486
    resource: repo://crates/ygg-sandbox/examples/egg/args.rs
  - id: openwiki-source-791e5c16ac1b66a7c62d133f
    resource: repo://crates/ygg-sandbox/examples/egg/coordination.rs
  - id: openwiki-source-d303de7a370ffcc3b94e2fda
    resource: repo://crates/ygg-sandbox/examples/egg/daemon.rs
  - id: openwiki-source-4e7467d3ca65af187906d262
    resource: repo://crates/ygg-sandbox/examples/egg/drive.rs
  - id: openwiki-source-c42f7c15522e42a336979612
    resource: repo://crates/ygg-sandbox/examples/egg/single_shot.rs
  - id: openwiki-source-803df0137779c6ce0b350f7e
    resource: repo://crates/ygg-store/src/lib.rs
  - id: openwiki-source-3d69fe5377a4bdc3315a6fb1
    resource: repo://crates/ygg-store/src/mounts/store.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# CLI 与常驻契约

> **id** host/cli-daemon · **模块** ② egg CLI 与 host · **触发** 改命令行 / daemon 协议 / 存储与版本兼容面之前
> **读不到代价** hard-refusal（存储后端不匹配 fail-closed，旧局打不开）
> **replaces**：DESIGN §3.5（L509–589）（搬运基线 3a2536f）

**定位**：egg 单发 CLI 参数、六个 builtin 工具、daemon 行分隔 JSON-RPC 与 summary / HarnessAction Schema。
**上游**：[../../overview.md](../../overview.md) · **相关**：[../mod/action-faces.md](../mod/action-faces.md) · [lanes.md](lanes.md) · [../kernel/compaction.md](../kernel/compaction.md)

## 1. 单次 CLI 与内置工具参数

- **运行执行**：新局 `egg new --mod <mod_dir> [--in .ygg/runs] [--packages <目录>] [--raw] [--event <event> [--payload <json>]]`（自动 run-id，回执末行 `run-root`）；续跑 `egg open <run-root> [--at g<代>] [--packages <目录>] [--raw] --event <event> [--payload <json>]`。
  - **回执渲染口径**：缺省人读文本；`--raw` = 单行 JSON 机器契约（与历史缺省逐字节一致）；解析链 `--raw` > `YGG_FORMAT`（text\|json）> 缺省 text。不改字段契约、退出码与 daemon 协议面。脚本 / CI 解析单发 stdout 必须加 `--raw`（或 `export YGG_FORMAT=json`）。
- **未注册事件（fail-closed）**：`--event <name>`（或省略时的 `entryEvent`）不在 Mod registry 内时，派发在**零副作用**下拒绝（不进回放日志、不动状态基线）：回执 `ok:false` + `error.code=VALIDATION`，`message` 三要素（事件名原样 + 已注册计数 + 已注册全集字典序），退出码 1，审计行 `ok=false` 照写。`ygg.*` 保留事件同样只认 registry；空 / 空白事件名走参数面（usage / DAEMON 层）。
- **能力包根显式覆盖**：`--packages <目录>` 把能力包根显式交给装载链；优先级 `explicit > $YGG_PACKAGES_ROOT > <mod>/../../packages/libs` 布局推导，语义零新增。仅 new / open 装载路径消费；其余模式接受但忽略。
- **激活触点**：开局与热载两个内核构造点把既有库面装配进 CLI——① fs 桥：读 `pack.json` 的 `mounts` 段 → `MountStore::attach`（挂载点落 `<run>/mounts/`、审计落 `fs.jsonl`）→ `set_mount_fs`，`ygg.fs.*` 的 `mounts/` 面在真 CLI 可用（不再 `YGG-FS-UNMOUNTED`）；② sim：`Sim::new(固定常量种子)` → `set_sim`，`ygg.sim.*` Tier 1 面在真 CLI 激活。失败语义：开局 attach 失败 = 装载失败（fail-closed 退出）；热载失败沿用旧内核（不分裂半更新态）。不声明 mounts 的 mod 零行为差异。
- **结算**：`egg open <run-root> --settle`。
- **燃料预算**：原 r15 机制（转换预算自动扩缩 + `YGG_FUEL_BUDGET` 逃生门）已随燃料机制整体退役删除；`YGG_FUEL_BUDGET` 现为 no-op，裁定见 [../kernel/lua-api.md](../kernel/lua-api.md) §3.1.1。

### 1.1 存储选择与版本硬门

- 默认 build 启用 `store-sqlite`（`crates/ygg-sandbox/Cargo.toml` 的 `default = ["store-sqlite"]`），新局默认 `<run-dir>/run.sqlite`。
- 带 `autosave.json` / `events.jsonl` 的旧局**自动检测并留在 JSON 后端，不迁移**；`--store json` 仅为新局显式回退。
- `run.json` 的 `store` 一旦写入即权威；`--store` 只是开局断言，不匹配 **fail-closed 且零写入**。
- host 与 Mod 版本不匹配时装载立即失败，错误点名缺失字段与期望形状，**绝不静默降级**。
- `.ygg/bin/*`、`.ygg/runs/*` git-ignore；`.ygg/mods/` 入仓。

### 1.2 内置宿主工具（`--builtin-tool <op>`）

| op | 输出 |
| --- | --- |
| `worktree-claim --lane <1..4> --req <rN> --branch <br> [--repo <path>]` | `lane=<k>;req=<rN>;branch=<br>;base=<sha>;clean=1;path=<wt_path>` |
| `lane-probe --lane <1..4> --base <sha> --branch <br> [--repo <path>]` | `lane=<k>;commits=<N>;clean=<0\|1>;head=<sha>;files=<csv>` |
| `worktree-release --lane <1..4> [--repo <path>]` | `lane=<k>;released=1;idle=<0\|1>` |
| `artifact-probe --path artifacts/<name>.md [--run-dir <dir>]` | `path=<p>;exists=<0\|1>;bytes=<N>;sha256=<hex>` |
| `verify-suite [--mode fast\|full] [--repo <path>]` | `passed=1;summary=<M>/<M>`（回退到默认 cargo 门禁前尊重 `YGG_VERIFY_COMMANDS="cmd1 args;cmd2 args"`） |
| `archive-sessions --parent <id> --batch <run_id>` | `moved=<N>;remaining=0;dest=<path>`（宿主 builtin 本体保留，但 Mod 工具桥合同已摘除：`harness.archive_sessions` 不再声明，子会话清理改提示词驱动 + `settlement.sessions` 声明制） |

`--repo` 指向非运行仓的本地检出时，池化目录落位 `<pool-root>/<检出 basename>-lane-<k>`（Mod 侧经 `git.worktree_claim_pool` / `git.lane_probe_pool` / `git.worktree_release_pool` 携带该参）。

## 2. Daemon 模式（`--daemon`）行分隔 JSON-RPC

每个请求是 `stdin` 上一行 UTF-8 JSON；每个响应是 `stdout` 上一行 UTF-8 JSON：

- **请求**：`{ "id": <number|string>, "op": "<ping|summary|dispatch|vfs_put|vfs_get|vfs_list|settle|shutdown>", ...params }`
- **响应**：`{ "id": …, "ok": true, "result": <object> }` 或 `{ "id": …, "ok": false, "error": { "code": "<DAEMON|VALIDATION|…>", "message": "<text>" } }`（`error` 是**对象形**；`dispatch` 的失败应答是步回执形——附 `op:"dispatch"` 与 `summary`，错误码随内核信封；`op` 面自身错误用 `DAEMON`）
- **提交者身份**：`op=dispatch` 请求可带 `"actor"` / `"actor_session"`，仅在旗标（`--actor` / `--actor-session`）与环境变量（`YGG_ACTOR` / `YGG_ACTOR_SESSION`）都缺席时兜底；形状非法 → 可读错误应答。派发回执恒回显 `actor` / `actor_session`（None → null，加性键）；inbox 写命令回执另回显 `opener`。

`op` 面的 8 个操作是状态推进之外的旁路端口；`op=vfs_put` 与 `event=vfs.write` 的语义差异（阶段守卫 / 内容上限 / 台账）见 [action-faces.md](../mod/action-faces.md)。

## 3. 结构化 summary 与 `HarnessAction` Schema

`result.summary` 的稳定字段：`stage` / `ending` / `verifyPassed` / `nextEndsRun` / `lifecycle` / `batch` / `batchDrained` / `play` / `pendingProposals` / `requirementCount` / `pendingTools` / `conflicts` / `gates` / `profiles[]` / `jobs[]`。

`jobs[]` 每项至少：`id`(rN)、`rev`、`stage`、`status`、`phase`、`lane`、`branch`、`base`、`worktreePath`、`worktreeStatus`、`subagentStatus`、`probeStatus`、`executor`、`planExecutor`、`title`、`description`，以及 `subagentAction`：

```json
{ "action": "subagent.resume", "reqId": "r1", "rev": 1, "stage": "development", "lane": 1,
  "resumeAgentId": "agent-plan-1", "worktreePath": "…", "artifactPath": "artifacts/r1-development.md",
  "brief": "…", "deltaPrompt": "…", "fullFallbackBrief": "…",
  "submitCommand": ".ygg/bin/egg open <run-root> --event proposal.submit --payload '\"r1|1|development|<summary>|artifacts/r1-development.md||exec=<agent_id>\"'" }
```

- **宿主永不读 Mod 私有 KV**：Mod 没有 `agent.summary` 时宿主报 `unknown` 并安全暂停，不回落到 `ending` / `fsm.stage` / `prop.*` / `req.*`。
- **`meta` 是瞬时回执面**：不进存档、不进规范快照、不进 state hash——同一事件序列调用与否 `ygg.meta`，`state_hash` 相同。
- 自动收波看门狗契约已废弃（扩展层 2026-11-12 删除）；daemon 常驻由宿主 `--daemon` / `serve` 原生承担。

## 4. 收件箱（inbox）与跨进程协调协议

**文件**：`<runs-root>/<run-id>/inbox.jsonl`，一行一个对象 `{external_id, kind, title, description, payload, created_at}`，`kind ∈ add/amend/split/merge/conflict/resolve/proposal`。

**异步命令表**（只追加、不读写 autosave，因此不需要 `--mod`，可与活跃局并发）：

| 命令 | kind |
| --- | --- |
| `--requirement 'title\|description'` | add |
| `--proposal 'rN\|rev\|stage\|summary\|artifacts[\|changes]'` | proposal（external_id 自动作第 7 段追加） |
| `--amend 'rN\|new title\|new description\|deps'` | amend（rev+1） |
| `--split 'rN\|child A\|child B'` | split |
| `--merge 'r2,r3\|title\|description'` | merge |
| `--conflict 'r1,r2\|note'` | conflict |
| `--resolve 'r1,r2\|keep_both'` | resolve |

**消费点**：`cmd` 下每次自动推进前、`stdin` 下每次提示前、每次 single-shot dispatch 前——新行按 kind 排空并派发；**在局活跃期间到达的行永不可漏**。

**跨进程互斥（协议级，不是「再看一眼文件」）**：

- `coordination.lock`（`create_new` 排他；陈旧锁按 mtime 2s 接管）＋ `coordination.json` `{phase, writers, owner_pid, heartbeat_at}`，`phase ∈ running / closing / finalized`。
- **写命令先注册**：`running` → 在它的行落盘前算一个 writer；`closing`（心跳未过期）→ 命令自身失败退出 3；`finalized` → 非零拒绝（退出 3，需要新局）。
- **closing 协议**：自动推进在派发最终 `review.note` 前把 phase 置 `closing`，等在途 writer 归零，再在同一保护下做最后一次 drain——有新的未 done 需求则撤销 closing 继续循环（不写 ending）；没有新行则在同一保护下完成 `review.note` + settle 并置 `finalized`。因此「写命令成功了但局已越过最后一次 drain」在协议上不可能发生。
- **陈旧与崩溃恢复**：closing 用心跳（10s 过期）自保；过期后写命令或 `open`（attach）宣布它死亡并把 phase 拉回 `running`；锁同样按 mtime 接管——两者都不会因进程崩溃永久卡死。
- **`--settle` 在 closing 下同样走协议**：有未消费行时返回 `ok=false` 并回退 `running`；已经 `finalized` 时幂等返回旧报告，不改写任何东西。

**幂等与游标**：游标（`inbox.cursor` 的 `{consumed:N}`）只在派发成功且存档落盘之后推进；重放靠 `external_id`——add / proposal 由 Mod 按实体 ext 去重（`[REPLAY:REQ]` / `[REPLAY:PROPOSAL]`），amend / split / merge / conflict / resolve 用载荷尾部 `@ext=` 加 `inbox.last` 去重（`[REPLAY:ASYNC]`）。

**坏行与结算闸**：畸形行跳过并让游标越过它。**硬结算闸**：`--settle` 发现未消费行即返回 `ok=false` + 非零退出（写完后复查：结算期间到达的行会使报告作废）——「ending 之后才投递」永不会静默成功，必须开新局。

## 5. run 目录文件契约

| 文件 | 内容 |
| --- | --- |
| `run.json` | `{run_id, mod, mod_signature, started_at, updated_at}` |
| `autosave.json` | 逐步自动存档（实体表 + KV + 待决选择），可直接喂给 `load` |
| `events.jsonl` | 每次 dispatch 一行 `{seq, event, payload, ok, narrative_count, timestamp, ending, ending_line?}` |
| `inbox.jsonl` | 异步操作队列（append-only，由运行中的局消费） |
| `coordination.json` | `{phase, writers, owner_pid, heartbeat_at}` |
| `settlement.json` | 只在 `--settle` 时写；`ok` = 事件日志非空 ∧ 无失败步 ∧ 出现 ending 叙述行 ∧ inbox 无未消费行；退出码镜像 `ok`；有未消费行时报告写「ending 之后还有 N 条未消费 inbox 行——需要新局」且 `ok=false` |
| `decide-input.json` | 外部决策命令的输入：`--drive cmd:` 下每次出现待决选择、调用命令之前写入完整当前回执 |
| `mounts/` | `pack.json` `mounts` 段声明的挂载点根；`<run-dir>/mounts/<name>` 恒存在，私有挂载是真目录，`{name, dir}` 挂载是指向 `canonicalize(dir)` 的符号链接（bind 语义，不拷贝内容；`du` / 备份 / 归档默认会跳进业务目录，是声明过的运维副作用）。三态幂等（不存在→建；一致→静默；不一致→ `YGG-FS-MOUNT` 硬拒且零写入）由 `MountStore::attach` 承担 |
| `fs.jsonl` | `ygg.fs.write` 审计行（append-only、懒创建）；**不进 `events.jsonl`**——后者一行 = 一次 dispatch，`seq` 由提交临界区唯一分配 |

## 6. 驱动模式（`--drive`）与决策协议

| 模式 | 行为 |
| --- | --- |
| `stdin`（缺省） | 每一步、每次提案受理与每道门禁都从 stdin 读选择 |
| `cmd:<path>` | **不读 stdin**：每个待决选择交给决策命令；回答 `HUMAN` 则安全暂停（exit 0）；局在 ending 自动结算 |
| `none` | 只打印当前回执（含 `pendingChoice`）就退出，不推进、不读 stdin（exit 0） |

**决策命令协议**：命令的唯一参数是 `decide-input.json` 的路径；stdout **只允许一行**——`N`（1..=选项数）或 `HUMAN[:reason]`。**5 秒超时、非零退出、非法输出、越界索引全部 fail-closed**（安全暂停、状态不变）；`YGG_DECIDE_CMD` 可临时覆盖决策命令路径。

- **碰撞必须暂停**：`cmd:` 遇到冲突时打印双方与 note，然后 exit 0 不做选择；解决后重跑，或切 `stdin` 从 `pendingChoice` 选 1/2/3（keep_both / keep_first / revise）。
- **人类动作门**（`play.mode=guide` 时生效）：`proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note` / `config.set play.mode` **只能**经 `ygg.choice_selected` 进入；直派的 single-shot `--event` 以 `[REJECTED:ACTION]` 拒绝（信封仍 `ok:true`，主状态不变）。`gate.reject` / `proposal.reject` 不受限。
- **绝不伪造内容**：有任务在跑但无待决提案时，宿主**不生成占位提案**——打印可读理由、安全暂停并 exit 0。
- **失控护栏**：阈值来自 Mod `data/config.json` 的 `drive` 段（字段缺失或非法时逐个回退）：硬步数上限 = `maxSteps + stepsPerRequirement × 需求数`（缺省 64 + 16×n）；另有「连续 `stallLimit` 步 `pendingChoice` 不变」停止条件（缺省 12）。
- **`play.mode` 是 Mod 叙述语义，宿主不解释**：`config.set play.mode=<goal|guide>` 只切换 Mod 的提示词与叙述语气，推进只由 `--drive` 驱动——宿主不读 `data/config.json` 的 `play.mode`，也不读存档 KV 的 `settings.play.mode`。
- **goal 模式的判者是 agent 本身**：goal 下 Mod 只改措辞与语气，推进仍经 `ygg.choice_selected`，每一步由 agent 依叙述与候选自行判断——机械取第一项等于伪造结论。仓内不提供默认决策脚本；菜单第一项按设计是主推进动作，但**不保证在当步正确**。

## 7. 维护与检查

- 实现面：`crates/ygg-sandbox/examples/egg/{args,main,single_shot,daemon,coordination,drive,tool_protocol}.rs`；存储后端在 `crates/ygg-store/src/{json,sqlite}`。
- 改命令行 / daemon / 存储面后必须复跑 `cargo test --example egg -p ygg-sandbox` 与 `cargo test -p ygg-store --features sqlite`（CI 门禁见 [../host/harness-tools.md](harness-tools.md)）。
