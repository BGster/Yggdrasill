---
type: "Reference"
title: "entity — 实体与属性袋（机制层）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-fef267a23f756637e121d083
    resource: repo://crates/ygg-sandbox/src/sandbox.rs
  - id: openwiki-source-c08b8fe59395edc6994992a8
    resource: repo://crates/ygg-sandbox/tests/host_contract/entity_face.rs
  - id: openwiki-source-7e104178dc65c6625b2c2050
    resource: repo://packages/libs/entity/init.lua
  - id: openwiki-source-7e16e66d75bcd267f6045913
    resource: repo://packages/libs/entity/pkg.json
  - id: openwiki-source-6776c13720946b846d3d318d
    resource: repo://packages/libs/entity/query.lua
  - id: openwiki-source-3b2de8ff9e0387bf510f3d72
    resource: repo://packages/libs/entity/record.lua
  - id: openwiki-source-24ab2ec424a9cd3184e7cb72
    resource: repo://packages/libs/entity/test/pkg-entity-perf.lua
  - id: openwiki-source-a6cd8e6b971d63ec060cba5b
    resource: repo://packages/mods/pkg-selftest/tests/pkg-entity-smoke.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# entity — 实体与属性袋（机制层）

> **id** packages/entity · **模块** ⑤ packages · 通用能力层 · **触发** 改实体 / 属性袋形状、索引或生命周期事件之前
> **读不到代价** silent-drift（属性袋形状在两个游戏里各自演化，序列化与索引静默分叉，导入时才炸）
> **replaces**：DESIGN §4 entity（搬运基线 `3a2536f`）
> **上游**：[README.md](README.md) · **相关**：[codec.md](codec.md)（持久化形状）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.entity.*`）· [../kernel/state-surface.md](../kernel/state-surface.md)（KV 契约 §2.8）

## 1. 边界（这个包不做什么）

- **不定义属性语义**：生命归零算死、攻击力怎么算、抗性怎么叠加——是 mod 的规则。
- **不落盘**：持久化格式归 [codec.md](codec.md)；本包只声明 KV 前缀与形状。
- **不复制内核实体面**：内核实有 **8 面**——`spawn` / `get_attribute` / `set_attribute` / `add_tag` / `remove_tag` / `has_tag` 六面，加 S9 交付的**控制面两键** `exists(id)` / `destroy(id)`（`crates/ygg-sandbox/src/host_api/` 的 `entity_table` 这八次 `raw_set`；`crates/ygg-core/src/entity/` 的 `EntityStore` 里 `remove_attribute` / `iter` 仍在 store 层、**不在宿主面上**——它们没有消费者）。本包在它之上加 **schema 校验、生命周期事件、引用槽**，不实现第二套 id 分配，不存第二份属性，**也不镜像存活位**。
- **`destroy` 是内核真删，不是墓碑降级**（S9）：`ygg.entity.destroy(id)` → `EntityStore::remove`，属性 + tag + 记录**一并回收**；分配水位（`next_id`）不动 ⇒ I1「id 永不复用」是**分配**性质，与本体是否永存无关。存活真源因此只有一个 = 内核存在性（I5），本包不再写任何 `alive` 位。**旧档边界（不兼容，不写迁移）**：本改动前产生的、含 `pkg.entity` 状态的存档里，死亡实体的内核记录还在，升级后会被读成存活——须重开一局；一次性 purge 登记在 §9。
- **不提供查询语言、不建继承树**：没有 `where` 式 DSL，按 tag / schema 的集合查询是唯一查询面（`entity.query`，见 §5）；类型 = 已注册的属性 schema，类型差异是数据。

## 2. 数据形状

**对外的 spec 形状**（`define(name, spec)` 的入参，也是 `schema(name)` 的返回）：
`{ attrs = { <name> = { type = "number"|"string"|"boolean"|"ref", required = <bool>, default = <v> } }, tags = { <name> = { required = <bool> } } }`。
`type = "ref"` 的值是正整数 id，与 number 分开声明是为了让悬垂引用可被断言发现。

**存储形状**由两个宿主事实决定，不是风格选择：① 属性值与 tag 有现成内核面（`set_attribute` / `add_tag`，随快照往返、按 id O(1)）；② `ygg.kv` 只收**标量**（`boolean` / `number` / `string`）——没有表、没有删除、没有遍历（`crates/ygg-core/src/kv.rs` 的标量值域 + `host_api` 的 `value_from_lua` 四型直通）。于是「`attrs` 存一张表」「`index` 存一张表」在宿主上**不可实现**，改成下表：

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

**为什么 `attrs` 进内核而不是 KV。** 放进 KV 要给每个属性一个 `pkg.entity.rec.<id>.a.<name>` 键（也成立），但那会把 `destroy` 的残留从「内核一条记录」变成「KV 里十几个墓碑键」，并丢掉内核已有的按 id O(1) 与快照持久化。代价是**属性名必须可枚举**——恰好由 schema 的 `attrs` 名表提供：I3 已把「未声明键」判为错误，所以「属性名集合 = schema 声明集」是**已经隐含**的不变量。

**`seq` 的语义**：分配 id 的是内核（只增不减、不复用），本包只记**水位镜像**：`spawn` 后 `seq = max(seq, id)`。它服务两件事：`ids` 的扫描上界，与 codec 的「导入保 id」判定。

**冷启动边界（诚实声明）。** `pkg.entity.seq` 缺失时（包刚接入一个已有存档的局，或 mod 直接调过 `ygg.entity.spawn()`）本包**无法收养**它出生前的实体：宿主面没有「列 id」调用（`EntityStore::iter` 在，但没上去），扫不到就是扫不到。本包只索引自己 `spawn` 过的实体。

### 事件载荷（`ygg.emit` 只收标量，形状必须显式定形）

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `entity.created` / `entity.destroyed` | 实体 `id`（number） | 与 `spawn` 的返回值一致 |
| `entity.attr_changed` | `"<id>\|<key>\|<tostring(value)>"` | `wire` 是事件文本，不是持久化字节 |
| `entity.tagged` / `entity.untagged` | `"<id>\|<tag>"` | |

## 3. 不变量

- **I1 单调分配**：`spawn` 返回的 id 严格大于此前所有返回过的 id；`destroy` 后不复用。**分配者是内核**，本包只把水位镜像到 `pkg.entity.seq`（`max(seq, id)`）。S9 的真删**不动水位**（`EntityStore::remove` 只动实体表与 `revision`）。
- **I2 属性值域窄化**：值只能是 `boolean` / 有限 `number` / `string`，`ref` 值是正整数 id（`2.5` / `"3"` / `0` / `-1` 全拒）。表、函数、thread 写入即报 `YGG-PKG-ENTITY-ATTR`。理由：属性袋一旦允许嵌套表，`set` 的「变化才发事件」要写深比较，「按属性查找」要写路径解析，两处都会漂成不同语义。
- **I3 schema 门**：`spawn(schema, …)` 与该 schema 的 `set(id, k, v)` 只接受 spec 声明过的**属性键**，`tag` / `untag` 只接受声明过的 **tag**；未声明即报错（属性 / 值域 → `YGG-PKG-ENTITY-ATTR`，tag / spec 面 → `YGG-PKG-ENTITY-SCHEMA`）。新增属性或 tag = 改数据里的 spec，不改包。
  - tag 为何也收窄：它仍是 I3 的拒绝面（拼错 tag 名 = 错，不是无操作）。
- **I4 悬垂引用不是错误**：`destroy(id)` 不遍历、不清理其它实体的引用槽；读悬垂 ref 仍得 id，`exists(ref) == false`。清理归 mod 的语义（「召唤物死亡，主人失去它」是规则）。
- **I5 内核存在性是唯一真源**（S9 修订）：`ygg.entity.exists(id)` 是「这个 id 能不能被看到」的唯一判据（底层 = `EntityStore::contains`，一次 BTreeMap 查），**每一读口都过它**（`get` / `attrs` / `has` / `tags` / `schema_of` / `ids` / `count`），所以死掉的实体在对外面上与不存在**不可区分**。本包**不再写 `pkg.entity.alive.<id>`**：一旦 `ygg.entity.destroy` 公开在宿主面上，任何 mod 都能直呼它，KV 位就会与内核裂成两套可见性（`ids` 吐幽灵、`count` 偏高、`set` 招 core 的 `not found`）——真源只能一个。
  - **残留恰好一枚键**：`rec.<id>.schema` 留在原处（`ygg.kv` 无删除面）。所以一次 `destroy` **恰好**新增 1 枚 KV 键——`pkg-entity-perf.lua` 的泄漏用例把这个「恰好」当断言（S9 前是 2 枚）。
  - **旧档边界**：S9 前的存档里死亡实体的内核记录还在（当时用 `alive=false` 遮蔽），升级后会被读成存活——**不兼容、不写迁移**，重开一局。
- **I6 生命周期事件恰好一次**：每次 `spawn` 恰好一次 `entity.created`；每次 `destroy` 恰好一次 `entity.destroyed`（二次 `destroy` 返回 `false` 且零事件）；`set` 只在值真的变化时发一次 `entity.attr_changed`（同值写入不发，`math.type` 参与比较）。
- **I7 顶层纯度**：包模块顶层只有纯导出，状态改动一律在 `M.install(deps)` 或调用期。
- **I8 无时钟**：实体**不得产生任何时间字段**（`ygg.ticks()` 有 API 无推进者，恒 0；事件 seq 不注入 Lua）。生命周期先后由**宿主派发序**保证。
- **I9 查询序确定性**（S-24）：无 `order` ⇒ **id 升序**；有 `order` ⇒ `(值, id)` 升序——值同以 id 兜底（`asc` / `desc` 都不反转），缺属性排最后（与方向无关），跨族按固定族序 `Num < Float < Bool < Str`。索引是**剪枝结构**：候选集出表后仍逐条跑谓词，所以索引路径与全扫路径**逐值逐序相等**（差分用例 `index_path_equals_scan_path`）；增量维护的索引 ≡ 从零重建（`index_incremental_equals_rebuild`）。
- **I10 查询是只读面**（S-24）：`entity.query` 不改任何状态（零写入、零事件）；`schema` 后过滤只读 `rec.<id>.schema`。内核属性索引是**派生结构**（`#[serde(skip)]` + 手写 `Clone` / `PartialEq` 丢索引）：不进存档字节、不进规范快照，`SAVE_FORMAT_VERSION` 不动。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `entity.define` | `(name, spec)` | `true` | 写（`pkg.entity.schemas*`）；同 spec 重复定义幂等且**零写**，异 spec 报 `YGG-PKG-ENTITY-SCHEMA` |
| `entity.schema` | `(name)` | `spec\|nil`（深副本） | 纯读 |
| `entity.spawn` | `(name, attrs?, tags?)` | `id` | 写 + 事件 `entity.created` |
| `entity.destroy` | `(id)` | `boolean`（首次 `true`，二次 `false`） | 写（内核真删）+ 事件 `entity.destroyed`；缺失 id = `false` 不报错 |
| `entity.exists` | `(id)` | `boolean` | 纯读（一次内核存在性读，零 KV 读）；非数字 / 非整数 id 一律 `false`（不让宿主面的 u64 参数转换把探询变成抛错） |
| `entity.get` | `(id, key)` | `value\|nil` | 纯读 |
| `entity.set` | `(id, key, value)` | `boolean`（值是否变化） | 写 + 事件（变化才发） |
| `entity.attrs` | `(id)` | table（浅副本） | 纯读 |
| `entity.tag` / `entity.untag` | `(id, tag)` | `boolean`（是否真的变化） | 写 + 事件 `entity.tagged` / `entity.untagged` |
| `entity.has` | `(id, tag)` | `boolean` | 纯读 |
| `entity.tags` | `(id)` | `{tag, …}`（升序） | 纯读；**导出侧所需**（宿主无列 tag 面） |
| `entity.schema_of` | `(id)` | `schema\|nil` | 纯读（死之或不存在 = `nil`） |
| `entity.ids` / `entity.count` | `(tag?, schema?)` | `{id, …}`（升序）/ number | 纯读；O(n)，n = 存活实体数 |
| `entity.query` | `(selector)` | `{count, ids}` 或 `{count, rows}` | 纯读（一次内核穿越，零写入零事件）；`schema` 键是包面专有（§5） |

副作用类别：`纯读` / `写` / `写 + 事件`。`写` 类在模块顶层调用即被 `YGG-MOD-EFFECT` 拒绝。

## 5. 性能预算

**口径声明**：实测值产在 **selftest（普通 Lua）**，因为沙箱内既无 `collectgarbage` 也无 `os.clock`。所以内存那行是**包侧 Lua 堆上界**（含 mock KV 与 mock 实体表的开销），不是宿主 Rust 堆。来源：每行都有对应用例打印 `PERF entity.<指标>=<值> <单位>`，三次取中位。复算：`--filter pkg-entity`。

| 指标 | 阈值 | 本包实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 实测 **0.44 ms**（目标 `≤ 1ms` 达成；S9 复测 0.4394）。测法：`loadfile` + `install` 各 100 次取每轮均值，`os.clock` 计时——P0 写的「空 run 起局 vs 含本包起局的墙钟差」信号（≤ 1ms）**低于整条链的启动抖动**，测不出东西。 |
| 实体按 id 查找 / 更新 | `O(1)` | 实测 **1022.5 ns/op**（S9 后；S9 前同机 1259.2）；N = 1 / 100 / 10000 三档档间比 **1.38×–1.51×**，断言 **≤ 2.0×**。**强约束的落点**：`get` / `set` = 一次内核存在性读 + `rec.<id>.schema` 读 + 内核属性面，无遍历。载体限制：这条跑在 mock 上，能抓到的是**包侧**引入的遍历，不是内核面的复杂度。 |
| 序列化吞吐 | 附基准集与耗时 | 不适用：承包给 [codec.md](codec.md) §5，基准集与口径只在那里定义一次。 |
| 10k 实体常驻内存 | 给出数字 | 实测 **5557.0 KiB**（包侧 Lua 堆上界；S9 起每实体少一枚 `alive.<id>` 键，同口径比 S9 前 6637.1 低 1080 KiB）+ **1155576 B**（键名 + 值字节和）。口径：10k × 8 属性 + 3 tag，`collectgarbage("count")` 前后差 ÷ 1024。P0 写的 `≤ 2 MiB` 是沙箱内 Rust 侧口径，与这个上界口径**不可比**；沙箱硬帽是 64 MiB。 |
| 路径基线（B-2，10⁴ 实体集） | 逐路径给数字、**不设阈值**（机差会把门禁打红） | `PERF entity.paths_*` 七行（spawn / destroy / get / set / get_set_pair / attrs8 / tags3 / ids_10k） |
| 查询（S-24） | 分档给数字；**不设阈值**（同 `paths_*` 纪律） | **真沙箱端到端**（`crates/ygg-sandbox/tests/path_costs.rs` 的 `probe.query_face.*`，`--release` 三跑取中位）：`ids_10k` **0.333 ms** / `where2_10k` **0.241 ms** / `indexed_10k` **0.067 ms** / `scan_same_10k` **0.159 ms**。**原生侧**：全扫 **37 µs** / 两谓词全扫 **164 µs** / 单属性区间走索引 **42 µs**。**包侧**：转发 **350 ns/call**、schema 后过滤 10⁴ 条 **1.18 ms/call**。数组填充系数 `probe.crossing.array_fill_10k_ns` = **8.3 ns/元素**。 |
| 泄漏检测 | 100 轮不增长 | `spawn` + `destroy` × 100 后：`count` / `#ids` 回起点、`seq` **恰好** +100（id 不复用）、**内核残留回零**（S9 新轴）、KV 键数 **恰好** +100（每轮一枚 `rec.<id>.schema`），内存 △**0.071%**（断言 ≤ 0.5%）。「键数不变」在无删除面的 KV 上**不可达**，所以断言改成「**恰好等于** destroy 次数」，同时排掉「每轮多写一个计数键」这类慢性漏。 |

**阈值为何是 2.0（两次都明写理由）。** 契约原写「≤ 1.2×」，但实测三跑档间比 1.07 / 1.09 / 1.18，另有一次 1.4× **假红**——噪声源是 `os.clock` 粒度与「全量跑前面已堆到 12 MB 量级」的 GC 脉冲。处置：① 内层轮次提到 5000 × 3 批取中位；② 阈值提到 1.5×。S9 后再抬到 **2.0×**：S9 前 1.47 / 1.42 / 1.38，S9 后 1.43 / 1.48 / 1.51 / 1.43，逐档**绝对耗时**降了约 15%（存活门从 KV 读换内核存在性读），spread 没动 ⇒ 是机器噪声不是遍历；真遍历是 10³–10⁴×，一眼可辨。

### 5.1 查询面（S-24）

**为什么有它**：① 「列活物」在包侧是**每实体一次存活门跨界**（`ids` 的 10⁴ 次穿越，真沙箱实测 234 ns/实体 ⇒ 2.34 ms）；② **属性谓词根本没有面**——mod 只能先 `ids()` 全量、再逐实体 `get_attribute`（440.9 ns/键），「hp ≤ 100 的敌人」最省写法约 11 ms。S-24 把两件事一起解：一次跨界、内核进程内选、**可走索引时走索引**。

| selector 键 | 语义 |
| --- | --- |
| `tag` | 先于 `where` 的最便宜过滤 |
| `where` | **属性名 → 谓词**映射（同键多条 = 隐式 AND）；字符串态 `"op.operand"` 糖 + 结构态 `{op=…, value=…}` 精确逃生口；op 族 `eq/neq/gt/gte/lt/lte/in/like/ilike/is` + `not.` 前缀 |
| `select` | 缺省 = **裸 id 数组**；非空 = 行表（行恒带 `id`，缺属性不设键） |
| `order` | `field` / `field.asc` / `field.desc`；值同 ⇒ id 升序兜底；缺属性排最后 |
| `schema` | **包面专有**（内核面不认识它） |

**五条语义裁断**（都在 core 的 `crates/ygg-core/src/entity/`，无 VM 可单测）：① `is` 管存在性、其余 op 对缺属性一律假，且 `not.` 不翻——否则 `neq` 与 `not.eq` 会分叉出两套缺失语义；② 跨族比较是假不是错（含 `neq`），扫描中报错会产出半截结果，整浮同族（`eq 2` 命中 `2.0`）；③ `like` 只认四形态（无 `*` / `P*` / `*S` / `*C*`，字面段非空）——零正则、零回溯、零 ReDoS 面，`ilike` 只做 ASCII 折叠；④ 不分页、不 JOIN——跨包关联属 mod 语义；⑤ `id` 是保留伪属性（真属性名 `id` 因此在 `where` / `order` 里不可寻址），换来 `query` 对 `scan(lo, hi, tag)` 的语义覆盖。

**索引（惰性首建 + 写点增量维护）**：按键有序索引，值 → id 列表，四段 `Num < Float(位序化 u64) < Bool < Str`，桶内 id 升序。可走索引的条件只有一条：**所有谓词落在同一属性、每条都可索引（`eq`/`gt`/`gte`/`lt`/`lte`/`in`）、且无 `not.`**；出现 `neq` / `like` / `ilike` / `is` 或 **≥2 个属性**一律回全扫。三条依据：① 索引进 `EntityStore` 而不是宿主侧——宿主 6 个写键各要记得调一次增量补丁，漏一处 = **静默错答**（查询引擎最坏的故障形态）；② 不「标脏 + 下次重建」——重建 10⁴ 实体 ≈ 0.3 ms，**比不建索引的全扫（0.15 ms）还贵 2×**；③ 索引是剪枝结构不是权威——候选集出表后仍逐条跑谓词，所以「索引 ≡ 全扫」是构造性的。

**确定性（by construction，五条）**：`CompareOp` 里没有算术臂（浮点不可结合性无从发生）；id 序遍历（`BTreeMap::values()`）；浮点只 `partial_cmp`（`-0.0` 在写入口规范化）；单线程（`Rc ⇒ !Send` + mlua 未启 `send`）；索引 (值, id) 序 + 增量 ≡ 重建。**不做运行期断言**：负向 trait bound 无法表达，写了就是不可证伪的测试。

**载体选择**：语义（选择器 / 比较矩阵 / 序遍历 / 走索引判定）全在 `crates/ygg-core/src/entity/`——无 VM 可单测，语义不随宿主重构漂；宿主 `crates/ygg-sandbox/src/host_api/` 只做**文法解码 + 两阶段材质化**（借用纪律同 `attr_fetch`：阶段 1 持借用零 Lua 调用，阶段 2 只建表）。错误码**不新增族**：文法 / 形状错 `YGG-ENTITY-FRAME`、值域错 `YGG-ENTITY-VALUE`。

**包面加什么、不加什么**：加**两样**——`schema` 门（`YGG-PKG-ENTITY-SCHEMA`；schema 是 KV 里的 `rec.<id>.schema`，**不是内核状态**，放进内核面 = 第二个 schema 注册表，违 I3）与未声明键门 / 结构态类型门（`YGG-PKG-ENTITY-ATTR`，复用 `value_ok`）。一句话：**包面把类型错变成错误，内核面把类型错变成假**。三档成本：无 `schema` ⇒ 1 次穿越透传；有 `schema` ⇒ 1 次穿越 + R 次 `kv.get` 比对；再带 `select` ⇒ 行表原地滤行。包**不二次解析**谓词文本——那会造出第二份文法真源。

**before/after（同机同数据集，10⁴ 实体）**：

| 档 | before | after（端到端 = 真沙箱 + 一次跨界） | 比 |
| --- | --- | --- | --- |
| 列活物 | **2.34 ms**（真沙箱 S9 `PERF entity.paths_ids_10k`） | **0.333 ms**（`probe.query_face.ids_10k`） | **7.0×** |
| 属性筛 2 谓词 | **~11 ms**（构成式：`ids` 2.34 + 10⁴×2 次 `get_attribute` 8.8） | **0.241 ms**（`probe.query_face.where2_10k`） | **~46×** |
| 同结果集：全扫 vs 走索引 | 0.159 ms（`scan_same_10k`，1000 行结果） | **0.067 ms**（`indexed_10k`） | **2.4×** |

**口径纪律（不许只报最好档）**：7.0× 是最诚实那档（估算用 Lua 侧 8.3 ns/元素，而 Rust→Lua 每元素材质化实测 ≈30 ns）；属性筛档的 before 是**构成式**（两个在案数字相加），after 是一次端到端测量，故标注「构成式对照」；全命中 + 投影 2 键档不重测（v1 已按 mlua 行表 marshal 固定价 183.9 ns/行如实分档为约 4.8×，那是 mlua 的价）。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 生物 / 道具实例化 | schema 声明属性袋（HP 上限、类型、稀有度）+ 组合而非继承 | P3 |
| CDDA | JSON 定义 → 实体实例 | `define` + `spawn`：把 `ygg.data` 读到的数据条目直接实例化，零手写类型代码 | P3 |
| DCSS | 玩家 / 怪物 / 地面物品 | 同一套属性袋承载三种「看着不同」的存在（死物同构） | P3 |
| Saltline | 玩家、拾取物、关卡标记 | 同上（第三个到第四个用点，验证「实体 = 死物同构」） | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.entity.`；perf 用例名前缀 `pkg.entity.perf-`。
- **断言面**：I1–I10 每条至少一个用例；I2 / I3 的拒绝路径必须断言「报错而非静默忽略」（`expect_error("YGG-PKG-ENTITY-ATTR", ...)`）。
- **计数**：`--filter pkg-entity` → **29** 条（2026-11-26 实测；= 26 条 `pkg.entity.*` 行为用例 + `pkg-entity-paths` 的 2 条 + S9 的 `destroy-reclaims-kernel-record`；S-24 的 `query` 占其中 8 条）；pkg-selftest 全量 **199**。生产 Mod（agent-seed）不声明包，计数保持 **79/0**。
  - **perf 文件不计数**：`pkg-entity-perf.lua` 用 file 级断言（`error` ⇒ `ok=0`），不进 `cases passed=`。
  - **`--filter` 的硬约束**：它同时对 mod 侧与包侧文件过滤，且「一处都不匹配」会以「没有可执行的测试文件」直接 exit 1。
  - **禁止宽松断言**：`assert(true)`、只断类型不断值、只断「没报错」一律作废。
- **真沙箱面不计入 `cases passed=`**：包能否在**无 `inject`** 的裸沙箱里装载、`set` 的整 / 浮身份判据不自相矛盾，由 `cargo test -p ygg-sandbox --test packages_sandbox` 的用例 ② 钉住（`2` 与 `2.0` 必须判为异值）。

## 8. 设计依据

**为什么这样切。** 实体只做三件事：分配 id、装属性、发事件。这三件事在四个复现项目里逐字相同，差别只在属性**名字**与规则，所以名字进数据、规则进 mod。属性值域收窄到标量是刻意的：一旦允许嵌套表，`set` 的「变化才发事件」判定要写深比较，「按属性查找」要写路径解析，两处都会随时间漂移成不同的语义。

**反方案一：继承树**（生物 → 怪物 → Boss）——加一个「既是物品又能施放」的存在就要多重继承，且每加一层都要改包，直接违背「新增类型 = 加数据」。**反方案二：每个游戏自带实体系统**——代价可数：四份 `next_id`、四份序列化、四份悬垂引用处理，且两个游戏的同一属性可能一个存 number 一个存 string，跨游戏导入时才炸。

**沙箱方言纪律**：沙箱 globals 里没有 `string` / `table` / `utf8` / `coroutine`，而 `selftest` 跑在**普通 Lua** 上——「selftest 绿、沙箱 nil-index 崩」的写法测不出来。两条硬纪律：① 字符串操作只能走**方法语法**，`string.x` 与 `table.sort` 一律不可用，排序 / 拼接手写（`sort_names` / `join_lines`）；② 交付门禁带静态 grep（`grep -n 'string\.\|table\.\(sort\|concat\|insert\|remove\)' packages/libs/entity/*.lua` 必须 0 行，注释里也不写）。

## 9. 下沉边界与后续

**只下沉批量形态**（单条按 id 访问仍留 Lua）：「现状」= 包侧 Lua + mock 宿主实测 ns/op（10⁴ 迭代 × 三批中位），数据源 `PERF entity.paths_*`。

| 导出（签名零变化） | 去向 | 理由 |
| --- | --- | --- |
| `install` / `VERSION` / `define` / `schema` | 留 Lua | 装配与 schema 层；`define` 的「同 spec 幂等且零写」靠 KV 精确写点数断言 |
| `spawn` | **留 Lua**（`spawn_batch` 已交付，包内未采用——见下） | 实测：n=1 的帧构造成本 > 省下的跨边界 |
| `get` / `set` / `has` / `exists` / `schema_of` / `count` | 留 Lua（n=1 可内部复用批式面） | 单值；语义大头是存活门 + 同值判定 + 事件 |
| `attrs` / `tags` / `ids` | **留 Lua**（`attr_fetch` / `scan` 已交付，包内未采用） | 实测：n=1 不划算 |
| `tag` / `untag` | 留 Lua（批式面 `tag_batch` 供将来批量调用方） | 单条仍是 1 次跨边界 + 1 条事件 |
| `destroy` | 留 Lua + 委托内核真删（一次穿越） | 见 I5 |

**值域不下沉（硬结论）**：`number` 是**任意有限 double**，且 `2` 与 `2.0` 是**两个值**（`is_integer` 参与同值判定与事件判定，`packages_sandbox.rs` 用例②钉住）。所以属性值**不得**落到 Q32.32 定点列：`0.1 × 2³²` 不可整表示，`2`/`2.0` 会塌成一个 i64。属性值只能落在 `Value::Int|Float|Bool|Str`；批式面只负责「把 n 次写合一次」，**不改值的表示**。

**批式面已交付但包内委托未采纳（2026-11-21 实测裁决）**：五个批式键（`spawn_batch` / `attr_apply` / `attr_fetch` / `tag_batch` / `scan`）在 `ygg.entity` 上落地并有契约用例（键集闭集断言在 `crates/ygg-sandbox/tests/host_contract/entity_face.rs`），但 `pkg.entity` 内部没有改用它。理由是同机同 10⁴ 实体集三跑中位：`get` 748 → 3 147 ns、`set` 2 967 → 3 540、`attrs` 2 444 → 4 512、`tags` 1 596 → 3 289、`ids` 503 → 602 ns/实体、`spawn` 12 656 → 15 310 ns——**n=1 全亏**。机理可复算：构造一次批式帧（外层数组 + 行表 + key 数组）≈ 200–600 ns，而它换掉的跨边界只有 **44–57 ns/次**。**多键表形面同样不划算**（同一裸 mlua release 三轮中位）：3 参标量调用 **56.6 ns**、返回两字段表 **180.6 ns**、8 值进 8 值出表形 **485.3 ns**（> 8 次标量穿越 320 ns）、**8 个标量多返回值 19.7 ns**——表不是零成本容器，每值 marshal 两次 ≈120 ns。所以唯一便宜的批形状是「标量多返回值 + 注册行序」，它需要新契约，本需求未授权。

**S-21 正式关闭（`query ⊃ scan`）**：S21 的内容是「给内核加存活位、与 KV 双写」，动机是 `ids()` 每实体一次存活门。`entity.query({})` 把动机**整体消掉**：`EntityStore::iter()` 就是活物集（S9 真删后表里没有死物），列活物 = 一次迭代 + 一次数组填充 + 一次跨界。真源仍唯一（内核表），本面**不可能**与 `exists` 分歧；双写方案反而引入分歧面。`where = { id = "gte.10" }` 让 `query` 成为 `scan(lo, hi, tag)` 的语义超集。

**已交付（S9）：`ygg.entity.exists` / `ygg.entity.destroy`。** 两条**后续**（都不在本需求边界，另立需求）：① 残留的 `rec.<id>.schema` 键归零——把 schema 搬进内核保留属性（名首字符 `.` 落在 schema 名文法之外，包不可撞名），随真删自动回收；或补 `ygg.kv.remove(key)` 面。② **S9-b 旧档一次性 purge**：迁移只能挂在**写路径**（读路径触发会撞 `modules.rs` 的 `ShadowBanned` 运行时影子表），≈12 行 + 版本标记键。

**判据（①②③）**。① 阻断——在 mod 里自己实现 id 分配或属性存储；在包里写属性语义；把表的形状偷偷塞进 `attrs`；用「全量 ids + 逐实体属性读」筛选择集。② 症状——两个游戏各有一份 `next_id`；同一属性在两游戏里类型不同导致导入失败；`destroy` 之后引用槽被包悄悄改写；`paths_get` 从 748 涨到 3 147 ns；`index_path_equals_scan_path` 差分变红。③ 确认——`--filter pkg-entity` 的 `cases passed=29`、`PERF entity.paths_*` 七行、`probe.query_face.*` 四行有数字、`cargo test --workspace` 不回退。

**模块化拆分（S-25）**：内核 `crates/ygg-core/src/entity/` 由单文件 1928 行（800 上限的 2.4×）拆为 `entity/` 七文件——`mod.rs`（装配 + `EntityStore` + 手写 `Clone`/`PartialEq`/`Default` + `AttrIndex` + 全量 re-export）+ `value` / `store` / `compare` / `selector` / `index` 五子系统 + `tests.rs`（26 测整块随迁）；纯搬移，非空行多集守恒、平铺 pub 路径逐字节不变。Lua 侧 `init.lua` 1032 行（1.3×）拆为 `init`（装配转口，18 键不变）+ `base`（KV 键唯一出处 / 错误码 / `host()`）+ `schema`（spec 编译 / 比较 / 落盘 / `value_ok`）+ `record`（存活判定 / 计数 / 17 API）+ `query`（S-24 两道门），无环 `base ← schema ← record ← query`。拆分动机只有一个：单文件超限使 schema / record / query 边界不可读。

维护责任 = 改 `packages/libs/entity/**` 或 `crates/ygg-core/src/entity/**` 的执行者；触发 = 任一相关 commit；检查 = `--filter pkg-entity` 计数 + `path_costs` 探针 + `packages_sandbox` 用例②。
