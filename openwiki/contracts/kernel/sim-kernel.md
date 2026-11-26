---
type: "Reference"
title: "原生模拟内核契约（engine Tier 1 · `crates/ygg-sim`）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-6a509e19fb830c77216e7ba9
    resource: repo://crates/ygg-sim/fixtures/canonical-order.json
  - id: openwiki-source-69ac4babee0699279fa53b28
    resource: repo://crates/ygg-sim/fixtures/fnv64-vectors.json
  - id: openwiki-source-d0af2cac42efa01feac13735
    resource: repo://crates/ygg-sim/src/bits.rs
  - id: openwiki-source-86c13e4901257c152b5f1bfe
    resource: repo://crates/ygg-sim/src/fixed.rs
  - id: openwiki-source-40bf7934333e8b316826ca7d
    resource: repo://crates/ygg-sim/src/hash.rs
  - id: openwiki-source-06a44f5a944119bb5b73eaf6
    resource: repo://crates/ygg-sim/src/sim.rs
  - id: openwiki-source-b888e3dc95ca0a07465ab295
    resource: repo://crates/ygg-sim/src/snapshot.rs
  - id: openwiki-source-d2391d71ffa8516188253d73
    resource: repo://crates/ygg-sim/src/store.rs
  - id: openwiki-source-9cadef0bf6fe1b20c8572167
    resource: repo://crates/ygg-sim/tests/determinism_scan.rs
  - id: openwiki-source-3e913ec77081ce52d9f30e11
    resource: repo://crates/ygg-sim/tests/perf_tick.rs
  - id: openwiki-source-ec2190b196f1e5905d7e38c0
    resource: repo://crates/ygg-sim/tests/replay_cross_process.rs
  - id: openwiki-source-4d55badba2eaf9dcc23f4a9c
    resource: repo://crates/ygg-sim/tests/replay_tick_hash.rs
  - id: openwiki-source-2edec0c8c110c6b8b7951b0e
    resource: repo://crates/ygg-sim/tests/snapshot_roundtrip.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 原生模拟内核契约（engine Tier 1 · `crates/ygg-sim`）

> **id** kernel/sim-kernel · **模块** ① ygg 内核（`crates/ygg-sim`） · **触发** 改模拟内核之前（tick / ECS 状态 / 快照 / 回放 / 确定性策略 / 热核面）
> **读不到代价** hard-refusal（确定性破坏 = 回放字节不一致 = agent 侧可复算性全失，属不可逆缺陷）
> **replaces**：DESIGN §5（L891–L1179）（搬运基线 `3a2536f`）
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[state-machine.md](state-machine.md) · [lua-api.md](lua-api.md)（宿主面注册范式，同 fs 桥）· [packages/README.md](../packages/README.md)（Tier 2 壳纪律）

## 0. 三层引擎与归属裁决

| 层 | 载体 | 职责 | 约束 |
| --- | --- | --- | --- |
| **Tier 1 原生模拟内核** | `crates/ygg-sim`（本页） | tick 循环、ECS 列式状态、快照 / 回放 / state hash、noise / DSP / 几何 / 物理原语 | 无模拟天花板；确定性是结构不变量 |
| **Tier 2 沙箱规则层** | `packages/libs/**`（壳）+ mod（Lua） | 游戏规则、组合、剧情胶水 | 永久沙箱：帽 / 无 IO / 无时钟 |
| **Tier 3 内容与数据** | schema、数据文件、生成产物 | 不变的内容 | 纯数据 |

**判定三问**：逐实体逐帧的热逻辑 → Tier 1；事件驱动的规则与组合 → Tier 2；不变的内容 → Tier 3。

**下沉铁律**（每个 Tier 2→Tier 1 下沉面必须全过）：① 契约先行（本页或包契约先改定再实现）；② **批式 API**——bytes / 数组进出、单次调用承载整批，**禁止细粒度逐项 getter**；③ before/after 表（同数据集 / 同机 / 三次取中位）；④ 八包 API 兼容——`pkg-selftest` 全量不回退。

**批式铁律的本意是「帧编解码与值搬运随 n 线性增长」，不是「穿越本身贵」**：穿越实测 ~19.5 ns（零参）/ 44.6–56.7 ns（带参）/ 178.1 ns（返回两字段表），而**表形**数组面实测 485 ns / 8 值进出，反而贵于 8 次逐项穿越（≈320 ns）。因此**多键 / 多位姿的数组进出面合法且鼓励**（`attr_get_many(id, keys)` 式）——它们不是「逐实体 getter」。真正的逐实体 getter 指「一次调用只服务一个实体的一条属性、且无批量入口」，那是把 n 次成本逐次付清。标量**多返回值**最便宜（19.7 ns / 8 个），但要求调用方静态知道元数。

**五不变量**：mod 永在笼内；数据契约先行；确定性是结构不是纪律；能力出口走证据触发；agent 内省面（快照 / 回放 / state hash / 查询）是一等公民。

## 1. 边界（这个内核不做什么）

不做渲染 / 表现（viewer 投影域永远在内核之外）；不内嵌任何游戏语义（生命 / 伤害 / 捕捉率 / 难度，那是 Tier 2）；不自带 IO 策略（落盘经宿主 fs 桥 / 产物面）；v1 不预造物理 / 寻路引擎——随 **P3-4 / P4** 用例需求立项（防投机性建造）。

## 2. 数据形状

| 对象 | 形状 | 说明 |
| --- | --- | --- |
| `SimState` | `{ tick: u64, world: ArchetypeStore, resources: KV<u64> }` | 模拟态全量；**浮点运算禁入模拟态**（位串列仅搬运不算术） |
| 组件 | 列式 SoA，按 archetype 分桶；组件 schema 与 Tier 2 属性袋同源形状 | 数据形状单源：包契约即 schema 来源 |
| `Snapshot` | 确定性编码全量字节（复用 codec 的**序**） | restore 后 state hash 必须与原运行一致 |
| `ReplayLog` | `{ seed, inputs: [(tick, input)] }` | 回放 = restore + 逐 tick 重放 |

**「复用 codec 编码序」= 复用排序规范，不是复用文本容器**：指 codec 契约那四条**序**的性质（键 / 组件名按字节序、整数标量与位串定宽 LE、无浮点运算、版本前置），**不是**「快照是一段 JSON 文本」。Tier 1 快照是**二进制列帧**（`magic | fmt_version | flags | tick | next_entity | 组件名表 | resources | archetype 段 | 尾 digest`），理由可算：10⁵ 实体 8 属性转 JSON 约 12 MB、`serde_json` 编码 ~150–300 MB/s ⇒ 40–80 ms **仅编码**，再加一次哈希即逼近 100 ms 线；同期二进制列拷贝 ≤10 ms。文本容器还会把「数值的十进制文本形态」（`-0.0` / `1e3` / 尾随零）变成契约的一部分——给确定性凭空加一条待测边界。

**JSON 文本容器归宿主面，不归这里**：`pkg.codec` 的 `encode`/`decode` 已下沉到 `crates/ygg-sandbox/src/codec_face/` 的 `ygg.codec.*` 面，容器仍是 JSON 文本（存档兼容面）；`ygg-sim` 保持零 mlua、零浮点运算 token、二进制帧不变。两侧的序由同一份 fixture 钉住（`crates/ygg-sim/fixtures/canonical-order.json`，两个消费者各断言一次）。

**尾 digest 就是 state hash**：帧末 8 字节小端 = `fnv1a64(帧去尾 digest)` = `hash()` 的返回值。于是 `restore(b)` 的校验与 `hash()` 的取值是**同一个数、同一次计算**——不存在「restore 通过了但 hash 不同」这种要第三种解释的状态。

**组件形状：Tier 2 属性袋的类型词汇 + 命名投影**

| Tier 2 词汇（[packages/entity.md](../packages/entity.md)） | Tier 1（`crates/ygg-sim`） | 存储列 | 边界规则 |
| --- | --- | --- | --- |
| `number`（定点） | `AttrType::Number { frac_bits }`（缺省 32） | `Column::I64` | 内核只保证 `i64` 与位宽声明；Q32.32 是**跨面约定**，`frac_bits` 只被 Lua 面换算助手消费 |
| `number`（位串承载，`f64bits`） | `AttrType::F64Bits` | `Column::F64Bits`（u64 位串 8B/值 LE） | 写入即归一（±0→+0、任意 NaN→单一 quiet NaN），读回非 canonical 即 `YGG-SIM-FRAME`；整 / 浮身份（`2` vs `2.0`）不归内核，随调用面 vtype 元数据走 |
| `string` | `AttrType::Str` | `Column::Bytes`（`off` + `buf`） | UTF-8 字节序原样，内核不做规范化 |
| `boolean` | `AttrType::Bool` | `Column::U8` | 只允许 0/1，其余 restore 即 `YGG-SIM-FRAME` |
| `ref` | `AttrType::Ref` | `Column::U64` | `0` = 无引用（实体 id 从 1 起）；悬垂 ref 不自动修（谁引用谁是游戏语义） |
| `tags.<t>` | `AttrType::Tag` | 无列（只进 archetype key） | — |
| `required` / `default` | **不下沉** | — | 存在性 / 缺省校验属 Tier 2 schema 层，内核不做业务校验 |

**`frac_bits` 的作用域（诚实声明）**：内核**不解释小数位**，只存 `i64`；`frac_bits` 是 schema 元数据，今天是**声明面**（供 Lua 面换算助手与将来按面声明），不是运算面——不要以为内核会按 `frac_bits = 16` 做 Q16.16 运算。`F64Bits` 同样不解释值语义，只认 canonical 位串（归一规则见 `crates/ygg-sim/src/bits.rs`）。

**行序与删除语义**：archetype 内行是 **dense 插入序**，删除一律 `swap_remove`（末行填洞）并同步 `entity_index`；组件**注册后永不移除**（Registry 只增）；`restore` 按**组件名升序**重排内部 id ⇒ **内部 id 不是持久事实**（快照与 hash 只认名称与行序）。这条是回放对「换行策略」耦合的唯一定义处：改 despawn 策略即改逐 tick hash。

**`restore` 不改既有内部 id**：快照按名称排序重建注册表，但对**已经登记过**的名字保持原 id（只对快照里多出来的组件按名序追加）。原因是系统闭包捕获的是注册时的 `ComponentId`——若 restore 换掉 id 空间，系统就会写到错的列上，而这种错误只在「存档 → 读档 → 继续跑」时显形。于是 `restore(b).snapshot() == b`（快照只认名称）与「系统不需要知道任何 id 变了」同时成立。

## 3. 不变量

- **I1 确定性**：同 seed + 同 ReplayLog → 逐 tick 状态字节一致（跨 tick 数、跨进程）。
- **I2 定点政策与位串承载**：模拟**数值运算**默认 i64 定点（Q32.32 或按面声明）；`F64Bits` 承载的 IEEE 754 double **仅搬运不算术**——只做存储、拷贝、归一（±0 与 NaN 载荷）与字节级 hash，禁止算术 / 超越运算；模拟态仍禁浮点运算。
- **I3 回放闭环**：`restore(snapshot) + replay(log)` → 每 tick `state hash` 与原始运行逐值一致。
- **I4 容量线**：10⁴ 活跃实体单 tick ≤ 16.6 ms（目标线；实测进 §5，未达即回炉）。
- **I5 隔离**：模拟态不读时钟 / 外部随机源——随机只由 seed 流派生（与 procgen rng 契约同源单份）。
- **I6 兼容**：任何下沉不得改变 Tier 2 包的对外 API——`pkg-selftest` 全量用例是回归钉。

## 4. API 面与 hash 的跨面形状

| 面 | 签名形状 | 副作用 |
| --- | --- | --- |
| `sim.step(dt)` | 推进一 tick（系统注册制，系统列表是数据） | 变更模拟态 |
| `sim.snapshot()` → bytes / `sim.restore(bytes)` | 全量存取（先校验后改） | 纯 / 变更 |
| `sim.hash()` → u64 | 规范化序列化序的 FNV-1a 64；**Rust 面 u64，Lua 面 = 16 位小写 hex** | 纯 |
| bulk entity ops | `spawn_batch` / `despawn_batch` / `insert_batch`（批量字节进出） | 变更模拟态 |
| noise / DSP / 几何原语面 | `eval(buffer_in) -> buffer_out` 批式 | 纯 |

**hash 的跨面形状不是强度妥协**：Lua 5.4 整数是 i64，u64 上半区不可表、直转会变负——于是 Lua 面统一返回 16 位小写 hex（与 `pkg.procgen` 的「64 位一律十六进制」同口径），Rust 面保持 `u64` 裸值。两侧是同一个数的两种呈现，**不是两种算法**：全仓额外哈希仅此一份 FNV-1a 64 实现，向量表与 `pkg.procgen.rng.fnv64` 双向断言。

**宿主注册范式与 fs 桥同款**：库侧 `driver/`（`surface.rs`）注入（可改），CLI 激活触点在 `egg.rs`（宿主面——改动走用户授权账）。**激活触点已接（S-1，2026-11-21）**：开局与热载两构造点调 `activate_host_faces`（`Sim::new(固定常量种子 20261121)` → `set_sim`；失败语义 = 开局 fail-closed、热载沿用旧内核），`ygg.sim.*` 十键在真 CLI 已可用；per-run seed 属后续需求。

## 5. 性能预算与实测回填

**阈值是契约线，实测不回流去改阈值**（未达线即回炉）。每行必须带可复算命令。

| 指标 | 阈值 | 实测（三轮取中位） | 方法 |
| --- | --- | --- | --- |
| tick 10⁴ 活跃实体 | ≤ 16.6 ms | **0.553 ms**（0.590 / 0.553 / 0.551；p95 0.588–0.603） | `cargo test --release -p ygg-sim --test perf_tick -- --nocapture` |
| snapshot 10⁵ 实体 | ≤ 100 ms | **8.351 ms**（帧 7 200 130 B） | 同上 |
| restore 10⁵ 实体 | ≤ 100 ms | **16.633 ms**（含尾 digest 校验） | 同上 |
| 回放一致 | 逐 tick hash 全等 | **matched=150/150（快照续跑）、200/200（创世重跑），first_mismatch=None** | `cargo test -p ygg-sim --test replay_tick_hash` |
| 穿越成本 | — | 零参 19.5 / 带参 44.6–56.7 / 返回两字段表 178.1 ns | `cargo test --release -p ygg-sandbox --test path_costs -- --nocapture` |
| 跨进程 digest | parent == child | `ba8070ac5c67faa4`（60 行 trace） | `cargo test -p ygg-sim --test replay_cross_process` |
| tick 10⁴（F64Bits 档） | ≤ 16.6 ms | **0.527 ms** | `--test perf_tick perf_tick_10k_f64bits` |
| spawn 10⁴×8attr | — | I64 档 **212.5 ns/entity** vs F64Bits 档 **226.8 ns/entity**（+6.7%） | `--test perf_tick perf_spawn_10k_8attrs_both_kinds` |
| snapshot/restore 10⁵（F64Bits 档） | ≤ 100 ms 双向 | **8.951 / 17.367 ms**（同 7 200 130 B；I64 档 8.785 / 17.070，差 +1.9% / +1.7%） | `--test perf_tick perf_snapshot_restore_100k`（F64Bits 档在同一测内） |
| 跨平台三腿 digest | 三腿 × debug∧release 逐字节全等 | 已实测全等（same_process `c50030402fe00c70` / from_genesis `1e9199cad033e0cb` / cross_process `ba8070ac5c67faa4`） | `--test replay_tick_hash` ∧ `--test replay_cross_process` ∧ `YGG_PACKAGES_ROOT=packages/libs packages/mods/superagent/test/final-verify.sh`；腿 = linux/arm64 原生 + linux/amd64（Rosetta）+ macOS/arm64 裸机，rustc 1.99.1 同版 |
| Tier 2 兼容 | `pkg-selftest` 199/0 不回退 | 每次下沉后全量 | 复算命令见 [packages/entity.md](../packages/entity.md) §5 |

口径：macOS / arm64、`--release`（`[profile.release.package.ygg-sim] opt-level = 3` 单点覆盖，根 `opt-level = "s"` 是 wasm 体积面）、三轮各新建 Sim、逐 tick 中位再取三轮中位。实测比预判（1–3 ms）快，余量 30×；**不因此下调阈值**。**run-to-run 波动诚实声明**：同一条命令验收时再跑一次给 tick 0.515 / snapshot 7.440 / restore 15.360 ms，相差 5–8%，属同机常规抖动，不影响任何阈值判定；表格记首次入册值，验收另记原始输出。

**跨平台覆盖面的如实声明**：linux/amd64 腿是 Rosetta 翻译执行（本机无 x86 真机），覆盖面收窄为「模拟 x86_64」；该腿登记 3 条 serve SSE / IO 时序用例失败与 pkg-codec-perf 吞吐 2498–2528 < 3000 entities/s，属模拟层性能 / 时序边界，**不触碰哈希等价结论**——性能阈值本是机器相对量，矩阵只对「同源哈希」作断言。

## 6. 用点计划

| 复现项目 | 用点 | 内核提供什么 | 阶段 |
| --- | --- | --- | --- |
| Saltline | 实时帧内碰撞 / 投影 / 弹道 | 批式 step + 几何原语面 | P3-4 帧率不达标即启用 |
| superagent game | 人物 / 道具 / 任务状态机 | bulk 状态面 + 快照 / 回放 | P4 |
| CDDA / DCSS | 大实体量回合制 | bulk 面性能余量 | P3（Lua 壳先行） |

## 7. 自测与验收

replay 字节一致用例（restore + replay 逐 tick hash，含篡改负例：改 dt +1 ulp 必在该 tick 报失配）；定点政策静态检查（模拟 crate 禁浮点字面量与浮点算术，`src/bits.rs` 位串搬运面按白名单豁免）；八包 API 兼容回归（`pkg-selftest` 全量）；字面三件套全绿；每个下沉面一表 before/after。

## 8. 设计依据

- **为什么直接立三层**：Lua 解释域实时上限 ~10² 活跃实体@60fps，距 AAA 级（10⁴⁺/帧）差 2 个数量级，语言不可弥补、层级可；确定性回放是 agent 游戏核心卖点，只能做成结构；Factorio（原生引擎 + Lua mod + 确定性联机）同款分工已被大规模验证。反方案：全 Lua 包 = 上限不足（保留为 Tier 2）；八包重写 Rust = 杀掉笼内免编译生态（拒绝）。
- **为什么 v1 收窄**：tick / ECS 存储 / 快照 / hash 是所有后续面的公共底座，先立底座；物理 / 寻路等重系统随用例立项——防投机性引擎建造。
- **为什么名称键控（内部 id 不进快照 / hash）**：`ComponentId` 是注册序的副产物，注册序又是宿主装载序的副产物——把 id 写进存档等于把「装载顺序」变成持久契约，跨进程 / 跨版本必然分叉。名称键控后同一世界在两种注册顺序下给出逐字节相同的快照与 hash（`snapshot_roundtrip::id_order_independent` 实测：内部 id 0 vs 4，字节与 hash 全同）。代价是一次按名称排序的编码，相对列载荷可忽略。
- **为什么结构操作延迟到 tick 边界**：系统迭代列的同时增删行，会让「这一帧到底有哪些行」依赖遍历进度——同一份代码在 `Vec` 增长 / 搬移下给出不同结果。延迟到边界 FIFO 应用后，系统看到的行集与行序整帧稳定，且换进 / 换出顺序可复算（逐 tick hash 依赖它）。反方案（就地增删 + 索引失效重查）不是更快，只是把确定性换成「跑两次看看」。
- **为什么 hash 每次真编码一遍快照**：等式 `state_hash == fnv1a64(快照字节去尾 digest)` 只有在真的编码时才可执行。另写快路径（遍历列直接算 hash）会让两条路径在「加了一个字段」时静默分叉，症状是「存档能读、回放对不上」——最难查的一类故障。代价在 §5：10⁴ 实体 tick 仍只用 0.553 ms。
- **为什么位串列**：50× 线的实体态承载需要内核侧 8B 定宽浮点路径——Tier 2 浮点进状态面必走 codec 文本（≥30 MiB/s 在案），逐实体逐帧的文本往返是数量级错配。位串列把浮点确定性从「环境纪律」改写成**结构事实**：内核只做存储 / 拷贝 / 归一 / 字节级 hash，浮点环境差异（FTZ / DAZ、x87 扩展精度）在结构上不可达——`src/bits.rs` 是唯一 `f64` 豁免面且被扫描断言零算术运算符。**不声称**端到端 74×（那是 Lua 批量面 slice 的数字），也**不声称**对原始 Lua 值无损（无损是对 v1 所存 Q32.32 值而言）。
- **为什么 Docker 三腿**：跨平台确定性此前只有结构性论证（整数语义、种子构建期常量、位串零算术豁免面）——论证能证明「不该漂」，证明不了「没漂」：编译器后端在异架构上的代码生成（指令调度、FMA 合并、libm 内联）是论证覆盖不到的自由变量。三腿矩阵以约 10 分钟容器构建 + 每腿全量的成本把这个自由变量钉进实测：哈希逐字节全等 ⇒ 跨平台声明从论证级升实测级；若现差异，测试金字塔天然给出分层定位（引擎 replay / Lua 串哈希 KAT / mod ②′）。成本与隔离：双派生镜像 + 独立 target 卷，宿主检出零改动零污染。
- **驱动面 `KernelFacade` 的拆分（S-25 批六）**：`driver.rs` 由单文件 1682 行（800 行上限 2.1×）拆为 `driver/` 6 文件——`mod.rs` 装配（13 个全私有字段 + `apply_save` 跨面件 + `resource_error_to_kernel` 触面件 + 四出口 `pub use`）+ `error`（五稳定错误码 + `KernelError`）+ `build`（装配管线 + 8 构造器 + `from_parts` / `restore`）+ `dispatch`（信封命令 save / load / state / log + `cmd_event` 事件机）+ `surface`（状态字节 + vfs×6 + 挂载 / 工具 / sim 面）+ `tests.rs`（21 测整块随迁）。**纯搬移**：字段可见性只向下流动（子模块按 Rust 可见性子树直读私有字段，零 `pub(super)` / `pub(crate)` 放宽），非空行多集守恒、21 测逐名守恒、七门禁 before/after 逐项相等、外部 `ygg_sim::driver::*` 路径零 diff。**判据**：① 阻断——以「顺手改行为」的方式拆分，或为拆分放宽可见性；② 症状——门禁计数不再守恒；③ 确认——拆分前后各跑全部门禁并逐 binary 比对计数。
