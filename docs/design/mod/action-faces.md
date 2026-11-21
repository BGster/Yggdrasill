# Yggdrasill · 三层动作面契约（event/op/action）
> **id** mod/action-faces · **模块** ③ agent-seed mod · **触发** 新增或移动一个动作面之前
> **读不到代价** silent-drift（把 op 当 event 用 → 状态旁路）

> **定位**：L1/L2/L3 分层、三条硬不变量（I1–I3）、A1–A4 落点与 event 全集 registry 口径末注（33/34 口径）。
> **replaces**：DESIGN §3.4（L446–508）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[events.md](../kernel/events.md) · [cli-daemon.md](../host/cli-daemon.md) · [compaction.md](../kernel/compaction.md)
> **搬运**：以下正文自 DESIGN.md L446–L508 逐字迁移，不重写语义。

### 3.4 三层动作面契约：`event` / `op` / `action`

Yggdrasill 的动作面被严格分成三层，任何一层都不得越权承担另一层的职责。这三个词在本文档后文一律按此定义使用，不许混用。

| 层 | 载体 | 值域 | 权威来源 | 职责边界 |
| --- | --- | --- | --- | --- |
| **L1 `event`** | `ygg.on(event_name, handler)` 注册的 handler（`stage.advance` 由 `ygg.fsm.define` 转手注册） | 39 个事件名（全集见本节末注，payload 位次见 §3.3；`guide.show` 已随 r1 注销、`task.intake` 已随 r2 rev2 注销） | Mod（`.ygg/mods/agent-seed/src/**`（core/domain/view/runtime/handlers 五层）、`mod.lua`） | **唯一的状态推进货币**：`(state, event) -> state'`，全有或全无（见 CONTEXT `P1`）。事件之外不存在任何改状态的路径。 |
| **L2 `op`** | daemon JSON-RPC 请求的 `op` 字段 | `ping` / `summary` / `dispatch` / `vfs_put` / `vfs_get` / `vfs_list` / `settle` / `shutdown`（8 个） | Rust 宿主（`crates/ygg-sandbox/examples/egg/`）；线协议见 §3.5 | **状态推进之外的旁路能力**：制品读写、摘要查询、结算报告、进程控制。`op=vfs_put` 与 `event=vfs.write` 是**两条平行端口**，互不经过对方，且语义不等价（见下文对照表）。 |
| **L3 `action`** | `ygg_session` 工具的 `action` 参数（`StringEnum`） | `kickoff` / `dispatch` / `approve_gate` / `reject_gate` / `vfs_write` / `vfs_read` / `vfs_list` / `handoff_compact` / `settle_close`（9 个） | 各 harness 适配器（本仓库无内置，执行壳已删除）；L3 由各 harness 自行定义，值域随适配器可变，不是跨 harness 契约 | **面向模型/人的语义入口**：它只决定「用哪个动词说出意图、由谁拼 payload」，**不决定状态**。值域随适配器可变，**不是**跨 harness 契约。 |

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

#### 本节末注 · `event` 全集（39 个）

`agent.summary`、`requirement.add` / `requirement.list` / `requirement.dispatch` / `requirement.amend` / `requirement.split` / `requirement.merge` / `requirement.conflict` / `requirement.resolve`、`lane.pool` / `lane.claim` / `lane.release`、`proposal.submit` / `proposal.accept` / `proposal.reject`、`coordinate.submit`、`verify.submit`、`settlement.evolution` / `settlement.sessions` / `settlement.todo`、`review.note`、`gate.approve` / `gate.reject`、`config.set` / `config.open` / `config.close`、`map.show`、`ygg.choice_selected`、`event.start`、`tool.call` / `tool.result`、`vfs.write`、`stage.advance`。

计数口径（可复核，r6 实测复核）：`grep -rho 'ygg\.on("[a-z._]*"' .ygg/mods/agent-seed | sed 's/.*ygg\.on("//;s/"//' | sort -u` 得到 **38** 项；`stage.advance` 由 `ygg.fsm.define`（`crates/ygg-sandbox/stdlib/fsm.lua`）转手注册，`mod.lua` 里声明 `event = "stage.advance"`，不在 `ygg.on` 的字面出现处——38 + 1 = **39**，与 `events.md` 的 39 设计目标全集一致。r1 时代的 33 / registry 34 两数只描述当时尚未递增的已装载事实集（滞后已在 r2 集成交付时登记）；r6 复核后已装载集与设计目标全集追平，不再有两个口径。这 39 个事件构成状态机的完整原子转换集合；`action` 面**从不**表示「这 39 个之外还有别的状态转换」。registry 之外的事件名（含拼错的名字与未注册的 `ygg.*`）派发一律 **fail-closed**——外部名在门面预检零副作用拒绝（可读 VALIDATION，文案带已注册全集），派生名（handler `ygg.emit`）使整次转换失败回滚。registry 是运行时唯一事实，宿主不持任何事件名白名单。

#### 设计依据（部分收敛 / 适配器为什么只是“薄壳”）

**三层不做完全收敛，而是部分收敛**：`event` 是推进的唯一货币（全集不以字面硬编码，口径与复算命令见上节），`op` 是状态之外的宿主端口（8 个），`action` 是适配器的语义入口（值域随 harness 可变）。**禁止把 `action` 当作 `event` 的第二份清单**：9 个 `action` 里只有 `approve_gate` / `reject_gate` 与 `event` 重叠，且是有意保留的**位次正确性别名**（`reqId` 必填可防“批错需求”）。**Mod 自带适配器指令**：`summary.jobs[i].subagentAction`（`subagent.spawn` / `subagent.resume`）携带渲染后的 brief、`worktreeStatus` / `subagentStatus` / `probeStatus` 与提交命令，使任何上层执行壳（Claude Code / Codex / Cursor 适配器）能当**薄执行壳**。

**依据**：全收敛会逼 `action` 变成 `event` 的镜像，于是每加一个 `event` 都要同步加一个 `action`，而 `action` 的消费者是模型可读的语义面，不是状态机——两边同步只会制造二份真相。反过来，`action` 若不携带 Mod 的实时状态（就绪序、泳道、探针），适配器就必须自己重建一份工作流知识，那是真正的第二个状态机。把指令做成结构化的 `subagentAction`，适配器只需执行、不需理解。

**判据**：① 阻断——给一个答不出“能防哪类具体 payload 错位”的 `action` 新增别名、或让 `action` 直接写 `run.sqlite` / `settings.*` / `req.*` / `stage`；② 症状——`action` 清单与 `event` 清单逐渐对齐（同义重复），或适配器里出现第二套状态判定；③ 确认——`cargo test -p ygg-sandbox --test selftest` 的三层动作面契约用例与 `summary.jobs[].subagentAction` 形状用例。


### 动作载荷与语义细节（原 playbook `actions` 章，2026-11-15 归位）

**载荷是 `|` 分段字符串**。形状在拒绝回执里会原样打出——**照抄，永不猜段位**：

| event | payload |
| --- | --- |
| `event.start` | `""` / `"title\|goal\|success"`（可选章程） |
| `charter.set` | `"title\|goal\|success"` |
| `run.begin` | `""` |
| `requirement.add` | `"title\|description[\|external_id[\|depends_on]]"` |
| `requirement.dispatch` | `""` / `"0"`（就绪序）/ `"rN"`（仅该需求） |
| `proposal.submit` | `"rN\|rev\|stage\|summary\|artifacts[\|changes[\|external_id[\|exec=<agent_id>[:<reason>]]]]"`（尾链 `exec`→`actor`→`actor_session`，后者两条由宿主在声明路由注入） |
| `proposal.accept` | `"pN"` |
| `proposal.reject` | `"pN\|<reason>"` |
| `gate.approve` / `gate.reject` | `"rN[\|note]"` / `"rN\|feedback"`（或 `""`） |
| `integration.confirm` / `integration.reject` | `"rN\|develop@<sha>"` / `"rN\|<reason>"` |
| `coordinate.submit` | `""` |
| `verify.submit` | `"true"` / `"all_green=false;report=…"` |
| `settlement.evolution` | `"artifact\|summary[\|c=<kept>/<screened>[\|cls=<kind>:<n>,...]][\|sessions=<moved>/<remaining>/<destination>][\|todo=<remaining>/<verified_by>]"` |
| `settlement.sessions` / `settlement.todo` | `"moved\|remaining\|destination"` / `"remaining\|verified_by"` |
| `release.confirm` / `release.skip` | `"master@<sha>\|vYYYY.MM.DD.N"` / `"<reason>"` |
| `review.note` | `"lesson\|score(1-5)"` |

- **`rev` 与 `id` 必须取自回执**（`requirement.list` / `map.show`），不得凭记忆或沿用上一次提交：错的 rev 会以 stale 被拒（回执里 `{{want}}` 打出真实当前 rev），每次猜都要多一个完整 dispatch 往返。
- **`requirement.dispatch` 按就绪序派发**（draft → plan；ready → development·lane，且开发依赖全 done），job 上限 4；无候选或槽位已满属正常的 `[DISPATCH] 0 candidates` 回执（幂等，**不计失败步**）。
- **`changes` 只是建议**：`split=a,b` / `merge=r5` / `amend=title:description` / `conflict=r5:note`——子代理只提议，**变更在 `proposal.accept` 时才落地**。
- **`summary` 是裁决面，不是变更日志**：按门禁决策写——先写计划的形状与理由，再写影响与细节，是评审者**不打开代码就能裁决**的人读散文。技术大件（`file:line`、键名、伪代码、测试清单）放 `artifacts`。分工：summary 论证「为什么这份计划该过」，artifacts 展示「怎么实现、怎么验」。
- **`proposal.accept` 串行且必须点名**：一次只受理一条，用 `pN` / `rN` 指定；空载荷取需求 id 规范序的第一条——**待决提案多于一条时必须点名**，否则验错需求。
- **三条 reject 路径**：`proposal.reject`（plan 提案）→ 回 planning（门禁清空、`job="plan"` 重新挂到执行者、`plan_ok` 保持 false、rev 不动）；`proposal.reject`（development 交付）/ `gate.reject`（验收门）/ `integration.reject` 都落 ready 幕（`plan_ok` 保住、`job=""`、泳道释放、写 feedback）；`gate.reject`（计划门）→ planning 且 `plan_ok=false`、`rev+1`（需要新计划）。
- **stale 与 rebase**：受理时 `rev ≠ req.<i>.rev` → 提案变 stale 并在需求上打 stale 标记（可读拒绝；**旧提案永不得覆盖更新的决定**）。恢复路径是「同一阶段、当前 rev 的新提案」：这种 rebase 提案只在需求**没有其他待决提案**时被受理，rebase 落地后 stale 标记自动清除。
- **四类拒绝前缀**：动作不在其阶段 / 人类动作被直派 → `[REJECTED:ACTION]`；载荷非法 → `[REJECTED:PAYLOAD]`；前置未满足 → `[BLOCKED:TODO]`。全部**原位重渲染**（主状态不变），且都登记在 `pack.json` 的 `agent.rejectionPrefixes`——宿主据此判「无效果」，**不记失败步**。
- **主代理的直接变更动作**：`requirement.split "parent|child A|child B[|…]"`（父变 split，子继承依赖，至少 2 个）；`requirement.merge "id1,id2|title|description"`（源变 merged，新需求取并集依赖）；`requirement.amend "id|new title|new description|new deps"`（rev+1；对 done 需求 amend 派生新需求，对开发中 amend 只更新说明）；`requirement.conflict "id1,id2|note"`（双方 blocked，派发与收口跳过）；`requirement.resolve "id1,id2|keep_both|keep_first|revise[|note]"`。
- **泳道台账**：`lane.pool "label|path"` 注册具名池（幂等 upsert；label `^[a-z][a-z0-9-]{0,23}$`，保留字 seq/pools；path 即传给宿主的 `--repo` 本地检出）；`lane.claim "lane|req_id|branch|base|clean|label"` 认领或补挂牌（`clean` 必须为 "1"，脏树不得认领；末尾池段可省 = 运行仓默认池）；`lane.release "lane|label"` 释放（幂等，按 (池, lane) 作用域）。并行度按池计；冲突雷达只在同池内比对探针。
- **config 子页与直写键**：`config.open` / `config.close` 进出二级配置页（play mode · evolution preference · gates），纯呈现层、永不推进状态；`config.set` / `map.show` 是常开动作。不走菜单的预登记键一律直写 `config.set`：`review.evolution.artifact|summary|candidates|classes`（存 `settings.review.evolution.*`，逐键查形状；`settlement.evolution` 统一校验并写本轮登记，`review.note` 再单源归档到 `review.evolution.*`）、`goal.authorization`（收口检查用的逐局授权）、`executor.<rN>.<stage>`（执行者台账，收口不校验、只作展示）、以及 harness 机证注册表：`tools.root`（机检开关 + 脚本根，**首次结算前必须注册**，未注册则回退人工见证并给可读提示、永不拒绝）、`tools.session` / `tools.sessions_root`（保留参数键：主会话 id 与会话库路径——沙箱读不到进程环境，结算 cross-check 的 argv 只能来自这些 KV）、`tools.<name>`（以 `.sh` 结尾的脚本路径覆盖）。无 egg 的参数约定见 host/harness-tools.md。
- **`requirement.list`（`""` / `"0"` / `"full"`）只读**：逐行需求图（状态 + 依赖 + rev + 泳道 + job + 冲突/stale 痕迹）、待决提案清单、泳道台账与图例。
- **`map.show` 打两层地图**：协调者 4 状态节点图 + 需求管线（含两道门禁）+ job / 泳道台账；泳道行形如 `lane-<k> ← <requirement> | branch | base | since #N | clean`（未设字段显示 `-`）。
- **`pendingChoice` 顺序**：解决冲突（若有）→ 主推进动作（`proposal.accept` > `gate.approve` > `requirement.dispatch` > `coordinate.submit`）→ `map.show` → `requirement.list` → `requirement.add` → 配置（子页内：play.mode → evolution.mode → gates + back）。
