# Yggdrasill · packages/entity — 实体与属性袋（机制层）

> **id** packages/entity · **模块** ⑤ packages · 通用能力层 · **触发** 改实体 / 属性袋形状、索引或生命周期事件之前
> **读不到代价** silent-drift（属性袋形状在两个游戏里各自演化，序列化与索引静默分叉，导入时才炸）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.entity.*`）· [../kernel/state-surface.md](../kernel/state-surface.md)（KV 契约 §2.8）

## 1. 边界（这个包不做什么）

- **不定义属性语义**：生命归零算死、攻击力怎么算、抗性怎么叠加——是 mod 的规则。
- **不落盘**：持久化格式归 [codec.md](codec.md)；本包只声明 KV 前缀与形状。
- **不复制内核实体面**：内核实有 **8 面**——`spawn` / `get_attribute` / `set_attribute` / `add_tag` / `remove_tag` / `has_tag` 六面，加 S9 交付的**控制面两键** `exists(id)` / `destroy(id)`（`crates/ygg-sandbox/src/host_api.rs` 的 `entity_table` `raw_set` 这八次；`crates/ygg-core/src/entity.rs` 的 `EntityStore` 里 `remove_attribute` / `iter` 仍在 store 层、**不在宿主面上**——它们没有消费者）。本包在它之上加 **schema 校验、生命周期事件、引用槽**，不实现第二套 id 分配，不存第二份属性，**也不镜像存活位**。
- **`destroy` 是内核真删，不是墓碑降级**（S9）：`ygg.entity.destroy(id)` → `EntityStore::remove`，属性 + tag + 记录**一并回收**；分配水位（`next_id`）不动 ⇒ I1「id 永不复用」是**分配**性质，与本体是否永存无关。存活真源因此只有一个 = 内核存在性（I5），本包不再写任何 `alive` 位。**旧档边界（不兼容，不写迁移）**：本改动前产生的、含 `pkg.entity` 状态的存档里，死亡实体的内核记录还在，升级后会被读成存活——须重开一局；一次性 purge 登记在 §8（S9-b）。
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
| 存活位（P0 版叫 `index`） | **内核存在性**（S9 起零 KV 键，见 I5）+ 计数 + 水位 | `ygg.entity.exists(id)`；`pkg.entity.count` = int；`pkg.entity.seq` = int | `spawn` / `destroy` |
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

**冷启动边界（诚实声明）。** `pkg.entity.seq` 缺失时（包刚接入一个已有存档的局，或 mod 直接调过 `ygg.entity.spawn()`）本包**无法收养**它出生前的实体：宿主面没有「列 id」调用（`EntityStore::iter` 在，但没上去），扫不到就是扫不到。本包只索引自己 `spawn` 过的实体。

### 事件载荷（`ygg.emit` 只收标量，形状必须显式定形）

`ygg.emit` 的载荷经 `value_from_lua` 只接受四个标量型，表会被拒——所以载荷不是「随便什么」，是下面这张表：

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `entity.created` / `entity.destroyed` | 实体 `id`（number） | 与 `spawn` 的返回值一致 |
| `entity.attr_changed` | `"<id>\|<key>\|<tostring(value)>"` | `wire` 是事件文本，不是持久化字节 |
| `entity.tagged` / `entity.untagged` | `"<id>\|<tag>"` | |

## 3. 不变量

- **I1 单调分配**：`spawn` 返回的 id 严格大于此前所有返回过的 id；`destroy` 后不复用。**分配者是内核**（`EntityStore::next_id`，只增不减），本包只把水位镜像到 `pkg.entity.seq`（`max(seq, id)`）——不是本包自增。S9 的真删**不动水位**（`EntityStore::remove` 只动实体表与 `revision`），所以「删除后 `spawn` 拿到更大 id」逐字保持——I1 是**分配**性质，与本体是否永存无关。
- **I2 属性值域窄化**：值只能是 `boolean` / 有限 `number` / `string`，`ref` 值是正整数 id（`2.5` / `"3"` / `0` / `-1` 全拒）。表、函数、thread 写入即报错 `YGG-PKG-ENTITY-ATTR`。理由：属性袋一旦允许嵌套表，`set` 的「变化才发事件」要写深比较，「按属性查找」要写路径解析，两处都会随时间漂成不同语义。（P0 版此处的理由写「KV 能存表」——那是错的，KV 只收标量；但结论不变，而且更强：宿主的 `set_attribute` 本身也拒非有限浮点。）
- **I3 schema 门**：`spawn(schema, …)` 与该 schema 的 `set(id, k, v)` 只接受 spec 声明过的**属性键**，`tag` / `untag` 只接受声明过的 **tag**；未声明即报错（属性/值域 → `YGG-PKG-ENTITY-ATTR`，tag / spec 面 → `YGG-PKG-ENTITY-SCHEMA`）。新增属性或 tag = 改数据里的 spec，不改包。
  - tag 为何也收窄：`tag` / `untag` 只收已声明的名单。S9 起 `destroy` 不再需要它来擦 tag（内核真删把整个 tag 集一并回收），但收窄保留：它仍是 I3 的拒绝面（拼错 tag 名 = 错，不是无操作）。
- **I4 悬垂引用不是错误**：`destroy(id)` 不遍历、不清理其它实体的引用槽；读悬垂 ref 仍得 id，`exists(ref) == false`。清理归 mod 的语义（「召唤物死亡，主人失去它」是规则）。
- **I5 内核存在性是唯一真源**（S9 修订）：`ygg.entity.exists(id)` 是「这个 id 能不能被看到」的唯一判据（底层 = `EntityStore::contains`，一次 BTreeMap 查），**每一读口子都过它**（`get` / `attrs` / `has` / `tags` / `schema_of` / `ids` / `count`），所以死掉的实体在对外面上与不存在**不可区分**。本包**不再写 `pkg.entity.alive.<id>`**（S9 前是 KV 布尔位）：一旦 `ygg.entity.destroy` 公开在宿主面上，任何 mod 都能直呼它，KV 位就会与内核裂成两套可见性（`ids` 吐幽灵、`count` 偏高、`set` 招 core 的 `not found`）——真源只能一个。
  - **残留恰好一枚键**：`rec.<id>.schema` 留在原处（`ygg.kv` 无删除面：`KvStore::remove` 在核里但没上宿主面）。所以一次 `destroy` **恰好**新增 1 枚 KV 键，不多不少——`pkg-entity-perf.lua` 的泄漏用例把这个「恰好」当断言（S9 前是 2 枚）。
  - **旧档边界**：S9 前的存档里死亡实体的内核记录还在（当时用 `alive=false` 遮蔽），升级后会被读成存活——**不兼容、不写迁移**，重开一局；一次性 purge 登记在 §8（S9-b）。
- **I6 生命周期事件恰好一次**：每次 `spawn` 恰好一次 `entity.created`；每次 `destroy` 恰好一次 `entity.destroyed`（二次 `destroy` 返回 `false` 且零事件）；`set` 只在值真的变化时发一次 `entity.attr_changed`（同值写入不发，`math.type` 参与比较）。载荷形状见 §2 的「事件载荷」。
- **I7 顶层纯度**：包模块顶层只有纯导出，状态改动一律在 `M.install(deps)` 或调用期（对齐 [../mod/mod-package.md](../mod/mod-package.md) 的 `mod.pure-export`）。
- **I8 无时钟**：实体**不得产生任何时间字段**（沙箱里 `ygg.ticks()` 有 API 无推进者，恒 0；事件 seq 不注入 Lua）。生命周期先后由**宿主派发序**保证，包不做任何时序判断、也不发明时间戳。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `entity.define` | `(name, spec)` | `true` | 写（`pkg.entity.schemas*`）；同 spec 重复定义幂等且**零写**，异 spec 报 `YGG-PKG-ENTITY-SCHEMA` |
| `entity.schema` | `(name)` | `spec\|nil`（深副本） | 纯读 |
| `entity.spawn` | `(name, attrs?, tags?)` | `id` | 写 + 事件 `entity.created` |
| `entity.destroy` | `(id)` | `boolean`（首次 `true`，二次 `false`） | 写（内核真删：属性 + tag + 记录）+ 事件 `entity.destroyed`；缺失 id = `false` 不报错 |
| `entity.exists` | `(id)` | `boolean` | 纯读（一次内核存在性读，零 KV 读） |
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
| 路径基线（B-2，10⁴ 实体集） | 逐路径给数字、**不设阈值**（机差会把门禁打红） | `PERF entity.paths_*` 七行（spawn / destroy / get / set / get_set_pair / attrs8 / tags3 / ids_10k）；复算命令同上方；说明见 §9 |
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
- **真沙箱面不计入 `cases passed=`**：包能不能在**无 `inject`** 的裸沙箱里装载、`set` 的整/浮身份判据能不能不自相矛盾，由 `cargo test -p ygg-sandbox --test packages_sandbox` 的用例 ② 钉住（`2` 与 `2.0` 必须判为异值）；`cases passed=` 只数 Lua 用例（[README.md](README.md) §5「沙箱可装载面」）。

## 8. 设计依据

**为什么这样切。** 实体只做三件事：分配 id、装属性、发事件。这三件事在四个复现项目里逐字相同，差别只在属性**名字**与规则，所以名字进数据、规则进 mod。属性值域收窄到标量是刻意的：一旦允许嵌套表，`set` 的「变化才发事件」判定要写深比较，「按属性查找」要写路径解析，两处都会随时间漂移成不同的语义。

**反方案一：继承树**（生物 → 怪物 → Boss）——加一个「既是物品又能施放」的存在就要多重继承，且每加一层都要改包，直接违背「新增类型 = 加数据」。**反方案二：每个游戏自带实体系统**——代价可数：四份 `next_id`、四份序列化、四份悬垂引用处理，且两个游戏的同一属性可能一个存 number 一个存 string，跨游戏导入时才炸（Goal 反例 2 判这种自造基建为未完成）。

**实现纪律（沙箱方言，不回写就等着真实跑局炸）。** 沙箱 globals 里没有标准库的 `string` / `table` / `utf8` / `coroutine` 表（`sandbox.rs` 的白名单测试钉死 `string == nil` 等六项），而 `selftest` 跑在**普通 Lua** 上、这些表都在——所以任何「在 selftest 里绿、在沙箱里 nil-index 崩」的写法都测不出来。两条硬纪律：① 字符串操作只能走**方法语法**（`("x"):sub(1, 1)`，走字符串元表的 `__index`），`string.x` 与 `table.sort` 之类一律不可用，排序/拼接手写（`sort_names` / `join_lines`）；② 交付门禁带一条静态 grep（`grep -n 'string\.\|table\.\(sort\|concat\|insert\|remove\)' packages/libs/entity/*.lua` 必须 0 行，注释里也不写——否则它会变成噪声真阳性）。

**已交付（S9，2026-11-21）：`ygg.entity.exists` / `ygg.entity.destroy`。** 见 [../kernel/lua-api.md](../kernel/lua-api.md) §3.1 与本文 §1 / I5：`destroy` = 内核真删（属性 + tag + 记录一并回收，水位不动），存活真源 = 内核存在性（零 KV 键）。两条**后续**（都不在本需求边界，另立需求）：① 残留的 `rec.<id>.schema` 键归零——把 schema 搬进内核保留属性（名首字符 `.` 落在 schema 名文法 `^[A-Za-z_]…` 之外，包不可撞名），随真删自动回收；或补 `ygg.kv.remove(key)` 面。② **S9-b 旧档一次性 purge**：S9 前的存档里死亡实体的内核记录还在，升级后会被读成存活；迁移只能挂在**写路径**（读路径触发会撞 `modules.rs` 的 `ShadowBanned` 运行时影子表，把今天合法的「模块顶层 `pkg.entity.exists`」变成错误），≈12 行 + 版本标记键。内核 iter 仍未上宿主面——所以「收养出生前的存量实体」仍做不到。

**判据（①②③）。** ① 阻断——在 mod 里自己实现 id 分配或属性存储；在包里写属性语义（「生命归零算死」）；把表的形状偷偷塞进 `attrs`。② 症状——两个游戏各有一份 `next_id`；同一属性在两个游戏里类型不同导致导入失败；`destroy` 之后引用槽被包悄悄改写。③ 确认——本文件存在且被 [../README.md](../README.md) 索引；`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-entity` 的 `cases passed=` 含 18 条 `pkg.entity.*`；§5 五个指标有数字，异常则 file 级断言先红。

## 9. 下沉边界（P-K A1；逐函数去向与硬判据）

**为何有这一节。** 热路径下沉改变了「哪个函数跑在哪层」，而 API 面、事件与墓碑语义必须**一字不变**（I6）——这条约束只管**下沉轮**；S9 的 `destroy` 语义修订是另一件事（见 I5 / 9.3），改了就得改契约，不是静默改行为。这张表是二者的接缝：每个导出函数的去向都有理由与数字，改包前先看它，而不是「顺手也把它下沉了」。

### 9.1 逐函数去向

判定口径：**只下沉批量形态**（单条按 id 访问仍留 Lua——它仍可内部复用批式面实现 n=1，但 API 形状不变）。「现状」= 包侧 Lua + mock 宿主实测 ns/op（10⁴ 迭代 × 三批中位），数据源 `PERF entity.paths_*`（§5）。

| 导出（签名零变化） | 去向 | 理由 |
| --- | --- | --- |
| `install` / `VERSION` / `define` / `schema` | 留 Lua | 装配与 schema 层（Tier 2）；`define` 的「同 spec 幂等且零写」靠 KV 精确写点数断言 |
| `spawn` | **留 Lua**（批式面 `spawn_batch` 已交付，包内未采用——见 9.6） | 实测：n=1 的帧构造成本 > 省下的 18 次跨边界 |
| `get` / `set` / `has` / `exists` / `schema_of` / `count` | 留 Lua（n=1 可内部复用批式面） | 单值；语义大头是存活门 + 同值判定 + 事件 |
| `attrs` / `tags` | **留 Lua**（`attr_fetch` 已交付，包内未采用——见 9.6） | 实测：同上（n=1 不划算） |
| `ids` | **留 Lua**（`scan` 已交付；包内未采用——见 9.6） | 存活门每 id 一枚 KV 读，扫描省下的只是 `has_tag` 那一次 |
| `tag` / `untag` | 留 Lua（n=1；批式面 `tag_batch` 供将来批量调用方） | 单条仍是 1 次跨边界 + 1 条事件 |
| `destroy` | 留 Lua + 委托内核真删（`ygg.entity.destroy` 一次穿越） | 见 9.3 |

### 9.2 值域不下沉（硬结论）

`number` 是**任意有限 double**，且 `2` 与 `2.0` 是**两个值**（`is_integer` 参与同值判定与事件判定，`tests/packages_sandbox.rs` 用例②钉住）。所以属性值**不得**落到 Q32.32 定点列：`0.1 × 2³² = 429496729.6` 不可整表示（往返即变值），`2`/`2.0` 会塌成一个 i64。属性值只能落在 `Value::Int|Float|Bool|Str` 这条承载上；批式面只负责「把 n 次写合一次」，**不改值的表示**。

### 9.3 真删与存活位（S9 修订；写点已改）

S9 起 `destroy` 的可见行为是：**内核记录真删**（属性 + tag + 记录一起回收）→ `count` 减一 → 一条 `entity.destroyed`；`rec.<id>.schema` 留在原处，**恰好一枚 KV 键**（`pkg-entity-perf.lua` 的泄漏用例把它当断言）。**不再写 `alive.<id>`**（位退役，I5 真源 = 内核存在性）。
`EntityStore::remove` **不得**被换成「保留墓碑 + purge」：那会让每次死亡永久留一条记录（`iter` / `scan` / `canonical_snapshot` 各自都要补遮蔽，即 9.3 原本警示的两类可见性搬进内核）。`despawn_batch` 仍不交付——没有消费者（见 9.4）。

### 9.4 宿主面新增的批式键（`ygg.entity.*`，键集闭集）

五个：`spawn_batch` / `attr_apply` / `attr_fetch` / `tag_batch` / `scan`（形状见 [../kernel/lua-api.md](../kernel/lua-api.md) §3.1）。`despawn_batch` **不交付**——没有消费者就不该有面（S9 的 `destroy(id)` 已覆盖单条真删；批式真删等真正有批量死亡的消费者再议）。三条纪律：① **一个逐实体数据 getter 都不新增**（键集闭集断言在 `crates/ygg-sandbox/tests/host_contract.rs`，多一个即红；S9 的两个**控制面单标量键** `exists` / `destroy` 是例外且已入表，理由见 9.3：存在性位不搬运数据，批式化它按 9.7 实测必亏）；② 入参是 **Lua 数组**（铁律② 原文允许「bytes/数组进出」），这样 n=1 热路径不必付 pack 成本，测试夹具也不必写第二份编解码器；③ 新面一律**先校验后改**（整批要么全成、要么零改）。

### 9.5 判据（①②③）

① 阻断——想给本包加一条「更快的按 id 读」而绕开批式面，或把属性值搬进定点列；② 症状——`2` / `2.0` 在沙箱用例②里变成同值，或 `pkg-entity-perf.lua` 泄漏用例的「恰好 +100 键」变红；③ 确认——`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-entity` 的 `PERF entity.paths_*` 七行 + `cases passed=` 不回退。

### 9.6 实测裁决：批式面**已交付**，但包内委托**未采纳**（2026-11-21）

**结论先行**：五个批式键在 `ygg.entity` 上落地并有契约用例（13 键闭集 + 五面往返 + 三族错误码；A1 交付时 11，S9 的 `exists` / `destroy` 是后加的两键），
但 `pkg.entity` 内部**没有**改用它。理由是实测，不是口味。

**为什么**（同机同 10⁴ 实体集，`PERF entity.paths_*` 三跑中位）：

| 路径 | 委托前 | 委托后（走批式面） | 差 |
| --- | --- | --- | --- |
| `get`（n=1 `attr_fetch`） | 748 ns | 3 147 ns | **4.2× 更慢** |
| `set`（n=1 `attr_apply`） | 2 967 ns | 3 540 ns | 更慢 |
| `attrs`（8 键 `attr_fetch`） | 2 444 ns | 4 512 ns | 更慢 |
| `tags`（n=1 `attr_fetch`） | 1 596 ns | 3 289 ns | 更慢 |
| `ids`（`scan`） | 503 ns/实体 | 602 ns/实体 | 更慢 |
| `spawn`（`spawn_batch` n=1） | 12 656 ns | 15 310 ns | 更慢 |

**机理（可复算）**：Tier 2 侧构造一次批式帧（外层数组 + 行表 + key 数组，n=1 时 2–3 张表）
≈ 200–600 ns，而它换掉的跨边界只有 **44–57 ns/次**（`path_costs` 的 `probe.crossing.*`）。
n=1 时这笔账**必然亏**；n 大时（`ids` 的 10⁴ 个 id）本该赢，但 `ids` 的成本大头是**存活门**
——每个 id 一枚存活判定，而 S9 之前它是 `pkg.entity.alive.<id>` 的 KV 读（S9 起换成
`ygg.entity.exists` 的内核查，**这条数字已变**：见 §5 的 `paths_ids_10k`）。于是扫描省下的
只有 `has_tag` 那一次跨边界，构造成本反超。

**两个能翻盘的方向（都不在本需求边界内，需裁决）**：

1. **逐项原生面**（`get_attribute` 级批量 1 次穿越、按位参数，不构造帧）：可省 200–600 ns 的
   构造费，n=1 才有得赚。代价是它与「禁逐实体 getter」的批式铁律直接冲突（`sim-kernel` §0②），
   且要再开 4–6 个键。
2. **存活位搬进原生面**（`EntityStore` 上加 alive 位或原生 alive 位图），让 `ids` 的 10⁴ 次
   KV 读消失（这是唯一有数量级收益的一条）。代价：墓碑位从「KV 唯一真源」变成双写，
   直接动 I5 与泄漏用例的键数断言——必须由契约层面拍板。——**S9 已用更便宜的方式解了这条**：
   存活真源直接改成内核存在性（`exists`），既不新增 alive 位也不双写，KV 键反而从 2 枚降到 1 枚。

**判据（①②③）**：① 阻断——想靠「把 n=1 的调用包成批式帧」来提速；② 症状——`PERF entity.paths_get`
从 748 涨到 3 147 ns；③ 确认——上面六行数字 + `cargo test -p ygg-sandbox --test path_costs`
的穿越系数。

### 9.7 A 路实测（多键/多位姿数组面）：前提被否证（2026-11-21 第二轮）

裁决授权了「多键数组面不算逐实体 getter」并把它作为翻盘路 A。**先量成本，再决定要不要造键**——结论是这条路的收益前提不成立：

| 形状（同一裸 mlua、release、三轮中位） | 每 call 成本 | 与「逐项穿越」对比 |
| --- | --- | --- |
| 带 3 参标量调用（`(id, key, value)` 型） | **56.6 ns** | 逐项基线 |
| 返回两字段表（`{value=…}` 型） | **180.6 ns** | 已贵过 3 次标量调用 |
| **8 值进 + 8 值出（表形多键面）** | **485.3 ns** | 8 次标量穿越 ≈ 8×40 = **320 ns** ⇒ 表形**更贵** |
| **8 个标量多返回值** | **19.7 ns** | 唯一真正便宜的批形状 |

**为什么**：表形的成本不在「一次调用」，而在**每值 marshal 两次**（Lua 表 → Rust `Value`、Rust → Lua 表，各一次分配 + 一次类型转换，~60 ns/值）。逐项调用虽然穿 8 次边界，但每次只搬一个标量（~40 ns/值）。**n=1 时表形必亏，n=8 时也亏** —— 与「批式摊薄」的直觉相反，因为表不是零成本容器。

**所以 A 路的预期（`get` 750→150、`attrs` 2444→300）不可达**：`get` 的成本大头是**存活门**（S9 前 = `pkg.entity.alive.<id>` 的 KV 读 + `tostring`/拼接；S9 起 = 内核存在性读，拼接没了），不批式化就搬不走；`attrs` 唯一可行的便宜形状是**标量多返回值 + 注册行序**（19.7 ns 出 + 129 ns 原生读），但那需要「define 时把属性行序注册进原生面」的新面（本需求未授权，且它把 schema 顺序变成原生状态）。

**结论**：A 路在本需求内**不实现**（造了也是负收益的面）。真正可行的两条仍是：① 注册行序 + 多返回值（`attrs` 有 7–10× 空间）；② 存活位下沉（`ids`/`get` 有数量级空间，S21）。两条都要新契约。数字与命令：`cargo test --release -p ygg-sandbox --test path_costs -- --nocapture`。
