---
type: "Reference"
title: "quest — 任务类型、实例与委托关系"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-1069a71ebdbfb2150160e2f4
    resource: repo://crates/ygg-sandbox/src/host_api/value.rs
  - id: openwiki-source-6b1a66be116e5bf7e39548a0
    resource: repo://packages/libs/quest/init.lua
  - id: openwiki-source-08cd5334be1cb72434f1cfa6
    resource: repo://packages/libs/quest/pkg.json
  - id: openwiki-source-55c3720264a7916dd1a016f0
    resource: repo://packages/libs/quest/test/pkg-quest.lua
  - id: openwiki-source-0a013123dca8c7d13f37d07a
    resource: repo://packages/mods/pkg-selftest/tests/pkg-quest-smoke.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# quest — 任务类型、实例与委托关系

> **id** packages/quest · **模块** ⑤ packages · 通用能力层 · **触发** 改任务模板 / 实例形状、步骤推进语义或终态判定之前
> **读不到代价** silent-drift（两个游戏的步骤推进语义各自演化，同一份存档在两边读出不同的「进度」）
> **replaces**：DESIGN §4 quest（搬运基线 `3a2536f`）
> **上游**：[README.md](README.md) · **相关**：[entity.md](entity.md)（委托方 / 承接方是实体 ref）· [codec.md](codec.md)（`quest` 前缀随存档导出）· [dialogue.md](dialogue.md)（订阅派生事件）· [../kernel/events.md](../kernel/events.md)

## 1. 边界（这个包不做什么）

- **不定义类型清单**：有哪些任务类型、每类几步、每步叫什么——数据。
- **不写指引文案**：步骤指引是文本键，经 `ygg.data` 读、经 [dialogue.md](dialogue.md) 播。
- **不定义委托门**：谁能接、接了能不能退——是 mod 的规则；本包只记 `giver` / `taker` 两个 ref。
- **不做 UI**：不渲染任务列表，不发 choice。
- **不推进对话**：任务推进只发派生事件，由 [dialogue.md](dialogue.md) 订阅消费，不反向调用。

## 2. 数据形状

逻辑形状：`types.<name>` = 流程模板、`inst.<id>` = 实例、`index` = id 集合、`seq` = 已分配的最大 id。

**物理落点（KV 只收标量）**：宿主 `ygg.kv.set` 经 `value_from_lua` 只接受四个标量型，表写入即 `unsupported payload type: table`。所以「表值」的字段全部落成**扁平标量键 + 换行名表**（与 [entity.md](entity.md) §2 同一套形状）：

| 键 | 值 | 说明 |
| --- | --- | --- |
| `quest.seq` | number | 已分配的最大 id；**唯一写口 = `open`** |
| `quest.index.<id>` | `true` | id 集合的成员标记（唯一索引真源；泄漏口径数它） |
| `quest.types` | string | 类型名表（升序、换行连接） |
| `quest.t.<name>.steps` | number | 步骤数（`advance` 只读这一枚——§5 的 O(1) 依据） |
| `quest.t.<name>.s.<i>.id` / `.guide` / `.on_enter` | string | 第 `i` 步（1 基）；`on_enter` 的 `nil` 落**空串**（KV 无删除，空串是唯一可往返的「无」） |
| `quest.t.<name>.m.names` | string | `meta_spec` 的键名表（升序） |
| `quest.t.<name>.m.<key>` | string | 类型词 `number` / `string` / `boolean` / `ref`（复用 entity 的词表，不另造第五种） |
| `quest.inst.<id>.type` / `.step` / `.state` | string / number / string | |
| `quest.inst.<id>.giver` / `.taker` | number | `0` = 无（`nil` 存不进 KV） |
| `quest.inst.<id>.m.names` | string | 本实例**实际写过**的 meta 键名表（升序） |
| `quest.inst.<id>.m.<key>` | 标量 | meta 值（只写调用方真的给了的键） |

实例上的 `m.names` 不是冗余：读面一旦向模板要 meta 名表，`get` / `advance` / `open_ids` 就全部退化回 `O(步数)`（每次重建整张步骤表），而 §5 把「按 id 查找 / 更新」钉在 `O(1)`。一枚键换 O(1) 是这笔账的解法。

模板（对外）：`{ steps = { { id, guide, on_enter }, … }, meta_spec = { <键> = <类型词> } }`；步骤 `id` 在类型内唯一，`guide` 是文本键、本身不含文案。实例（对外）：`{ id, type, step, state, giver, taker, meta }`——`step` 是 1 基下标，`giver` / `taker` 是实体 ref 或 `nil`。

### 事件载荷（`ygg.emit` 只收标量，形状必须定形）

| 事件 | 载荷 |
| --- | --- |
| `quest.opened` | `"<id>\|<type>"` |
| `quest.advanced` | `"<id>\|<step>"`（推进**后**的步下标） |
| `quest.completed` | `"<id>"` |
| `quest.abandoned` | `"<id>\|<reason>"`（`reason` 缺省 = 空串） |
| `quest.jumped` | `"<id>\|<from>\|<to>"`（步**下标**，不是步 id） |

`on_enter` **不是**事件：它是 `quest.step()` 的读数，由 mod / dialogue 消费。自动 emit 它会让 I6「名字集合恰好相等」这条断言失去意义。

## 3. 不变量

- **I1 类型是模板不是继承**：实例只存 `type` 名 + 当前 `step` 下标，不复制步骤内容。改模板的 steps 不改已有实例的数据（否则改一次数据 = 改所有存档的解释）。
- **I2 推进单调**：`advance` 只 `+1`，不 `-1`、不跳。要跳到别的步骤必须显式 `jump(id, step_id)` 并留下 `quest.jumped`（否则历史不可审计）。
- **I3 终态唯一**：`state` 从 `open` 只能一次进入 `done` 或 `abandoned`；终态后再 `advance` / `jump` / `abandon` 一律报错 `YGG-PKG-QUEST-TERMINAL`。
- **I4 委托是引用不是所有权**：`giver` / `taker` 是实体 ref；实体销毁不自动关闭任务，悬垂由 mod 处理（对齐 [entity.md](entity.md) I4）。
- **I5 元数据袋不参与流程**：`meta` 里任何键都不影响 `advance` 的判定（难度、奖励、来源都在这里）；流程只看 `step` 下标与模板。
- **I6 派生事件面固定四个**：`opened` / `advanced` / `completed` / `abandoned`（外加显式跳步的 `jumped`）。它们经 `ygg.emit` 走**内部派生事件**通道，**不是**内核事件——新增内核事件仍然要改 `crates/**` 与 [../kernel/events.md](../kernel/events.md)，包不得为此扩容。
- **I7 无时钟**：步骤推进不得依据 `ygg.ticks()`（有 API 无推进者，恒 0）；「限时任务」由 mod 用事件计数表达。
- **I8 顶层纯度**：包模块顶层只有纯导出，注册一律经 `M.install(deps)`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `quest.define` | `(name, template)` | `true` | 写（`quest.types`）；异 spec 重复定义报错 |
| `quest.type` | `(name)` | template\|nil | 纯读 |
| `quest.open` | `(type_name, opts)` | `id` | 写 + 事件 `quest.opened` |
| `quest.get` | `(id)` | 实例表（浅副本） | 纯读 |
| `quest.assign` | `(id, giver_ref, taker_ref)` | `boolean` | 写；只设 ref，不判权限 |
| `quest.advance` | `(id)` | `state`（推进后） | 写 + `quest.advanced` / `quest.completed` |
| `quest.jump` | `(id, step_id)` | `boolean` | 写 + `quest.jumped` |
| `quest.abandon` | `(id, reason)` | `boolean` | 写 + `quest.abandoned` |
| `quest.open_ids` | `(owner_ref?)` | `{id, …}`（升序） | 纯读；O(n)，n = 存活任务数 |
| `quest.step` | `(id)` | `{ index, id, guide, on_enter }` | 纯读 |
| `quest.keys` | `()` | `{键, …}`（升序） | 纯读；`codec.export({ keys = quest.keys() })` 的点名面 |

**错误码（逐条可断）**：码面 `YGG-PKG-QUEST-<COND>`，`: ` 之后是可读细节；用例按码做子串断言。

| 码 | 触发 |
| --- | --- |
| `-NAME` | 类型名不匹配 `^[a-z][a-z0-9_-]*$` |
| `-TEMPLATE` | 模板形状坏：`steps` 空 / 非表、步 `id` 重复或非串、`guide` / `on_enter` 非串、`meta_spec` 键或类型词非法 |
| `-TYPE` | 类型未注册；或异 spec 重复 `define`（不覆盖，I1） |
| `-META` | `meta` 键未在 `meta_spec` 声明 / 值与声明的类型词不符 / `opts.meta` 非表 / `reason` 非串 |
| `-REF` | `giver` / `taker` / `owner_ref` 不是正整数 id 或 `nil` |
| `-ID` | 实例 id 不存在，或不是正整数 |
| `-STEP` | `jump` 的步 id 不在该类型的模板里 |
| `-TERMINAL` | 终态（`done` / `abandoned`）后的任何原语（I3） |
| `-INSTALL` | `install(deps)` 收到非表 |
| `-HOST` | `ygg` 不可达且未注入 `deps.host`（环境面，与 entity 的 `-HOST` 同义） |

## 5. 性能预算

**口径声明**：下列实测值产在 **selftest（普通 Lua + mock KV）**，因为沙箱里既无 `os.clock` 也无 `collectgarbage`。所以内存那行是**包侧 Lua 堆上界**（含 mock KV 的表开销），不是宿主 Rust 堆；序列化吞吐不在此表（[codec.md](codec.md) §5 独占那个口径）。复算：`--filter pkg-quest`（三次取中位）。

| 指标 | 阈值 | 实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`；**实测 0.9625 ms**（单次 0.87–1.05 ms；50 次 clear+require 的每轮均值，批内三批再取中位） |
| 实体按 id 查找 / 更新 | `O(1)` | **强约束**：`get` / `advance` 都是「按 id 直取标量键」，无遍历。N = 1 / 100 / 10k 三档档间比 **1.045×**（单次 1.038–1.058）；判据 `t(10k)/t(1) ≤ 1.5`（1.2 会被 GC 脉冲打成假红）。`get` 三档 7617 / 7761 / 7975 ns/op、`advance` 8.6 / 8.2 / 8.5 µs/op（5000 / 300 次 × 3 批取中位） |
| 序列化吞吐 | 附基准集与耗时 | **不适用**：实例随 `quest` 前缀由 [codec.md](codec.md) §5 的基准集覆盖；本包不自定口径 |
| 10k 实体常驻内存 | 给出数字 | 1k 同时存活任务（4 步 + 3 个 meta 键）**实测 598 KiB**（目标 `≤ 1 MiB`，硬帽 `16 MiB`）；口径同 [entity.md](entity.md) §5 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | **口径改写成「每轮重置后恒为 1」**：包无删除面（§4 的 11 个函数里没有 `delete`，[codec.md](codec.md) I5 亦明令不删），所以「100 轮后回到起点」只能在每轮一份新 KV 的语义下成立。实测 100 轮 `open → advance×末步`：每轮 `quest.index` 条目数**恒为 1**，GC 差 ≤ 1% |

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 每层事件任务 | 模板 + 实例 + 步骤推进；「这一层触发了什么」是数据，不是代码 | P3 |
| CDDA | mission 链 | 委托关系（NPC → 玩家）+ 多步链；任务就是实体 ref，不需要并行系统 | P3 |
| DCSS | 分支任务 | 显式 `jump` 表达分叉（拿到符文后走另一支），事件留痕可审计 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.quest.`。
- **断言面**：I1 用「改模板后旧实例的 `step` 仍解释为同一语义」；I2 / I3 断拒绝路径与事件计数；I5 用「往 meta 塞任意键，推进结果不变」；I6 断事件名集合**恰好相等**（不是「包含」）。
- **计数**：**24** 条（20 行为 + 4 性能）计入 `--filter pkg-quest` 的 `cases passed=`；载体是 `packages/mods/pkg-selftest/`——生产 Mod（`agent-seed`）不声明包，它的计数永远是 79/0，这是回归钉。
- **禁止宽松断言**：只断「有事件发出」不断事件名与次数，视同没测 I6。

## 8. 设计依据

**为什么类型 / 实例两分。** 四个复现项目的任务系统看着差异很大（每层事件、mission 链、分支任务），但骨架相同：一类流程 + 多个在跑的具体实例 + 谁给的谁接的。把「类」与「个」分开，才能做到「加一类任务 = 加一条数据」，而不是给每个任务写一个 Lua 状态机。

**反方案一：把流程写成 JSON 条件树**——表达力被锁死在固定 schema 里，每加一种叙事花样就要改 schema，而 schema 是双语言共同产物（Rust 与 TS 各校验一遍），改动成本远高于改数据；步骤 + 事件钩子这种最薄的骨架能覆盖四项目的全部用点。**反方案二：每个 mod 自带任务状态机**——代价是四份「进度怎么存」的隐式约定，且没有一处能被断言。

**判据（①②③）**。① 阻断——在 mod 里自建任务进度存储；把奖励数值 / 审批规则写进包；用 `ygg.ticks` 做限时。② 症状——同一任务在存档里有两个进度字段；分支任务靠改模板硬编码成两条类型；任务终态被重复设置导致奖励双发。③ 确认——本页被索引；`selftest` 含 `pkg.quest.*`；§5 五个指标有数字。

维护责任 = 改 `packages/libs/quest/**` 的执行者；触发 = 任一相关 commit；检查 = `--filter pkg-quest` 计数 + §5 五指标 + 事件名集合断言。
