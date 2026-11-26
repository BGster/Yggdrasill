---
type: "Reference"
title: "三层动作面契约：event / op / action"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-cfe8ec9e3ed7fc4dc881025e
    resource: repo://.ygg/mods/agent-seed/mod.lua
  - id: openwiki-source-9706fc11c20facdfc630a8e9
    resource: repo://.ygg/mods/agent-seed/src/domain/readset.lua
  - id: openwiki-source-55f7cfd3b64c4982c69dd5df
    resource: repo://.ygg/mods/agent-seed/src/domain/wire/intake.lua
  - id: openwiki-source-1687a5fc04658c13a78d4590
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/gate_actions.lua
  - id: openwiki-source-1a8a660d43047137a9cf72f5
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/proposals.lua
  - id: openwiki-source-4dcd58388243470923e255aa
    resource: repo://.ygg/mods/agent-seed/src/handlers/requirements/requirement_ops.lua
  - id: openwiki-source-22b30a4cbdd3f021cc6c2529
    resource: repo://.ygg/mods/agent-seed/src/runtime/flow.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 三层动作面契约：event / op / action

> **id** mod/action-faces · **模块** ③ agent-seed mod · **触发** 新增或移动一个动作面之前
> **读不到代价** silent-drift（把 `op` 当 `event` 用 → 状态旁路）
> **replaces**：DESIGN §3.4（L446–508）+ playbook `actions`（搬运基线 `3a2536f`）
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[../kernel/events.md](../kernel/events.md) · [../host/cli-daemon.md](../host/cli-daemon.md) · [../host/harness-tools.md](../host/harness-tools.md)

## 1. 三层与一句话边界

| 层 | 载体 | 值域 | 权威来源 | 职责 |
| --- | --- | --- | --- | --- |
| **L1 `event`** | `ygg.on(name, handler)` 注册的 handler（`stage.advance` 由 `ygg.fsm.define` 转手注册） | **39 个事件名**（§5） | Mod（`src/{core,domain,view,runtime,handlers}` + `mod.lua`） | **唯一的状态推进货币**：`(state, event) -> state'`，全有或全无。事件之外不存在任何改状态的路径。 |
| **L2 `op`** | daemon JSON-RPC 的 `op` 字段 | `ping` / `summary` / `dispatch` / `vfs_put` / `vfs_get` / `vfs_list` / `settle` / `shutdown`（8） | Rust 宿主（`crates/ygg-sandbox/examples/egg/`） | **状态推进之外的旁路能力**：制品读写、摘要查询、结算报告、进程控制。 |
| **L3 `action`** | `ygg_session` 工具的 `action` 参数 | 9 个（随适配器可变） | 各 harness 适配器（本仓库无内置） | **面向模型/人的语义入口**：决定用哪个动词、由谁拼 payload，**不决定状态**。 |

**`event` 决定状态能不能变；`op` 决定旁路能力能不能用；`action` 只决定模型怎么用一句话说出它想干什么。** 因此 `action` 的值域永远可以变，`event` 与 `op` 的值域基本不能变——重叠发生在哪一层，就只在哪一层可能是冗余。

## 2. 三条硬不变量

1. **I1 · 推进唯一性**：任何 `action` 都**不得**绕过 `op=dispatch` → Mod handler 改状态。适配器里没有第二套状态机，也没有第二份状态投影。
2. **I2 · 状态不可写性**：适配器**绝不**直接写 `run.sqlite` / `settings.*` / `req.*` / `stage`。`stage` 由 `ygg.fsm` 独占写；其余领域键只由 Mod handler 写（`ygg.kv.set` 的领域键全部落在 `src/handlers/**`）。
3. **I3 · 拒绝不可洗白**：回执一旦带 `agent.rejectionPrefixes` 命中的行（`[REJECTED:*]` / `[BLOCKED:*]`），适配器必须**原样回贴**并停止该步后续编排，绝不复述为成功，也绝不重试到看起来通过。

## 3. `action` 的四类落点

| 类别 | `action` | 落点 | dispatch 次数 |
| --- | --- | --- | --- |
| **A1 · 多步编排** | `kickoff`、`settle_close` | 多个 `event`（`settle_close` 另含 `op=vfs_put` + `op=settle`） | ≥ 2，**任一步被拒即停** |
| **A2 · 单 event 别名** | `approve_gate` → `gate.approve`、`reject_gate` → `gate.reject` | 单个 `event` | 1 |
| **A3 · 事件直通** | `dispatch` | 任意 `event`，payload 由调用方给 | 1 |
| **A4 · 非 event 旁路** | `vfs_write` / `vfs_read` / `vfs_list` / `handoff_compact` | `op=vfs_put|vfs_get|vfs_list`；`handoff_compact` = 纯适配器本地 | 0 |

A1 的正确性依赖适配器对 Mod 内部语义的理解（`kickoff` 里 `goal.authorization` 必须先于 `play.mode=goal`；`settle_close` 里 `verify.submit` → `settlement.evolution` → `review.note` 的顺序与 `sessions=` 段的幂等判断）——这条顺序契约**写在适配器里**，Mod 不兜底。

**A2 准入判据**：**当且仅当一个 `action` 能防止一类具体的、可命名的 payload 错位时，它才配拥有一个别名**，并须在适配器契约测试里以断言钉住。`approve_gate` / `reject_gate` 命中：`reqId` 必填可防「漏 `reqId` 时退化去批门禁队列里的下一个待批门」——那是**批错需求**的正确性问题，不是省 token；空 `note` 也必须产出 `"rN|"` 而不是 `"rN"`。答不出「能防哪类具体错误」的零增量别名应当退场，而不是当兼容残留。

**`dispatch` 与压缩的边界**：`dispatch` 严格等于「派发一次事件，返回这一次的完整回执」，**不得**附带任何回合后动作，尤其不得触发上下文压缩——`ctx.compact()` 会 abort 当前回合，与「回执整段可见」不可能同时成立。对上下文压力只**报告**（水位、稳定节点、提示显式调 `handoff_compact`）；压缩只允许在两个显式入口发生：① `action=handoff_compact`（强制）；② `ygg_run_lanes` 的编排层刹车（本波已落、交接单已写之后）。自动水位判断最多只能报告。

## 4. `op=vfs_put` 与 `event=vfs.write` 不等价

同一份字节可经两条路进 VFS，但只有事件路径带语义守卫与台账：

| 维度 | `event=vfs.write` | `op=vfs_put` |
| --- | --- | --- |
| 阶段守卫 | `stage == done` 时**拒绝** | **无**（收口后仍可写） |
| 内容上限 | `MAX_VFS_CONTENT` = **64 KiB**（`65536` 字节），超出即 `reject_payload` | 宿主 `validate_artifact_content` = **256 KiB** |
| 路径狱 | `artifacts/<name>.md`、≤ 128 字节、无 `..`、无空白 | `artifacts/` 前缀、≤ 128 字节、只允许 `[A-Za-z0-9._/-]`、无 `..` 与空路径段，**不强制 `.md`** |
| `artifact.*` 台账 | **写**（`record_artifact(path,bytes,sha,source="vfs")`） | **不写**（只落库 + `vfs_load` 灌进内核 `ygg.fs`） |
| `ygg.fs` 缺席时 | `[VFS LEDGER ONLY]` 降级：照记台账、照叙事，不声称落盘 | 宿主直接落库，不存在这次降级 |

凡需要进 `artifact.*` 台账、需要阶段守卫或 64 KiB 上限的制品，**必须**经 `event=vfs.write`；`op=vfs_put` 只用于不参与台账的宿主侧归档（交接单 `artifacts/handoff-latest.md`、`settle_close` 的进化复盘）。两个名字长得像，但**不是**同一原语的两种写法。

## 5. `event` 全集（39）

`agent.summary`；`requirement.{add,list,dispatch,amend,split,merge,conflict,resolve}`；`lane.{pool,claim,release}`；`proposal.{submit,accept,reject}`；`coordinate.submit`；`verify.submit`；`settlement.{evolution,sessions,todo}`；`review.note`；`gate.{approve,reject}`；`config.{set,open,close}`；`map.show`；`ygg.choice_selected`；`event.start`；`tool.{call,result}`；`vfs.write`；`stage.advance`。另有 `integration.{confirm,reject}`、`release.{confirm,skip}`、`charter.set` / `run.begin` 等生命周期名随幕激活。

**计数口径（可复核）**：`grep -rho 'ygg\.on("[a-z._]*"' .ygg/mods/agent-seed | sed 's/.*ygg\.on("//;s/"//' | sort -u` 得 38；`stage.advance` 由 `ygg.fsm.define`（`crates/ygg-sandbox/stdlib/fsm.lua`）转手注册，`mod.lua` 声明 `event = "stage.advance"`，不在 `ygg.on` 字面处 —— 38 + 1 = 39，与 [../kernel/events.md](../kernel/events.md) 的设计目标全集一致。r1 时代的 33 / registry 34 两数只描述当时尚未递增的已装载集合，滞后已在 r2 集成交付时登记，**不再有两个口径**。

registry 之外的事件名（含拼错的名字与未注册的 `ygg.*`）一律 **fail-closed**：外部名在门面预检零副作用拒绝（可读 VALIDATION，文案带已注册全集），派生名（handler `ygg.emit`）使整次转换失败回滚。registry 是运行时唯一事实，宿主不持任何事件名白名单。

## 6. 载荷速查与语义要点

**载荷是 `|` 分段字符串**，形状在拒绝回执里原样打出——**照抄，永不猜段位**。

| event | payload |
| --- | --- |
| `event.start` / `charter.set` | `""` / `"title\|goal\|success"` |
| `requirement.add` | `"title\|description[\|external_id[\|depends_on]]"` |
| `requirement.dispatch` | `""` / `"0"`（就绪序）/ `"rN"`（仅该需求） |
| `proposal.submit` | `"rN\|rev\|stage\|summary\|artifacts[\|changes[\|external_id[\|exec=<agent_id>[:<reason>]]]]"` |
| `proposal.accept` | `"pN"` |
| `gate.approve` / `gate.reject` | `"rN[\|note]"` / `"rN\|feedback"`（或 `""`） |
| `integration.confirm` / `integration.reject` | `"rN\|develop@<sha>"` / `"rN\|<reason>"` |
| `verify.submit` | `"true"` / `"all_green=false;report=…"` |
| `settlement.evolution` | `"artifact\|summary[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/<remaining>/<destination>][\|todo=<remaining>/<verified_by>]"` |
| `release.confirm` / `release.skip` | `"master@<sha>\|vYYYY.MM.DD.N"` / `"<reason>"` |
| `review.note` | `"lesson\|score(1-5)"` |

- **`rev` 与 `id` 必须取自回执**（`requirement.list` / `map.show`），不得凭记忆：错的 rev 会以 stale 被拒，每次猜多一个完整往返。
- **`requirement.dispatch` 按就绪序派发**（draft → plan；ready → development·lane，且开发依赖全 done），job 上限 4；无候选或槽位已满属正常的 `[DISPATCH] 0 candidates` 回执（幂等，**不计失败步**）。
- **`changes` 只是建议**：`split=a,b` / `merge=r5` / `amend=title:description` / `conflict=r5:note`——子代理只提议，**变更在 `proposal.accept` 时才落地**。
- **`summary` 是裁决面，不是变更日志**：按门禁决策写——先写计划的形状与理由，再写影响与细节，让评审者**不打开代码就能裁决**。技术大件（`file:line`、键名、伪代码、测试清单）放 `artifacts`。分工：summary 论证「为什么这份计划该过」，artifacts 展示「怎么实现、怎么验」。
- **`proposal.accept` 串行且必须点名**：一次只受理一条，用 `pN` / `rN` 指定；空载荷取需求 id 规范序第一条——**待决提案多于一条时必须点名**。
- **三条 reject 路径**：`proposal.reject`（plan 提案）→ 回 planning（门禁清空、`job="plan"` 重挂执行者、`plan_ok` 保持 false、rev 不动）；development 交付 / 验收门 / 集成幕的 reject 都落 ready 幕（`plan_ok` 保住、`job=""`、泳道释放、写 feedback）；`gate.reject`（计划门）→ planning 且 `plan_ok=false`、`rev+1`。
- **stale 与 rebase**：受理时 `rev ≠ req.<i>.rev` → 提案变 stale 并在需求上打 stale 标记（**旧提案永不得覆盖更新的决定**）。恢复路径是「同一阶段、当前 rev 的新提案」，只在需求没有其他待决提案时被受理，rebase 落地后 stale 标记自动清除。
- **拒绝前缀**：动作不在其阶段 / 人类动作被直派 → `[REJECTED:ACTION]`；载荷非法 → `[REJECTED:PAYLOAD]`；前置未满足 → `[BLOCKED:TODO]`。全部**原位重渲染**（主状态不变），并登记在 `pack.json` 的 `agent.rejectionPrefixes`——宿主据此判「无效果」，**不记失败步**。
- **泳道台账**：`lane.pool "label|path"` 幂等 upsert（label `^[a-z][a-z0-9-]{0,23}$`，保留字 `seq`/`pools`；path 即传给宿主的 `--repo` 本地检出）；`lane.claim "lane|req_id|branch|base|clean|label"`（`clean` 必须为 `"1"`，脏树不得认领）；`lane.release "lane|label"` 幂等。并行度按池计，冲突雷达只在同池内比对探针。
- **`config` 子页与直写键**：`config.open` / `config.close` 纯呈现层、永不推进状态；`config.set` / `map.show` 常开。直写键：`review.evolution.{artifact,summary,candidates,classes}`、`goal.authorization`、`executor.<rN>.<stage>`，以及机证注册表 `tools.root`（**首次结算前必须注册**，未注册则回退人工见证并给可读提示、永不拒绝）、`tools.session` / `tools.sessions_root`、`tools.<name>`（`.sh` 脚本覆盖）。
- **`pendingChoice` 顺序**：解决冲突（若有）→ 主推进动作（`proposal.accept` > `gate.approve` > `requirement.dispatch` > `coordinate.submit`）→ `map.show` → `requirement.list` → `requirement.add` → 配置（子页内：play.mode → evolution.mode → gates + back）。

## 7. 设计依据与判据

**部分收敛而非全收敛**：全收敛会逼 `action` 变成 `event` 的镜像（每加一个 `event` 就要同步一个 `action`），制造二份真相；反过来，`action` 若不携带 Mod 的实时状态（就绪序、泳道、探针），适配器就得自建一份工作流知识，那是真正的第二个状态机。折中是让 Mod 自带适配器指令——`summary.jobs[i].subagentAction`（`subagent.spawn` / `subagent.resume`）携带渲染后的 brief、`worktreeStatus` / `subagentStatus` / `probeStatus` 与提交命令，使任何上层执行壳能当**薄执行壳**：只执行、不需理解。

**判据**：① 阻断——给一个答不出「能防哪类具体 payload 错位」的 `action` 新增别名，或让 `action` 直接写 `run.sqlite` / `settings.*` / `req.*` / `stage`；② 症状——`action` 清单与 `event` 清单逐渐对齐（同义重复），或适配器里出现第二套状态判定；③ 确认——Mod 套件 `.ygg/mods/agent-seed/tests/surface.lua` 的三层动作面与 `subagentAction` 形状用例，跑 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed`。
