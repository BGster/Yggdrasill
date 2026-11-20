# Yggdrasill · packages/quest — 任务类型、实例与委托关系

> **id** packages/quest · **模块** ⑤ packages · 通用能力层 · **触发** 改任务模板 / 实例形状、步骤推进语义或终态判定之前
> **读不到代价** silent-drift（两个游戏的步骤推进语义各自演化，同一份存档在两边读出不同的「进度」）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [entity.md](entity.md)（委托方 / 承接方是实体 ref）· [codec.md](codec.md)（`quest` 前缀随存档导出）· [../kernel/events.md](../kernel/events.md)（派生事件面）

## 1. 边界（这个包不做什么）

- **不定义类型清单**：有哪些任务类型、每类几步、每步叫什么——数据。
- **不写指引文案**：步骤指引是文本资源，经 `ygg.data` 读、经 [dialogue.md](dialogue.md) 播。
- **不定义委托门**：谁能接、接了能不能退——是 mod 的规则；本包只记 `giver` / `taker` 两个 ref。
- **不做 UI**：不渲染任务列表，不发 choice。
- **不推进对话**：任务推进只发派生事件，由 [dialogue.md](dialogue.md) 订阅消费，不反向调用。

## 2. 数据形状

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `types.<name>` | table（流程模板） | 是 | — | `ygg.kv` 键 `quest.types` |
| `inst.<id>` | table（实例） | 是 | — | `ygg.kv` 键 `quest.inst.<id>` |
| `index` | table（id 集合，键为 id） | 是 | — | `ygg.kv` 键 `quest.index`（唯一索引真源） |
| `seq` | number（已分配的最大实例 id） | 是 | `0` | `ygg.kv` 键 `quest.seq` |

模板形状：`{ steps = { { id = "…", guide = "text.key", on_enter = "event.name" }, … }, meta_spec = { <键> = <类型> } }`。步骤 `id` 在类型内唯一；`guide` 是文本键，本身不含文案。

实例形状：`{ type = "…", step = <1 基下标>, state = "open"|"done"|"abandoned", giver = <实体 ref|nil>, taker = <实体 ref|nil>, meta = { … } }`。

## 3. 不变量

- **I1 类型是模板不是继承**：实例只存 `type` 名 + 当前 `step` 下标，不复制步骤内容。改模板的 steps 不改已有实例的数据（否则改一次数据 = 改所有存档的解释）。
- **I2 推进单调**：`advance` 只 `+1`，不 `-1`、不跳。要跳到别的步骤必须显式 `jump(id, step_id)` 并留下 `quest.jumped` 事件（否则历史不可审计）。
- **I3 终态唯一**：`state` 从 `open` 只能一次进入 `done` 或 `abandoned`；终态后再 `advance` / `jump` / `abandon` 一律报错 `YGG-PKG-QUEST-TERMINAL`。
- **I4 委托是引用不是所有权**：`giver` / `taker` 是实体 ref；实体销毁不自动关闭任务，悬垂由 mod 处理（对齐 [entity.md](entity.md) I4）。
- **I5 元数据袋不参与流程**：`meta` 里任何键都不影响 `advance` 的判定（难度、奖励、来源都在这里）；流程只看 `step` 下标与模板。
- **I6 派生事件面固定四个**：`quest.opened` / `quest.advanced` / `quest.completed` / `quest.abandoned`（外加显式跳步的 `quest.jumped`）。它们经 `ygg.emit` 走**内部派生事件**通道，**不是**内核事件——新增内核事件仍然要改 `crates/**` 与 [../kernel/events.md](../kernel/events.md)，包不得为此扩容。
- **I7 无时钟**：步骤推进不得依据 `ygg.ticks()`（有 API 无推进者，恒 0，见 [../kernel/state-surface.md](../kernel/state-surface.md) §2.8.1）；「限时任务」由 mod 用事件计数表达。
- **I8 顶层纯度**：包模块顶层只有纯导出，注册一律经 `M.install(deps)`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `quest.define` | `(name, template)` | `true` | 写（`quest.types`）；异 spec 重复定义报错 |
| `quest.type` | `(name)` | template\|nil | 纯读 |
| `quest.open` | `(type_name, opts)` | `id` | 写 + 事件 `quest.opened` |
| `quest.get` | `(id)` | 实例表（浅副本） | 纯读 |
| `quest.assign` | `(id, giver_ref, taker_ref)` | `boolean` | 写；只设 ref，不判权限 |
| `quest.advance` | `(id)` | `state`（推进后） | 写 + 事件 `quest.advanced` / `quest.completed` |
| `quest.jump` | `(id, step_id)` | `boolean` | 写 + 事件 `quest.jumped` |
| `quest.abandon` | `(id, reason)` | `boolean` | 写 + 事件 `quest.abandoned` |
| `quest.open_ids` | `(owner_ref?)` | `{id, …}`（升序） | 纯读；O(n)，n = 存活任务数 |
| `quest.step` | `(id)` | `{ index, id, guide, on_enter }` | 纯读 |

## 5. 性能预算

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`（纯 Lua，`install` 不预读模板）。测法同 [entity.md](entity.md) §5。 |
| 实体按 id 查找 / 更新 | `O(1)` | **强约束**：`get` / `advance` = 一次 `ygg.kv.get("quest.inst.<id>")` + 模板直取，无遍历。测法：N 任务数 1 / 100 / 10k 三档，耗时不随 N 增长（≤ 1.2×）。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：任务实例随 `quest` 前缀由 [codec.md](codec.md) §5 的基准集覆盖；本包不自定口径。 |
| 10k 实体常驻内存 | 给出数字 | 目标 1k 同时存活任务 `≤ 1 MiB`（每任务 4 步 + 3 个 meta 键）；硬帽 `16 MiB`。口径同 [entity.md](entity.md) §5。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 100 轮 `open → advance×末步` 后 `quest.index` 条目数回到起点，内存差 `≤ 1%`。 |

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 每层事件任务 | 模板 + 实例 + 步骤推进；「这一层触发了什么」是数据，不是代码 | P3 |
| CDDA | mission 链 | 委托关系（NPC → 玩家）+ 多步链；任务就是实体 ref，不需要并行系统 | P3 |
| DCSS | 分支任务 | 显式 `jump` 表达分叉（拿到符文后走另一支），事件留痕可审计 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.quest.`（例 `pkg.quest.advance-monotonic`、`pkg.quest.terminal-once`）。
- **断言面**：I1 用「改模板后旧实例的 `step` 仍解释为同一语义」；I2 / I3 断拒绝路径与事件计数；I5 用「往 meta 塞任意键，推进结果不变」；I6 断事件名集合恰好相等（不是「包含」）。
- **计数**：计入 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed` 的 `cases passed=`（发现机制见 [README.md](README.md) §5 未决项）。
- **禁止宽松断言**：只断「有事件发出」不断事件名与次数，视同没测 I6。

## 8. 设计依据

**为什么类型/实例两分。** 四个复现项目的任务系统看着差异很大（每层事件、mission 链、分支任务），但骨架相同：一类流程 + 多个在跑的具体实例 + 谁给的谁接的。把「类」与「个」分开，才能做到「加一类任务 = 加一条数据」，而不是给每个任务写一个 Lua 状态机。

**反方案一：把流程写成 JSON 条件树。** 表达力被锁死在固定 schema 里，每加一种叙事花样就要改 schema，而 schema 是双语言共同产物（Rust 与 TS 各校验一遍），改动成本远高于改数据。步骤 + 事件钩子这种最薄的骨架能覆盖四项目的全部用点。

**反方案二：每个 mod 自带任务状态机。** 代价是四份「进度怎么存」的隐式约定，且没有一处能被断言——Goal 反例 2 判这种自造基建为未完成。

**判据（①②③）。** ① 阻断——在 mod 里自建任务进度存储；把奖励数值 / 审批规则写进包；用 `ygg.ticks` 做限时。② 症状——同一任务在存档里有两个进度字段；分支任务靠改模板硬编码成两条类型；任务终态被重复设置导致奖励双发。③ 确认——本文件被索引；`selftest` 含 `pkg.quest.*`；§5 五个指标有数字。
