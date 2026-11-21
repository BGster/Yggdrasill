# Yggdrasill · sim-kernel — 原生模拟内核（三层引擎 Tier 1）

> **id** kernel/sim-kernel · **模块** ① ygg 内核（`crates/ygg-sim`） · **触发** 改模拟内核（tick / ECS 状态 / 快照 / 回放 / 确定性策略 / 热核面）之前
> **读不到代价** hard-refusal（确定性破坏 = 回放字节不一致 = agent 侧可复算性全失，属不可逆缺陷）

> **上游**：[../README.md](../README.md) · **相关**：[../packages/README.md](../packages/README.md)（Tier 2 壳纪律）· [state-machine.md](state-machine.md) · [lua-api.md](lua-api.md)（宿主面注册范式，同 fs 桥）

## 0. 终局形态：三层引擎（本文件是 Tier 1 的契约）

| 层 | 载体 | 职责 | 约束 |
| --- | --- | --- | --- |
| **Tier 1 原生模拟内核** | `crates/ygg-sim`（本文件） | tick 循环、ECS 列式状态、快照/回放/state hash、noise/DSP/几何/物理原语 | 无模拟天花板；确定性是结构不变量 |
| **Tier 2 沙箱规则层** | `packages/libs/**`（壳）+ mod（Lua） | 游戏规则、组合、剧情胶水 | **永久沙箱**：帽/燃料/无 IO/无时钟 |
| **Tier 3 内容与数据** | schema、数据文件、生成产物 | 不变的内容 | 纯数据 |

**判定三问**（逐能力裁决归属）：逐实体逐帧的热逻辑 → Tier 1；事件驱动的规则与组合 → Tier 2；不变的内容 → Tier 3。

**下沉铁律**（每个 Tier 2→Tier 1 下沉面必须全过）：① 契约先行（本文件或包契约先行改定再实现）；② **批式 API**——bytes/数组进出、单次调用承载整批，**禁止细粒度逐项 getter**（**铁律的本意是「帧编解码与值搬运随 n 线性增长」，不是「穿越本身贵」**：穿越实测 ~45–57 ns/次带参、~19 ns 零参；而**表形**数组面实测 **485 ns/8 值进出**，反而贵于 8 次逐项穿越（≈320 ns）——见 §5.1 的 `probe.crossing.*` 行。因此：**多键 / 多位姿的数组进出面合法且鼓励**（`attr_get_many(id, keys)` 式），它们不是「逐实体 getter」；真正的逐实体 getter 指「一次调用只服务一个实体的一条属性、且无批量入口」，那是把 n 次成本逐次付清。标量**多返回值**是最便宜的一侧（19.7 ns / 8 个），但它要求调用方静态知道元数——数据驱动的 schema 只能配「注册行序」才用得上）；③ before/after 表（同数据集/同机/三次取中位，原则二）；④ 八包 API 兼容——`pkg-selftest` 全量不回退。

**五不变量**：mod 永在笼内；数据契约先行；确定性是结构不是纪律；能力出口走证据触发（AGENTS.md §1.7）；agent 内省面（快照/回放/state hash/查询）是一等公民。

**设计依据（为什么直接立三层，不再等演进数据）**：模拟上限——Lua 解释域实时上限 ~10² 活跃实体@60fps，距 AAA 级模拟（10⁴⁺/帧）差 2 个数量级，语言不可弥补，层级可；确定性回放是 agent 游戏的核心卖点，只能做成结构不能靠纪律；参照 Factorio（原生引擎+Lua mod+确定性联机）同款分工已被大规模验证。反方案：全 Lua 包=上限不足（保留为 Tier 2）；八包重写为 Rust=杀掉笼内免编译生态（拒绝）。

## 1. 边界（这个内核不做什么）

- 不做渲染/表现——viewer 投影域，永远在内核之外。
- 不内嵌任何游戏语义（生命/伤害/捕捉率/难度）——那是 Tier 2。
- 不自带 IO 策略——落盘经宿主 fs 桥/产物面（同 fs 契约）。
- v1 不预造物理/寻路引擎——随 P3-4/P4 用例需求立项（防投机性建造）。

## 2. 数据形状

| 对象 | 形状 | 说明 |
| --- | --- | --- |
| `SimState` | `{ tick: u64, world: ArchetypeStore, resources: KV<u64> }` | 模拟态全量；浮点运算禁入模拟态（I2：位串列仅搬运不算术） |
| 组件 | 列式 SoA，按 archetype 分桶；组件 schema 与 Tier 2 属性袋同源形状 | 数据形状单源：包契约即 schema 来源 |
| `Snapshot` | 确定性编码全量字节（复用 codec 编码序） | restore 后 state hash 必须与原运行一致 |
| `ReplayLog` | `{ seed, inputs: [(tick, input)] }` | 输入日志；回放 = restore + 逐 tick 重放 |

**「复用 codec 编码序」= 复用排序规范，不是复用文本容器**（本句是该行的判准，改快照容器前先读）。上表 `Snapshot` 行的「编码序」指 codec 契约里那四条**序**的性质——键/组件名按字节序、整数标量与位串定宽 LE、无浮点运算、版本前置——而不是「快照是一段确定性 JSON 文本」。Tier 1 快照是**二进制列帧**（`magic | fmt_version | flags | tick | next_entity | 组件名表 | resources | archetype 段 | 尾 digest`），理由是可算的：10⁵ 实体 8 属性转 JSON 文本约 12 MB，`serde_json` 编码 ~150–300 MB/s ⇒ 40–80 ms **仅编码**，再加一次哈希即逼近 §5 的 100 ms 线；同期二进制列拷贝 ≤10 ms。且文本容器要把「数值的十进制文本形态」变成契约的一部分（`-0.0` / `1e3` / 尾随零），那是给确定性凭空加一条待测边界。

**JSON 文本容器归宿主面，不归这里**（P-K A2）：`pkg.codec` 的 `encode`/`decode` 已下沉到 `crates/ygg-sandbox/src/codec_face.rs` 的 `ygg.codec.*` 面（契约 [lua-api.md §3.1.4](lua-api.md)），它复用的是上一条的**序**，容器仍是 JSON 文本（存档兼容面）。`ygg-sim` 保持零 mlua、零浮点运算 token（位串搬运面 `bits.rs` 豁免，见 I2）、二进制帧不变——两侧的序由同一份 fixture 钉住（`crates/ygg-sim/fixtures/canonical-order.json`，两个消费者各断言一次）。

**尾 digest 就是 state hash**：帧末 8 字节小端 = `fnv1a64(帧去尾 digest)` = `hash()` 的返回值。于是 `restore(b)` 的校验与 `hash()` 的取值是**同一个数、同一次计算**——不存在「restore 通过了但 hash 不同」这种要第三种解释的状态。

**组件形状：Tier 2 属性袋的类型词汇 + 命名投影**（`required` / `default` 不下沉的裁决理由同段末）：

| Tier 2（[../packages/entity.md](../packages/entity.md) §2） | Tier 1（`crates/ygg-sim`） | 存储列 | 边界规则 |
| --- | --- | --- | --- |
| `type = "number"` | `AttrType::Number { frac_bits }`（缺省 32） | `Column::I64` | 内核只保证 `i64` 与位宽声明；Q32.32 是**跨面约定**，`frac_bits` 只被 Lua 面换算助手消费 |
| `number`（位串承载，R1′ 宿主面词汇 `f64bits`） | `AttrType::F64Bits` | `Column::F64Bits`（u64 位串 8B/值 LE） | 写入即归一（±0→+0、任意 NaN→单一 quiet NaN），读回非 canonical 即 `YGG-SIM-FRAME`；整/浮身份（`2` vs `2.0`）不归内核，随调用面 vtype 元数据走 |
| `type = "string"` | `AttrType::Str` | `Column::Bytes`（`off` + `buf`） | UTF-8 字节序原样，内核不做规范化 |
| `type = "boolean"` | `AttrType::Bool` | `Column::U8` | 只允许 0/1，其余值 restore 即 `YGG-SIM-FRAME` |
| `type = "ref"` | `AttrType::Ref` | `Column::U64` | `0` = 无引用（实体 id 从 1 起）；悬垂 ref 不自动修（谁引用谁是游戏语义） |
| `tags.<t>`（无值标记） | `AttrType::Tag` | 无列（只进 archetype key） | |
| `required` / `default` | **不下沉** | — | 存在性/缺省校验属 Tier 2 schema 层；内核不做业务校验（§1「不内嵌游戏语义」） |

**`frac_bits` 的作用域（诚实声明）**：内核**不解释小数位**——它只存 `i64`。`frac_bits` 是 schema 元数据，今天是**声明面**（供 Lua 面换算助手与将来按面声明使用），不是运算面；不要以为内核会按 `frac_bits = 16` 去做 Q16.16 运算。F64Bits 同样不解释值语义，内核只认 canonical 位串（归一规则见 `crates/ygg-sim/src/bits.rs`）。

**行序与删除语义**：archetype 内行是 **dense 插入序**，删除一律 `swap_remove`（末行填洞）并同步 `entity_index`；组件**注册后永不移除**（Registry 只增），`restore` 按**组件名升序**重排内部 id ⇒ **内部 id 不是持久事实**（快照与 hash 只认名称与行序）。这条是回放对「换行策略」耦合的唯一定义处：改 despawn 策略即改逐 tick hash。

**restore 不改既有内部 id**：快照按名称排序重建注册表，但 `restore` 对**已经登记过**的名字保持原 id（只对快照里多出来的组件按名称升序追加）。原因是系统闭包捕获的是注册时的 `ComponentId`——若 restore 换掉 id 空间，系统就会写到错的列上，而这种错误只在「存档 → 读档 → 继续跑」时显形。于是两条性质同时成立：`restore(b).snapshot() == b`（快照只认名称）与「系统不需要知道任何 id 变了」。


## 3. 不变量

- **I1 确定性**：同 seed + 同 ReplayLog → 逐 tick 状态字节一致（跨 tick 数、跨进程）。
- **I2 定点政策与位串承载**：模拟**数值运算**默认 i64 定点（Q32.32 或按面声明）；位串列（`AttrType::F64Bits`）承载的 IEEE 754 double **仅搬运不算术**——内核对位串只做存储、拷贝、规范化（±0 与 NaN 载荷归一）与字节级 hash，禁止算术/超越运算；模拟态仍禁浮点运算；`x86_64`/`aarch64` 语义由整数运算与位搬运保证（跨平台矩阵见 §5）。
- **I3 回放闭环**：`restore(snapshot) + replay(log)` → 每 tick `state hash` 与原始运行逐值一致。
- **I4 容量线**：10⁴ 活跃实体单 tick ≤ 16.6ms（目标线；实测表进 §5，未达即回炉）。
- **I5 隔离**：模拟态不读时钟/外部随机源——随机数仅由 seed 流派生（与 procgen rng 契约同源单份）。
- **I6 兼容**：任何下沉不得改变 Tier 2 包的对外 API——`pkg-selftest` 全量用例是回归钉。

## 4. API 面（批式铁律的落地形状）

| 面 | 签名形状 | 副作用 |
| --- | --- | --- |
| `sim.step(dt)` | 推进一 tick（系统注册制，系统列表是数据） | 变更模拟态 |
| `sim.snapshot()` → bytes / `sim.restore(bytes)` | 全量存取 | 纯 |
| `sim.hash()` → u64 | 规范化序列化序的哈希（FNV-1a 64；**Rust 面 u64，Lua 面 = 16 位小写 hex 字符串**） | 纯 |
| bulk entity ops | `spawn_batch/despawn_batch/insert_batch`（批量字节进出） | 变更模拟态 |
| noise/DSP/几何原语面 | `eval(buffer_in) -> buffer_out` 批式 | 纯 |

**hash 的跨面形状（不是强度妥协）**：Lua 5.4 整数是 **i64**，u64 上半区不可表，直转会变成负数——于是 Lua 面统一返回**16 位小写十六进制字符串**（既有先例：`pkg.procgen` 的「64 位一律十六进制」口径，K5）。Rust 面保持 `u64` 裸值（库侧不为面形状付字符串代价）。两侧是同一个数的两种呈现，**不是两种算法**：全仓额外哈希仅此一份 FNV-1a 64 实现，向量表与 `pkg.procgen.rng.fnv64` 双向断言。

**noise/DSP/几何行不在首个需求范围内**：该行是契约承诺的**后续面**，验收归 Goal 排期总表（建议 S3：各面 buffer 吞吐数字 + 八包不回退）。首个需求只交付 `step / snapshot / restore / hash / bulk ops / replay`；读到本行时不得据此宣称 noise 面已存在。

宿主注册范式与 fs 桥同款：库侧 `driver.rs` 注入（可改），CLI 激活触点在 `egg.rs`（宿主面——**改动走用户授权账，与 U1 六行同笔**）。**激活触点已接（S-1，2026-11-21，charter GOAL AUTH「允许」）**：开局与热载两构造点调 `activate_host_faces`（`Sim::new(固定常量种子 20261121)` → `set_sim`；失败语义 = 开局 fail-closed、热载沿用旧内核），`ygg.sim.*` 十键在真 CLI 已可用；确定性种子来自固定常量，per-run seed 属后续需求。

## 5. 性能预算与验收

| 指标 | 阈值 | 测法 |
| --- | --- | --- |
| tick 10⁴ 活跃实体 | ≤ 16.6ms | 三次取中位，基准集入册 |
| snapshot/restore 10⁵ 实体 | ≤ 100ms | 同上 |
| 回放一致 | 逐 tick hash 一致 | replay 用例（字节对比） |
| Tier 2 兼容 | `pkg-selftest` 199/0 不回退（S-25 在树基线 2026-11-21；复算命令见 `packages/entity.md` §5） | 每次下沉后全量 |
| 跨平台 | x86_64 ↔ aarch64 digest 一致 | 当前仅单机（macOS/arm64），结构性论证为整数语义；**实测已排期：Goal 排期总表 S2**，落地前所有跨平台声明引用该排期 |

### 5.1 实测回填表（数字只落本表，不回写上方阈值）

阈值是契约线，**实测不回流去改阈值**（未达线即回炉，见 I4）。每行必须带可复算命令；跨平台行引用 Goal 排期总表 S2，落地前不得声称已测。

| 指标 | 阈值 | 实测（三轮取中位） | 方法（可复算） | 备注 |
| --- | --- | --- | --- | --- |
| tick 10⁴ 活跃实体 | ≤ 16.6 ms | **0.553 ms**（三轮 0.590 / 0.553 / 0.551；p95 0.588–0.603，max 0.757） | `cargo test --release -p ygg-sim --test perf_tick -- --nocapture` | workload = 4 archetype / 3 系统 / 200 tick（20 warmup） |
| snapshot 10⁵ 实体 | ≤ 100 ms | **8.351 ms**（帧 7 200 130 B） | 同上 | 1 archetype × 8 attrs |
| restore 10⁵ 实体 | ≤ 100 ms | **16.633 ms** | 同上 | 含尾 digest 校验 |
| 回放一致 | 逐 tick hash 全等 | **matched=150/150（快照续跑）、200/200（创世重跑）；first_mismatch=None** | `cargo test -p ygg-sim --test replay_tick_hash` | 含 1 条篡改负例（改 dt +1 ulp 必在该 tick 报失配） |
| 穿越成本（批式铁律②的实测依据） | — | **0 参 19.5 / 带参 44.6–56.7 / 返回两字段表 178.1 ns/次**（三轮中位） | `cargo test --release -p ygg-sandbox --test path_costs -- --nocapture` | 裸 mlua，release；`probe.crossing.*` 行 |
| 跨进程 digest | parent == child | **parent_digest = child_digest = `0c6199352032440b`（60 行 trace）** | `cargo test -p ygg-sim --test replay_cross_process` | `current_exe` 子进程 + 临时文件 |
| tick 10⁴（F64Bits 档，R1′） | ≤ 16.6 ms（I4 同线） | **0.527 ms**（三轮 0.592 / 0.523 / 0.527；p95 0.813–0.531，max 0.980–0.553） | `cargo test --release -p ygg-sim --test perf_tick perf_tick_10k_f64bits -- --nocapture` | workload 同 Number 档形状；负载为位串搬运（读→pack∘unpack 归一→写），≈I64 档（同 8B/值线性搬运），2026-11-21 实测 |
| spawn 10⁴×8attr（Rust 侧，R1′ 双档） | — | **I64 档 212.5 ns/entity**（259.7/209.7/212.5）vs **F64Bits 档 226.8 ns/entity**（217.0/226.8/227.1） | `cargo test --release -p ygg-sim --test perf_spawn_10k_8attrs_both_kinds -- --nocapture` | A1 §1.7 范式（release 三轮中位）；F64Bits +6.7%，位串直入列不落 codec 文本；2026-11-21 实测 |
| snapshot/restore 10⁵（F64Bits 档，R1′） | ≤ 100 ms 双向 | **8.951 / 17.367 ms**（帧 7 200 130 B，与 I64 档**同字节数**；I64 档同轮 8.785 / 17.070，差 +1.9% / +1.7%） | `cargo test --release -p ygg-sim --test perf_snapshot_restore_100k_f64bits -- --nocapture` | hash 路径对照口径：Tier 2 浮点进状态面必走 codec 文本（≥30 MiB/s 在案），位串 8B LE 直进帧 + FNV 折叠；2026-11-21 实测 |

口径：macOS / arm64、`--release`（`[profile.release.package.ygg-sim] opt-level = 3` 单点覆盖，根 `opt-level="s"` 是 wasm 体积面）、三轮各新建 Sim、每轮取逐 tick 中位、再取三轮中位。**实测比 §5.3 的预判（1–3 ms）更快**，余量 30×；不因此下调阈值——阈值是契约线，不是目标值。

**run-to-run 波动（诚实声明）**：同一条命令在交付验收时再跑一次给出 tick 0.515 ms（三轮 0.526 / 0.515 / 0.513）、snapshot 7.440 ms、restore 15.360 ms——与上表相差 5–8%，属同一台机的常规抖动，**不影响任何阈值判定**。表格记首次入册值，验收另行记录原始输出（两轮都在交付回执里）。


## 6. 用点计划

| 复现项目 | 用点 | 该内核提供什么 | 阶段 |
| --- | --- | --- | --- |
| Saltline（P3-4） | 实时帧内碰撞/投影/弹道 | 批式 step + 几何原语面 | P3-4 帧率不达标即启用 |
| superagent game（P4） | 人物/道具/任务状态机 | bulk 状态面 + 快照/回放 | P4 |
| CDDA/DCSS（P3） | 大实体量回合制 | bulk 面性能余量 | P3（Lua 壳先行） |

## 7. 自测与验收

- replay 字节一致用例（restore + replay 逐 tick hash）；
- 定点政策静态检查（模拟 crate 禁浮点字面量与浮点算术；`src/bits.rs` 位串搬运面按白名单豁免——determinism_scan R1′，同 assetgen 扫描范式）；
- 八包 API 兼容回归（`pkg-selftest` 全量）；
- 字面三件套全绿；
- 每个下沉面一表：before/after（同数据集/同机/三次中位）。

## 8. 设计依据

- **为什么三层**：模拟上限差 2 个数量级只能靠层级补；确定性回放是 agent 游戏核心卖点，必须结构化；Factorio 先例（原生引擎+Lua mod+确定性）已验证该分工。
- **为什么 v1 收窄**：tick/ECS 存储/快照/hash 是所有后续面的公共底座，先立底座；物理/寻路等重系统随用例立项——防投机性引擎建造。
- **反方案**：全 Lua 包（上限不足，保留为 Tier 2 角色）；八包重写 Rust（杀掉笼内免编译生态，拒绝）；等 P3 数据再定（被用户裁决否决——架构直接承诺，数字用于排序下沉面而非决定是否下沉）。
- **为什么名称键控（内部 id 不进快照/hash）**：`ComponentId` 是注册序的副产物，而注册序是宿主装载序的副产物——把 id 写进存档等于把「装载顺序」变成持久契约，跨进程/跨版本必然分叉。改成名称键控后，同一世界在两种注册顺序下给出**逐字节相同**的快照与 hash（`snapshot_roundtrip::id_order_independent` 实测：内部 id 0 vs 4，字节与 hash 全同）。代价是一次按名称排序的编码（每个 archetype 段几十字节的元数据），相对列载荷可忽略。
- **为什么结构操作延迟到 tick 边界**：系统迭代列的同时增删行，会让「这一帧里到底有哪些行」依赖遍历进度——同一份代码在 `Vec` 增长/搬移下给出不同结果。延迟到边界 FIFO 应用后，系统看到的行集与行序在整帧内稳定，且换进/换出顺序可复算（回放的逐 tick hash 依赖它）。反方案（就地增删 + 索引失效重查）不是更快，只是把确定性换成了「跑两次看看」。
- **为什么 hash 每次真编码一遍快照**：等式 `state_hash == fnv1a64(快照字节去尾 digest)` 只有在真的编码时才可执行。另写快路径（遍历列直接算 hash）会让两条路径在「加了一个字段」时静默分叉，症状是「存档能读、回放对不上」——最难查的一类故障。代价在 §5.1：10⁴ 实体 tick 仍只用 0.553 ms，远低于 16.6 ms 预算。
- **为什么位串列（R1′，2026-11-21）**：50× 线的实体态承载需要内核侧的 8B 定宽浮点路径——Tier 2 浮点今日进状态面必走 codec 文本（format_g，≥30 MiB/s 在案），逐实体逐帧的文本往返是数量级错配。位串列把浮点确定性从「环境纪律」改写成**结构事实**：内核对位串只做存储、拷贝、归一（±0→+0、任意 NaN 载荷→单一 quiet NaN）与字节级 hash，浮点环境差异（FTZ/DAZ、x87 扩展精度）在结构上不可达——`src/bits.rs` 是唯一 `f64` 豁免面且被扫描断言零算术运算符（I2 修宪 + R1′）。数字锚点：A1 §5.3/§5.5（sim 列式 128 ns/entity < 255 ns 门槛，EntityStore 616 > 255 ⇒ 位串列是 50× 线唯一已知承载）；授权锚点：用户 2026-11-21「不留旧账」+ A1 §9.3 六处改定批复。实测对照（§5.1）：tick 0.527 ms ≈ I64 档 0.526 ms，帧字节两档同 7 200 130 B。不声称端到端 74×（那是 Lua 批量面 slice 的数字）；不声称对原始 Lua 值无损（无损是对 v1 所存 Q32.32 值而言）。
- **①②③ 判据**（每个下沉面）：① 被阻动作=该面性能预算不达或需宿主资源；② 症状=对比表数字；③ 确认=digest/帧率/吞吐实测。
