# Yggdrasill · packages/entity — 实体与属性袋（机制层）

> **id** packages/entity · **模块** ⑤ packages · 通用能力层 · **触发** 改实体 / 属性袋形状、索引或生命周期事件之前
> **读不到代价** silent-drift（属性袋形状在两个游戏里各自演化，序列化与索引静默分叉，导入时才炸）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.entity.*`）· [../kernel/state-surface.md](../kernel/state-surface.md)（KV 契约 §2.8）

## 1. 边界（这个包不做什么）

- **不定义属性语义**：生命归零算死、攻击力怎么算、抗性怎么叠加——是 mod 的规则。
- **不落盘**：持久化格式归 [codec.md](codec.md)；本包只声明 KV 前缀与形状。
- **不复制内核实体面**：内核实有 **6 面**——`spawn` / `get_attribute` / `set_attribute` / `add_tag` / `remove_tag` / `has_tag`（`crates/ygg-sandbox/src/host_api.rs` 的 `entity_table` 只 `raw_set` 这六次，`crates/ygg-core/src/entity.rs` 的 `EntityStore` 也没有删除面）。P0 版这里写的 `exists` / `destroy` **内核没有**；本包在它之上加 **schema 校验、存活索引、生命周期事件、引用槽**，不实现第二套 id 分配，也不存第二份属性。
- **`destroy` 是语义降级，不是内核删除**：宿主没有删除属性/实体的面，所以本包的 `destroy` = 把存活位改写为 `false`（KV 无删除，只能写墓碑）+ 清掉已声明的 tag + 一条事件。**属性留在内核**，靠存活位在每一读口子遮蔽。代价可数：死掉的实体仍占内核内存与本包的两枚墓碑键。补齐这个缺口需要内核新增 `ygg.entity.destroy` / `exists`——已登记在 §8，属 `crates/**` 的改动面，**不在本包**。
- **不提供查询语言、不建继承树**：没有 `where` 式 DSL，按 tag / schema 的集合查询是唯一查询面；类型 = 已注册的属性 schema，类型差异是数据。

## 2. 数据形状

**对外的 spec 形状**（`define(name, spec)` 的入参，也是 `schema(name)` 的返回）：
`{ attrs = { <name> = { type = "number"|"string"|"boolean"|"ref", required = <bool>, default = <v> } }, tags = { <name> = { required = <bool> } } }`。
`type = "ref"` 的值是正整数 id，与 number 分开声明是为了让悬垂引用可被断言发现。

**存储形状**由下面两个宿主事实决定，不是风格选择：① 属性值与 tag 有现成内核面（`set_attribute` / `add_tag`，随快照往返、按 id O(1)）；② `ygg.kv` 只收**标量**（`boolean` / `number` / `string`）——没有表、没有删除、没有遍历（`crates/ygg-core/src/kv.rs` 的 S2 标量值域 + `host_api.rs` 的 `value_from_lua` 四型直通）。于是 P0 版写的「`attrs` 存一张表」「`index` 存一张表」在宿主上**不可实现**，改成下表：

| 逻辑字段 | 物理落点 | 键 / 调用 | 写点 |
| --- | --- | --- | --- |
| `id` | 内核 `EntityStore`（`next_id` 从 1 起、致密、单调、不复用、随快照持久） | `ygg.entity.spawn()` | `spawn` / `import` |
| `schema` | KV 标量 | `pkg.entity.rec.<id>.schema` = string | `spawn` |
| `attrs` | **内核属性面**（逐属性一个标量，值域天然 = I2） | `ygg.entity.set_attribute` / `get_attribute` | `set` / `spawn` |
| `tags` | **内核标签面** | `ygg.entity.add_tag` / `remove_tag` / `has_tag` | `tag` / `untag` / `spawn` / `destroy` |
| 存活位（P0 版叫 `index`） | KV 标量（逐 id 一布尔位）+ 计数 + 水位 | `pkg.entity.alive.<id>` = bool；`pkg.entity.count` = int；`pkg.entity.seq` = int | `spawn` / `destroy` |
| `schemas` | KV 扁平标量（无可存表） | 见下表 | `define` |

前缀命名权威 = [../mod/mod-package.md](../mod/mod-package.md) §3.2.3「包只写 `pkg.<id>.*`」——P0 版写的 `entity.*` 与之冲突，已全部改 `pkg.entity.*`。

schema 注册表的扁平键（`<n>` = schema 名，`<a>` = 属性名，`<t>` = tag 名）：

| 键 | 值 | 说明 |
| --- | --- | --- |
| `pkg.entity.schemas` | string（已定义名，**换行连接、字节序升序**） | 唯一的枚举源（无 KV 遍历） |
| `pkg.entity.schemas_rev` | int（每次 define +1） | 进程内 spec 镜像的失效锚（唯一用途） |
| `pkg.entity.schema.<n>.attrs` | string（属性名，换行连接、升序） | 属性枚举源 |
| `pkg.entity.schema.<n>.tags` | string（tag 名，换行连接、升序） | tag 枚举源 |
| `pkg.entity.schema.<n>.a.<a>.type` | `"number"` \| `"string"` \| `"boolean"` \| `"ref"` | |
| `pkg.entity.schema.<n>.a.<a>.required` | bool | |
| `pkg.entity.schema.<n>.a.<a>.has_default` | bool | 必填的**存在性**位：`false` / `0` / `""` 都是合法缺省值，不能用「键缺失」表达 |
| `pkg.entity.schema.<n>.a.<a>.default` | 标量（仅 `has_default=true`） | |
| `pkg.entity.schema.<n>.t.<t>.required` | bool | |

**为什么 `attrs` 进内核而不是 KV。** 放进 KV 要给每个属性一个 `pkg.entity.rec.<id>.a.<name>` 键（也成立），但那会把 `destroy` 的残留从「内核一条记录」变成「KV 里十几个墓碑键」，并丢掉内核已有的按 id O(1) 与快照持久化。代价是**属性名必须可枚举**——恰好由 schema 的 `attrs` 名表提供：I3 已把「未声明键」判为错误，所以「属性名集合 = schema 声明集」是**已经隐含**的不变量，本设计只是把它用起来。

**`seq` 的语义（P0 版写「本包自增」，不准）。** 分配 id 的是内核（`EntityStore::next_id`，只增不减、不复用），本包只记**水位镜像**：`spawn` 后 `seq = max(seq, id)`。它服务两件事：`ids` 的扫描上界，与 codec 的「导入保 id」判定。

**冷启动边界（诚实声明）。** `pkg.entity.seq` 缺失时（包刚接入一个已有存档的局，或 mod 直接调过 `ygg.entity.spawn()`）本包**无法收养**它出生前的实体：内核没有「列 id」面，扫不到就是扫不到。本包只索引自己 spawn 过的实体。

### 事件载荷（`ygg.emit` 只收标量，形状必须显式定形）

`ygg.emit` 的载荷经 `value_from_lua` 只接受四个标量型，表会被拒——所以载荷不是「随便什么」，是下面这张表：

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `entity.created` / `entity.destroyed` | 实体 `id`（number） | 与 `spawn` 的返回值一致 |
| `entity.attr_changed` | `"<id>\|<key>\|<tostring(value)>"` | `wire` 是事件文本，不是持久化字节 |
| `entity.tagged` / `entity.untagged` | `"<id>\|<tag>"` | |

## 3. 不变量

- **I1 单调分配**：`spawn` 返回的 id 严格大于此前所有返回过的 id；`destroy` 后不复用。**分配者是内核**（`EntityStore::next_id`，只增不减），本包只把水位镜像到 `pkg.entity.seq`（`max(seq, id)`）——不是本包自增。
- **I2 属性值域窄化**：值只能是 `boolean` / 有限 `number` / `string`，`ref` 值是正整数 id（`2.5` / `"3"` / `0` / `-1` 全拒）。表、函数、thread 写入即报错 `YGG-PKG-ENTITY-ATTR`。理由：属性袋一旦允许嵌套表，`set` 的「变化才发事件」要写深比较，「按属性查找」要写路径解析，两处都会随时间漂成不同语义。（P0 版此处的理由写「KV 能存表」——那是错的，KV 只收标量；但结论不变，而且更强：宿主的 `set_attribute` 本身也拒非有限浮点。）
- **I3 schema 门**：`spawn(schema, …)` 与该 schema 的 `set(id, k, v)` 只接受 spec 声明过的**属性键**，`tag` / `untag` 只接受声明过的 **tag**；未声明即报错（属性/值域 → `YGG-PKG-ENTITY-ATTR`，tag / spec 面 → `YGG-PKG-ENTITY-SCHEMA`）。新增属性或 tag = 改数据里的 spec，不改包。
  - tag 为何也收窄：`destroy` 只能清「已声明的 tag 名单」——内核没有列 tag 面，未声明的 tag 无法枚举也无从摸除，在「同 id 不复用」下会永久占内存。
- **I4 悬垂引用不是错误**：`destroy(id)` 不遍历、不清理其它实体的引用槽；读悬垂 ref 仍得 id，`exists(ref) == false`。清理归 mod 的语义（「召唤物死亡，主人失去它」是规则）。
- **I5 存活位是唯一真源**：`pkg.entity.alive.<id>` 是「这个 id 能不能被看到」的唯一判据，**每一读口子都过它**（`get` / `attrs` / `has` / `tags` / `schema_of` / `ids` / `count`），所以死掉的实体在对外面上与不存在**不可区分**。`exists(id)` 只读这一个键（`pkg.entity.alive.<id>`），绝不扫 KV——`ygg.kv` 没有遍历 API，只有 `get` / `set`（[../kernel/state-surface.md](../kernel/state-surface.md) §2.8）。
  - **墓秃是宿主无删除面的直接代价**：`alive.<id>` 只能 `true` → `false`，`rec.<id>.schema` 只能留在原处（内核也不删属性）。所以一次 `destroy` 恰好留下两枚键，不多少不少——codec 的泄漏用例把这个「恰好」当断言。
- **I6 生命周期事件恰好一次**：每次 `spawn` 恰好一次 `entity.created`；每次 `destroy` 恰好一次 `entity.destroyed`（二次 `destroy` 返回 `false` 且零事件）；`set` 只在值真的变化时发一次 `entity.attr_changed`（同值写入不发，`math.type` 参与比较）。载荷形状见 §2 的「事件载荷」。
- **I7 顶层纯度**：包模块顶层只有纯导出，状态改动一律在 `M.install(deps)` 或调用期（对齐 [../mod/mod-package.md](../mod/mod-package.md) 的 `mod.pure-export`）。
- **I8 无时钟**：实体**不得产生任何时间字段**（沙箱里 `ygg.ticks()` 有 API 无推进者，恒 0；事件 seq 不注入 Lua）。生命周期先后由**宿主派发序**保证，包不做任何时序判断、也不发明时间戳。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `entity.define` | `(name, spec)` | `true` | 写（`pkg.entity.schemas*`）；同 spec 重复定义幂等且**零写**，异 spec 报 `YGG-PKG-ENTITY-SCHEMA` |
| `entity.schema` | `(name)` | `spec\|nil`（深副本） | 纯读 |
| `entity.spawn` | `(name, attrs?, tags?)` | `id` | 写 + 事件 `entity.created` |
| `entity.destroy` | `(id)` | `boolean`（首次 `true`，二次 `false`） | 写（存活位 + 清已声明 tag）+ 事件 `entity.destroyed` |
| `entity.exists` | `(id)` | `boolean` | 纯读（一次 KV 读） |
| `entity.get` | `(id, key)` | `value\|nil` | 纯读 |
| `entity.set` | `(id, key, value)` | `boolean`（值是否变化） | 写 + 事件（变化才发） |
| `entity.attrs` | `(id)` | table（浅副本） | 纯读 |
| `entity.tag` / `entity.untag` | `(id, tag)` | `boolean`（是否真的变化） | 写 + 事件 `entity.tagged` / `entity.untagged` |
| `entity.has` | `(id, tag)` | `boolean` | 纯读 |
| `entity.tags` | `(id)` | `{tag, …}`（升序） | 纯读；**导出侧所需**（P0 版漏此面：codec 要把一条记录的 tag 全取出来，而宿主无列 tag 面） |
| `entity.schema_of` | `(id)` | `schema\|nil` | 纯读（死之或不存在 = `nil`） |
| `entity.ids` / `entity.count` | `(tag?, schema?)` | `{id, …}`（升序）/ number | 纯读；O(n)，n = 存活实体数 |

副作用类别沿用 [../kernel/lua-api.md](../kernel/lua-api.md) 的分类：`纯读` / `写` / `写 + 事件`。`写` 类在模块顶层调用即被 `YGG-MOD-EFFECT` 拒绝。

## 5. 性能预算

**口径声明（先读再读数字）**：下列实测值产在 **selftest（普通 Lua）**，因为沙箱内既无 `collectgarbage` 也无 `os.clock`（`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死）。所以内存那行是**包侧 Lua 堆上界**（含 mock KV 与 mock 实体表的开销），不是宿主 Rust 堆的读数；

来源：每行都有对应用例打印 `PERF entity.<指标>=<值> <单位>`，三次取中位（同一条命令跑 3 遍取第 2 个值）。复算：`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-entity`。

| 指标 | 阈值（Goal 原则二） | 本包实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 实测 **0.67 ms**（目标 `≤ 1ms` 达成）。测法（已回写）：`loadfile` + `install` 各 100 次取每轮均值，`os.clock` 计时——P0 写的「空 run 起局 vs 含本包起局的墙钟差」信号（≤ 1ms）**低于整条链的启动抖动**（> 1ms），测不出东西。 |
| 实体按 id 查找 / 更新 | `O(1)` | 实测 **1162.9 ns/op**；N = 1 / 100 / 10000 三档的档间比 **1.07×–1.19×**，断言 **≤ 1.5×**（见下面「阈值为何是 1.5」）。**强约束的落点**：`get` / `set` = 存活位读 + `pkg.entity.rec.<id>.schema` 读 + 内核属性面，无遍历。载体限制：这条跑在 mock 上，能抓到的是**包侧**引入的遍历（例如「按 id 找 schema 时扫一遍 ids」），不是内核面的复杂度。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：承包给 [codec.md](codec.md) §5，基准集与口径只在那里定义一次。 |
| 10k 实体常驻内存 | 给出数字 | 实测 **6637.1 KiB**（包侧 Lua 堆上界）+ **1155576 B**（同量实体的键名 + 值字节和，载体无关）。口径：10k × 8 属性 + 3 tag，`collectgarbage("count")` 前后差 ÷ 1024。P0 写的 `≤ 2 MiB` 是按沙箱内 Rust 侧口径定的阈值，与这个上界口径**不可比**（6.6 MiB 里 mock 的 Lua 表开销占大头）；沙箱硬帽是 `64 MiB`（`sandbox.rs` 的 `SANDBOX_MEMORY_LIMIT`——P0 此处写的 16 MiB 已被 P1 调整，本行对齐）。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 实测 `spawn` + `destroy` × 100 后：`count` / `#ids` 回起点、`seq` **恰好** +100（id 不复用）、KV 键数 **恰好** +200（每轮两枚墓秃键）、内存 △**0.668%**（断言 ≤ 1%）。口径（比 P0 更强）：P0 写「`entity.index` 条目数回到起点」——在无删除面的 KV 上「键数不变」**不可达**（每次 destroy 必留两枚墓秃键），所以断言改成「**恰好等于** destroy 次数」，同时排掉「每轮多写一个计数键」这类慢性漏。 |

**阈值为何是 1.5（R4 的落地，与契约原文的 1.2 不同，此处明写理由）。** 契约原写「单次调用耗时不随 N 增长（≤ 1.2×）」，但 1.2 是按「无噪声」估的：实测 3 次全量跑的档间比是 1.07 / 1.09 / 1.18，另有一次 1.4× **假红**（1000 次/档时）。噪声源是两个：`os.clock` 的粒度与「全量跑前面已经堆到 12 MB 量级」带来的 GC 脉冲。处置是两步：① 内层轮次提到 5000 × 3 批取中位（把 GC 噪声平均掉）；② 阈值提到 **1.5×**。为何 1.5 仍然有效：真的出现遍历是 **O(N)** 量级（10³–10⁴×），与噪声不在一个数量级；而 1.07–1.19 这个区间里没有任何一档表现出「随 N 单调增长」的形状。

P0 期这一列是**目标值**；P2 首实现填实测值，偏差回写本表（Goal §六「偏差回写文档」）。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 生物 / 道具实例化 | schema 声明属性袋（HP 上限、类型、稀有度）+ 组合而非继承 | P3 |
| CDDA | JSON 定义 → 实体实例 | `define` + `spawn`：把 `ygg.data` 读到的数据条目直接实例化，零手写类型代码 | P3 |
| DCSS | 玩家 / 怪物 / 地面物品 | 同一套属性袋承载三种「看着不同」的存在（死物同构） | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.entity.`（例 `pkg.entity.attr-domain`、`pkg.entity.attr-changed-once`）；perf 用例名前缀 `pkg.entity.perf-`。
- **断言面**：I1–I8 每条至少一个用例；I2 / I3 的拒绝路径必须断言「报错而非静默忽略」（`expect_error("YGG-PKG-ENTITY-ATTR", ...)`）。
- **计数**：用例计入 `cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-` 的 `cases passed=`。发现机制 = `pack.json` 的 `packages` 段声明驱动（[README.md](README.md) §5）；生产 Mod（`agent-seed`）不声明包，所以它看不到 `pkg.*`（计数保持 79/0），这就是为什么需要 `packages/mods/pkg-selftest` 这介消费夹具（D14 落地）。
  - **perf 文件不计数**：`pkg-entity-perf.lua` 用 file 级断言（`error` ⇒ `file FAIL` ⇒ `ok=0`），不进 `cases passed=`——这样「40 条」这个可复算计数不被 perf 污染，而判定一样硬。
  - **`--filter` 的硬约束**：它同时对 mod 侧与包侧文件过滤，且「一处都不匹配」会以 `没有可执行的测试文件` 直接 exit 1。所以按包跑只能用 `--filter pkg-entity` / `--filter pkg-codec`（各有对应的冒烟文件接住），不能单跑一个 perf 文件。
  - **禁止宽松断言**：`assert(true)`、只断类型不断值、只断「没报错」一律作废（Goal 反例 5）。

## 8. 设计依据

**为什么这样切。** 实体只做三件事：分配 id、装属性、发事件。这三件事在四个复现项目里逐字相同，差别只在属性**名字**与规则，所以名字进数据、规则进 mod。属性值域收窄到标量是刻意的：一旦允许嵌套表，`set` 的「变化才发事件」判定要写深比较，「按属性查找」要写路径解析，两处都会随时间漂移成不同的语义。

**反方案一：继承树**（生物 → 怪物 → Boss）——加一个「既是物品又能施放」的存在就要多重继承，且每加一层都要改包，直接违背「新增类型 = 加数据」。**反方案二：每个游戏自带实体系统**——代价可数：四份 `next_id`、四份序列化、四份悬垂引用处理，且两个游戏的同一属性可能一个存 number 一个存 string，跨游戏导入时才炸（Goal 反例 2 判这种自造基建为未完成）。

**实现纪律（沙箱方言，不回写就等着真实跑局炸）。** 沙箱 globals 里没有标准库的 `string` / `table` / `utf8` / `coroutine` 表（`sandbox.rs` 的白名单测试钉死 `string == nil` 等六项），而 `selftest` 跑在**普通 Lua** 上、这些表都在——所以任何「在 selftest 里绿、在沙箱里 nil-index 崩」的写法都测不出来。两条硬纪律：① 字符串操作只能走**方法语法**（`("x"):sub(1, 1)`，走字符串元表的 `__index`），`string.x` 与 `table.sort` 之类一律不可用，排序/拼接手写（`sort_names` / `join_lines`）；② 交付门禁带一条静态 grep（`grep -n 'string\.\|table\.\(sort\|concat\|insert\|remove\)' packages/libs/entity/*.lua` 必须 0 行，注释里也不写——否则它会变成噪声真阳性）。

**登记：内核需要 `ygg.entity.destroy` / `ygg.entity.exists`（本需求不实现）。** 本包的 `destroy` 是语义降级（墓秃 + 存活位），代价可数且会随复现项目放大：死掉的实体仍占内核内存与本包两枚 KV 键，且无法真的回收 id。补齐需要改 `crates/**`（内核能力面），属另一条需求的改动面；本需求的红线明确不改 `crates/**`，所以只登记、不实现。同理，内核补面后本包才能**收养**出生前的存量实体（现在只能索引自己 spawn 过的）。

**判据（①②③）。** ① 阻断——在 mod 里自己实现 id 分配或属性存储；在包里写属性语义（「生命归零算死」）；把表的形状偷偷塞进 `attrs`。② 症状——两个游戏各有一份 `next_id`；同一属性在两个游戏里类型不同导致导入失败；`destroy` 之后引用槽被包悄悄改写。③ 确认——本文件存在且被 [../README.md](../README.md) 索引；`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-entity` 的 `cases passed=` 含 18 条 `pkg.entity.*`；§5 五个指标有数字，异常则 file 级断言先红。
