# Yggdrasill · 三层动作面契约（event/op/action）

> **定位**：L1/L2/L3 分层、三条硬不变量（I1–I3）、A1–A4 落点与 event 全集 registry 口径末注（33/34 口径）。
> **replaces**：DESIGN §3.4（L446–508）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[events.md](events.md) · [cli-daemon.md](cli-daemon.md) · [compaction.md](compaction.md)
> **搬运**：以下正文自 DESIGN.md L446–L508 逐字迁移，不重写语义。

### 3.4 三层动作面契约：`event` / `op` / `action`

Yggdrasill 的动作面被严格分成三层，任何一层都不得越权承担另一层的职责。这三个词在本文档后文一律按此定义使用，不许混用。

| 层 | 载体 | 值域 | 权威来源 | 职责边界 |
| --- | --- | --- | --- | --- |
| **L1 `event`** | `ygg.on(event_name, handler)` 注册的 handler（`stage.advance` 由 `ygg.fsm.define` 转手注册） | 32 个事件名（全集见本节末注，payload 位次见 §3.3；`guide.show` 已随 r1 注销、`task.intake` 已随 r2 rev2 注销） | Mod（`.ygg/mods/agent-seed/src/**`（core/domain/view/runtime/handlers 五层）、`mod.lua`） | **唯一的状态推进货币**：`(state, event) -> state'`，全有或全无（见 CONTEXT `P1`）。事件之外不存在任何改状态的路径。 |
| **L2 `op`** | daemon JSON-RPC 请求的 `op` 字段 | `ping` / `summary` / `dispatch` / `vfs_put` / `vfs_get` / `vfs_list` / `settle` / `shutdown`（8 个） | Rust 宿主（`crates/ygg-sandbox/examples/egg.rs`）；线协议见 §3.5 | **状态推进之外的旁路能力**：制品读写、摘要查询、结算报告、进程控制。`op=vfs_put` 与 `event=vfs.write` 是**两条平行端口**，互不经过对方，且语义不等价（见下文对照表）。 |
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

#### 本节末注 · `event` 全集（32 个）

`agent.summary`、`requirement.add` / `requirement.list` / `requirement.dispatch` / `requirement.amend` / `requirement.split` / `requirement.merge` / `requirement.conflict` / `requirement.resolve`、`lane.pool` / `lane.claim` / `lane.release`、`proposal.submit` / `proposal.accept` / `proposal.reject`、`coordinate.submit`、`verify.submit`、`settlement.evolution` / `settlement.sessions` / `settlement.todo`、`review.note`、`gate.approve` / `gate.reject`、`config.set` / `config.open` / `config.close`、`map.show`、`ygg.choice_selected`、`event.start`、`tool.call` / `tool.result`、`vfs.write`、`stage.advance`。

计数口径（可复核）：前 31 个由 Mod 直接调用 `ygg.on(...)` 注册；`stage.advance` 是第 32 个，它由 `ygg.fsm.define`（`crates/ygg-sandbox/stdlib/fsm.lua`）转手注册，**不在** `ygg.on` 的字面出现处。可执行复算：`grep -rn 'ygg\.on("' .ygg/mods/agent-seed/ | sed 's/.*ygg\.on("\([^"]*\)".*/\1/' | sort -u` 得到 31 项，再加上 `mod.lua` 的 `ygg.fsm.define` 声明里那一项 `event = "stage.advance"`，合计 33。这 33 个事件构成状态机的完整原子转换集合；`action` 面**从不**表示「这 33 个之外还有别的状态转换」。运行期 registry = 这 33 个 + stdlib 内置注册面（`ygg.narrative.goto`，合计 34）；registry 之外的事件名（含拼错的名字与未注册的 `ygg.*`）派发一律 **fail-closed**——外部名在门面预检零副作用拒绝（可读 VALIDATION，文案带已注册全集），派生名（handler `ygg.emit`）使整次转换失败回滚。registry 是运行时唯一事实，宿主不持任何事件名白名单。

