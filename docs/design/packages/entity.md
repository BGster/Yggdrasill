# Yggdrasill · packages/entity — 实体与属性袋（机制层）

> **id** packages/entity · **模块** ⑤ packages · 通用能力层 · **触发** 改实体 / 属性袋形状、索引或生命周期事件之前
> **读不到代价** silent-drift（属性袋形状在两个游戏里各自演化，序列化与索引静默分叉，导入时才炸）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.entity.*`）· [../kernel/state-surface.md](../kernel/state-surface.md)（KV 契约 §2.8）

## 1. 边界（这个包不做什么）

- **不定义属性语义**：生命归零算死、攻击力怎么算、抗性怎么叠加——是 mod 的规则。
- **不落盘**：持久化格式归 [codec.md](codec.md)；本包只声明 KV 前缀与形状。
- **不复制 `ygg.entity.*`**：内核已有 `spawn` / `exists` / `destroy` / `get_attribute` / `set_attribute` / `add_tag` / `remove_tag` / `has_tag`。本包在它之上加 **schema 校验、存活索引、生命周期事件、引用槽**，不实现第二套 id 分配，也不存第二份属性。
- **不提供查询语言、不建继承树**：没有 `where` 式 DSL，按 tag / schema 的集合查询是唯一查询面；类型 = 已注册的属性 schema，类型差异是数据。

## 2. 数据形状

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `id` | 正整数，单调分配 | 是 | 由 `entity.spawn` 分配 | 内核实体表；对外只经 id 引用 |
| `schema` | string（已注册的 schema 名） | 是 | — | `ygg.kv` 键 `entity.rec.<id>.schema` |
| `attrs` | table<string, boolean\|number\|string> | 否 | `{}` | `ygg.kv` 键 `entity.rec.<id>.attrs` |
| `tags` | table<string, true> | 否 | `{}` | `ygg.kv` 键 `entity.rec.<id>.tags` |
| `index` | table<number, true>（存活 id 集合） | 是 | — | `ygg.kv` 键 `entity.index`（唯一索引真源） |
| `schemas` | table<string, spec> | 是 | — | `ygg.kv` 键 `entity.schemas` |
| `seq` | number（已分配的最大 id） | 是 | `0` | `ygg.kv` 键 `entity.seq` |

schema spec 形状：`{ attrs = { <name> = { type = "number"|"string"|"boolean"|"ref", required = <bool>, default = <v> } }, tags = { <name> = { required = <bool> } } }`。`type = "ref"` 的值是正整数 id，与 number 分开声明是为了让悬垂引用可被断言发现。

## 3. 不变量

- **I1 单调分配**：`spawn` 返回的 id 严格大于此前所有返回过的 id；`destroy` 后不复用。落点 `entity.seq` 自增。
- **I2 属性值域窄化**：值只能是 `boolean` / `number` / `string`，`ref` 值是正整数 id。表、函数写入即报错 `YGG-PKG-ENTITY-ATTR`。理由：KV 能存表，但属性袋一旦允许嵌套表，「按属性查找」与「序列化形状」立刻分成两套。
- **I3 schema 门**：`spawn(schema, …)` 与该 schema 的 `set(id, k, v)` 只接受 spec 声明过的键；未声明键写入即报错。新增属性 = 改数据里的 spec，不改包。
- **I4 悬垂引用不是错误**：`destroy(id)` 不遍历、不清理其它实体的引用槽；读悬垂 ref 仍得 id，`exists(ref) == false`。清理归 mod 的语义（「召唤物死亡，主人失去它」是规则）。
- **I5 索引是唯一真源**：`entity.index` 的键集合与 `entity.rec.<id>` 的实际存在逐一对应；`exists(id)` 只查索引，绝不扫 KV（`ygg.kv` 没有遍历 API，只有 `get` / `set`，见 [../kernel/state-surface.md](../kernel/state-surface.md) §2.8）。
- **I6 生命周期事件恰好一次**：每次 `spawn` 恰好一次 `entity.created`；每次 `destroy` 恰好一次 `entity.destroyed`；`set` 只在值真的变化时发一次 `entity.attr_changed`（同值写入不发）。
- **I7 顶层纯度**：包模块顶层只有纯导出，状态改动一律在 `M.install(deps)` 或调用期（对齐 [../mod/mod-package.md](../mod/mod-package.md) 的 `mod.pure-export`）。
- **I8 无时钟**：实体不得带时间戳字段；沙箱的 `ygg.ticks()` 有 API 无推进者（恒 0，见 [../kernel/state-surface.md](../kernel/state-surface.md) §2.8.1），生命周期先后用事件序号表达。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `entity.define` | `(name, spec)` | `true` | 写（`entity.schemas`）；同 spec 重复定义幂等，异 spec 报错 |
| `entity.schema` | `(name)` | `spec\|nil` | 纯读 |
| `entity.spawn` | `(name, attrs?, tags?)` | `id` | 写 + 事件 `entity.created` |
| `entity.destroy` | `(id)` | `boolean` | 写 + 事件 `entity.destroyed` |
| `entity.exists` | `(id)` | `boolean` | 纯读 |
| `entity.get` | `(id, key)` | `value\|nil` | 纯读 |
| `entity.set` | `(id, key, value)` | `boolean`（值是否变化） | 写 + 事件（变化才发） |
| `entity.attrs` | `(id)` | table（浅副本） | 纯读 |
| `entity.tag` / `entity.untag` | `(id, tag)` | `boolean` | 写 + 事件 `entity.tagged` / `entity.untagged` |
| `entity.has` | `(id, tag)` | `boolean` | 纯读 |
| `entity.ids` / `entity.count` | `(tag?, schema?)` | `{id, …}`（升序）/ number | 纯读；O(n)，n = 存活实体数 |

副作用类别沿用 [../kernel/lua-api.md](../kernel/lua-api.md) 的分类：`纯读` / `写` / `写 + 事件`。`写` 类在模块顶层调用即被 `YGG-MOD-EFFECT` 拒绝。

## 5. 性能预算

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`。纯 Lua，`M.install` 不做全量反序列化。测法：空 run 起局 vs 含本包起局，各 3 次取中位，量 `event.start` → 首个 choice 的墙钟差。 |
| 实体按 id 查找 / 更新 | `O(1)` | **强约束**：`get` / `set` = 一次 `ygg.kv.get("entity.rec.<id>")` + 一次表取，无遍历。测法：N = 1 / 100 / 10k 三档，单次调用耗时不随 N 增长（≤ 1.2×）。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：承包给 [codec.md](codec.md) §5，基准集与口径只在那里定义一次。 |
| 10k 实体常驻内存 | 给出数字 | 目标 `≤ 2 MiB`（沙箱硬帽 `16 MiB`，见 [../kernel/lua-api.md](../kernel/lua-api.md) 六项上限）。口径：10k 实体 × 8 属性 + 3 tag，`collectgarbage("count")` 前后差 ÷ 1024。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | `spawn` + `destroy` × 100 后 `entity.index` 条目数回到起点，`collectgarbage("count")` 差 `≤ 1%`。 |

P0 期这一列是**目标值**；P2 首实现填实测值，偏差回写本表（Goal §六「偏差回写文档」）。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 生物 / 道具实例化 | schema 声明属性袋（HP 上限、类型、稀有度）+ 组合而非继承 | P3 |
| CDDA | JSON 定义 → 实体实例 | `define` + `spawn`：把 `ygg.data` 读到的数据条目直接实例化，零手写类型代码 | P3 |
| DCSS | 玩家 / 怪物 / 地面物品 | 同一套属性袋承载三种「看着不同」的存在（死物同构） | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.entity.`（例 `pkg.entity.attr-domain`、`pkg.entity.attr-changed-once`）。
- **断言面**：I1–I8 每条至少一个用例；I2 / I3 的拒绝路径必须断言「报错而非静默忽略」（`assert_eq(err, "YGG-PKG-ENTITY-ATTR")`）。
- **计数**：用例计入 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed` 的 `cases passed=`（发现机制见 [README.md](README.md) §5 未决项，P1 定）；**禁止宽松断言**：`assert(true)`、只断类型不断值、只断「没报错」一律作废（Goal 反例 5）。

## 8. 设计依据

**为什么这样切。** 实体只做三件事：分配 id、装属性、发事件。这三件事在四个复现项目里逐字相同，差别只在属性**名字**与规则，所以名字进数据、规则进 mod。属性值域收窄到标量是刻意的：一旦允许嵌套表，`set` 的「变化才发事件」判定要写深比较，「按属性查找」要写路径解析，两处都会随时间漂移成不同的语义。

**反方案一：继承树**（生物 → 怪物 → Boss）——加一个「既是物品又能施放」的存在就要多重继承，且每加一层都要改包，直接违背「新增类型 = 加数据」。**反方案二：每个游戏自带实体系统**——代价可数：四份 `next_id`、四份序列化、四份悬垂引用处理，且两个游戏的同一属性可能一个存 number 一个存 string，跨游戏导入时才炸（Goal 反例 2 判这种自造基建为未完成）。

**判据（①②③）。** ① 阻断——在 mod 里自己实现 id 分配或属性存储；在包里写属性语义（「生命归零算死」）；把表的形状偷偷塞进 `attrs`。② 症状——两个游戏各有一份 `next_id`；同一属性在两个游戏里类型不同导致导入失败；`destroy` 之后引用槽被包悄悄改写。③ 确认——本文件存在且被 [../README.md](../README.md) 索引；`selftest` 的 `cases passed=` 含 `pkg.entity.*`；§5 五个指标有数字。
