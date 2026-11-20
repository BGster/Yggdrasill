# Yggdrasill · packages/procgen — 确定性种子、命名流与参数化生成器

> **id** packages/procgen · **模块** ⑤ packages · 通用能力层 · **触发** 改 PRNG 算法、流派生规则或生成器声明形状之前
> **读不到代价** irreversible（同一 seed 两次运行产物不同，复现与「素材字节一致」的验收同时失效，且无法事后修复已有产物）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [../kernel/lua-api.md](../kernel/lua-api.md)（沙箱六项硬上限与确定性种子）· [codec.md](codec.md)（产物落盘走它）

## 1. 边界（这个包不做什么）

- **不定义生成什么内容**：地图语义、房间用途、地形权重——数据（`{ algo, params }`）与 mod。
- **不做素材二进制**：纹理 / 音频 / 几何的字节输出是 P2.5 `assetgen` 的领地；本包只提供两者共享的 rng 契约。
- **不落盘**：产物是返回值；落盘经 [codec.md](codec.md) 或 fs 挂载桥，且必须由调用方先立事件（`AGENTS.md` §1）。
- **不渲染**：字符画 / 3D 网格的呈现归 mod。
- **不读全局熵**：不调 `math.random`、不读 `ygg.ticks`、不读 `os`（沙箱本来也没有 `os` / `io`）。
- **不自己写第二份确定性编码**：`procgen.hash` 的编码口径借 [codec.md](codec.md) I1（同一值两次 `encode` 逐字节相等），本包对 codec 的依赖**只在摘要面**（唯一的包依赖）。
- **不碰宿主面**：不写 KV、不发事件、不注册 handler——`procgen.install` 不需要 `ygg` 宿主句柄，一个纯函数包连 `host` 都不接。

沙箱的确定性由宿主保证（种子走 `luai_makeseed`，见 [../kernel/lua-api.md](../kernel/lua-api.md)），但那是**全局**种子：两个生成器共用会让彼此扰动。本包的命名流就是为把全局状态隔离成按用途独立的序列。

## 2. 数据形状

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `rng.state` | 4 × 32 位无符号整数 | 是 | 由 `(seed, stream)` 派生 | 调用方持有；不写 KV，随存档由 [codec.md](codec.md) 点名导出。`procgen.state(rng)` 的快照可直接再喂回 `procgen.rng(seed, stream?, state?)` |
| `seed` | string 或 number | 是 | — | 调用方给；包内不生成、不取时钟 |
| `stream` | string（命名流） | 否 | `"main"`（`procgen.run` 的缺省是**生成器名**） | 运行时 |
| `defs.<name>` | table `{ algo, params }` | 否 | `{}` | **不落 KV**（D4）：安装期 `install({ defs = ... })` 注入 + 运行期 `procgen.define` 的内存注册表。运行期 define 的 def **不跨存档**——要跨存档的生成器必须来自 pack 数据 |
| 产物 `algo` / `v` | string / 整数 | 是 | 由 `run` 附着 | 返回值的两个字段（`v` 是生成器契约版本，见 I6）；产物序列化随调用方 |

生成器产物形状由 `algo` 决定，包不统一：`algo` 是注册过的 Lua 函数名，`params` 是它自己的参数表。新增形状 = 加一条 `defs` 数据；新增算法 = 加一个 `algo` 实现并回写本契约。

**种子文本口径**：`seed` 为 string 时**原样**参与派生；为 number 时取其数字文本（整数 `"7"`）。所以整数 `7` 与字符串 `"7"` 落在**同一条流**——这是已声明性质，不是缺陷；要区分请换字符串（如 `"7a"`）。不提供浮点的 `%.17g` 口径：沙箱里 `string` 表不可达，而 `tostring` 在白名单内。

## 3. 不变量

- **I1 确定性**：同一 `(seed, stream, params)` 在同一 `procgen` 版本下产出同一序列，跨 run、跨机器成立。PRNG 为纯 Lua 的 32 位实现（xoshiro128\*\*），只依赖 Lua 5.4 的位运算符，不依赖宿主 PRNG。
- **I2 流隔离**：不同 `stream` 名的序列互不影响——每个流的状态由 `w[i] = mix64(fnv64(seed .. "/" .. stream .. "/" .. i))`（`i = 0..3`，四字全 0 用 `0x9E3779B9` 顶掉）独立派生，不共享任何全局计数器；插入新流不改变既有流的序列（用例 `stream-isolation`）。
- **I3 无外部熵**：包内不得出现 `math.random` / `math.randomseed` / `os.` / `io.` 字面量；静态用例逐文件扫。
- **I4 生成器纯函数化**：`procgen.run(name, seed, stream?)` 只返回产物表（自带 `algo` / `v`），不改 KV、不发事件、不碰宿主面；落盘与事件由调用方负责。运行期 `define` 只写内存注册表——所以 def 不跨存档（§2）。
- **I5 参数化而非分支**：生成器由 `{ algo, params }` 声明驱动；`algo` 内不得按「哪个游戏」分支——游戏差异全部经 `params` 进来。
- **I6 版本进入产物**：每个产物自带 `algo` 名与生成器契约版本 `v`，由 `run` 统一附着。`v` 是**单一常量**而不是每个 algo 一份：改任一生成器的产物形状就 +1，让所有 hash 一起变——这正是「换算法必然换哈希」要的效果（不会出现「算法变了但验收仍然通过」）；分开记反而给「漏改一个」留位置。
  - **不把版本塞进 `hash` 的入参**：产物自带版本时，`hash(产物)` 天然包含它；入参里再写一份就是第二处口径（改一处漏一处）。
- **I7 顶层纯度**：包模块顶层只有纯导出，`defs` 注册一律在 `M.install(deps)`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `procgen.install` | `(deps)` | 包表 | 写（安装期注册）：`deps = { codec?, defs? }`。`host` 本包**不需要**（不碰宿主面）；重复装幂等（算法同名覆盖、defs 同 spec 幂等） |
| `procgen.rng` | `(seed, stream?, state?)` | rng 句柄 | 纯（新流）。第三参是 `procgen.state` 的快照：给了它就用它，`seed` / `stream` 忽略；四字全 0 报 `-PARAM` |
| `rng.next32` | `(rng)` | number（32 位无符号） | 纯（推进状态）。**原语面**：`int` 的拒绝采样与 KAT 都要它，所以它是导出而不是内部实现 |
| `rng.int` | `(rng, lo, hi)` | number（闭区间） | 纯（推进状态）。拒绝采样（`limit = 2^32 - 2^32 % span`，无模偏）；`span == 1` 直接返回 `lo` 且**不取数**；`lo > hi` / `span > 2^32` 报 `-RANGE` |
| `rng.float` | `(rng)` | number `[0,1)` | 纯（推进状态）。`(next32 >> 8) * 2^-24`，24 位粒度 |
| `rng.pick` | `(rng, list)` | 元素 | 纯（推进状态）；空表 / 非数组报 `-PARAM` |
| `rng.weighted` | `(rng, weights)` | number（`1..#weights` 的下标） | 纯（推进状态）。加权抽样：`Σw` 后 `int(1, Σw)`，取首个累计 `≥ r` 的下标。**权重是非负整数的「相对份数」**：不做归一化、不收小数（需小数由调用方放大为整数——归一化必引浮点除法，跨机舍入会破 I1）；零权项合法但不可达；`Σw == 1` **不取数**（继承 `int` 的 `span == 1`）。`weights` 必须是 `1..n` 致密数字数组（洞的 `#` 边界语义不修，同 `pick`）。非表 / `#w < 1` / 负权 / 非整值 / `Σw == 0` 报 `-PARAM`；`Σw > 2^32` 报 `-RANGE` |
| `rng.shuffle` | `(rng, list)` | **新表**（Fisher–Yates） | 纯（推进状态）；不原地改入参 |
| `procgen.define` | `(name, spec)` | `true` | 写内存注册表（`defs`）；同 spec 幂等、异 spec 报 `-DEF`；**不落 KV** |
| `procgen.algo` | `(name, fn)` | `true` | 写；注册 `algo` 实现（`fn(params, rng)`），同名覆盖 |
| `procgen.run` | `(name, seed, stream?)` | 产物表（自带 `algo` / `v`） | 纯（不写不 emit）。`stream` 缺省 = **生成器名**；def 未定义报 `-DEF`、algo 未注册报 `-ALGO` |
| `procgen.hash` | `(value)` | string（16 位十六进制） | 纯；口径 = `fnv1a64(codec.encode(value))`。**变更检测**不是加密摘要 |
| `procgen.state` | `(rng)` | table（4 整数副本） | 纯；用于「存档后从同一位置继续生成」，改副本不影响句柄 |

句柄方法与模块函数是**同一个函数**：`rng:int(1, 6)` 与 `procgen` 模块上的 `rng.int(rng, 1, 6)` 等价。错误码面 `procgen.ERRORS` 恰好六个键（`SEED` / `RANGE` / `PARAM` / `ALGO` / `DEF` / `DEPS`），全部形如 `YGG-PKG-PROCGEN-*`——`rng.weighted` 的五个失败路径**全部落在既有 `-PARAM` / `-RANGE` 里**，不新增第七个码。

`rng.weighted` 与 `rng.pick` 的关系是**特例而非并列**：等权池上两者逐值相等，`rng.pick(list)` ≡ `list[rng.weighted(ones(#list))]`（用例 `pkg.procgen.weighted-equal-weight` 钉下标与句柄状态两侧）。所以加权抽样不是新取数面，是 `int` 的一个调用形状：它只调 `int`，不碰 `next32` 以外的东西，**不引入新的熵源，也不新增跨语言向量**（Tier 1 双实现的 pub 面只有 `next32`，见 [kernel/lua-api.md](../kernel/lua-api.md)）。

## 5. 性能预算

**口径声明（先读再读数字）**：下列实测值产在 **selftest 面**（普通 Lua），因为沙箱里既无 `os.clock` 也无 `collectgarbage`（`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死）。包本身不碰宿主面，所以计数替身只需要证明「没碰」（用例 `run-pure`）。复算：`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-procgen`（三次取中位；`print` 面就是本表口径）。

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`；**实测 0.848 ms**（清掉全部 `pkg.procgen*` 后 `require` + `install`（含四个自带算法注册、不含任何 `run`），5 次取中位，批内三次取中位；单次 0.760–0.919 ms；加 `rng.weighted` 前同口径为 0.813 ms）。打印 `perf.procgen.load_ms` |
| 实体按 id 查找 / 更新 | `O(1)` | **不适用**：本包不碰实体表；唯一的表访问是 `defs` 直取（`registry.defs[name]`），与实体总数无关是构造性的 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：产物序列化由 [codec.md](codec.md) §5 承包；本包的等价面是 §7 的**跨进程字节一致**（同 seed 两次独立进程的 canon 行 `cmp` 相等 + 宿主 sha256 相等），比吞吐更硬 |
| 10k 实体常驻内存 | 给出数字 | 不适用（无实体）。等价口径：一张 64×64 噪声产物 **实测 135.2 KiB**（目标 `≤ 256 KiB`，硬帽 `16 MiB`；`collectgarbage("count")` 差）。打印 `perf.procgen.map64_mem_kib` |
| 泄漏检测 | 100 轮不增长 | 确定性替身：同一 `(seed, stream)` 跑 100 轮 `run`，100 次 `hash` **全相等**（`leak_100_equal=1`）且内存差 **0.024%**（≤ 1%）。打印 `perf.procgen.leak_100_equal` / `perf.procgen.leak_gc_pct` |

另加两条本包专属数字。**取数吞吐** `≥ 100,000 次 rng.int/秒`，因为地图生成的热路径在这里：20 万次 `rng.int(0,255)`（含拒绝采样）**实测 750,497 次/秒**（三次取中位；单次 740,612–753,264），约 7.5× 余量。打印 `perf.procgen.rng_int_per_s`。

**加权取数吞吐** 同阈值 `≥ 100,000 次 rng.weighted/秒`：20 万次 `rng.weighted`（20 项合成权重池，**复用权重数组** —— 调用方的正确用法）**实测 184,700 次/秒**（三次取中位；单次 183,607–185,502），同段 `rng.int` 为 750,497 次/秒 ⇒ **`perf.procgen.weighted_vs_int_ratio = 4.06`**。比值构成：两轮遍历（Σw + 扫描）各 20 次，加每元素一次 `to_int` 校验；同机 stock lua 5.5 对照为 3.9×（`weighted` 1.112 ms/1k、`int` 0.287 ms/1k）。代价可查：每元素标签若用 `"weights[" .. i .. "]"`（每次调用拼 20 个字符串）比值会升到 5.45、吞吐降到 137,366 次/秒——这条实测就是把它改成常量标签的原因。打印 `perf.procgen.rng_weighted_per_s` / `perf.procgen.weighted_vs_int_ratio`。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 物种池 / 神秘遭遇池（真数据加权） | `rng.weighted` 直接吃 `biomes.json` 的生境池 tier 权重与 `encounters.json` 的第三列；替换其仓内 `rng.int` + 手搓前缀和表的临时形态（S7，已实证替换**序列不变**） | 已交付 |
| CDDA | 地图 / 城市场景；生成表的频度权重 | 命名流 + 参数化生成器；同一 seed 复现同一座城。加权取数面同供「按频度抽表项」（第二用点，未实证） | P3 |
| DCSS | 地牢布局 | 多流（布局 / 怪物 / 物品各一条流），改一条流不扰动另一条 | P3 |
| Saltline | 地形与素材 seed 链 | 一条 seed 派生出纹理 / 几何 / 音频三条流，与 P2.5 `assetgen` 共享同一份 rng 契约 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.procgen.`（例 `pkg.procgen.same-seed-same-hash`、`pkg.procgen.stream-isolation`）。
- **断言面**：I1 用「同 seed 两次 `run` → `cells` 逐值相等 + `hash` 相等」，且 `canon` 行由**两次独立进程**各跑一次、宿主比对（跨 run 确定性）；I2 用「插入一条新流不改变既有流的序列」；I3 用静态扫字面量（含 `ygg.` 与三张标准库表，`os.` 带边界匹配以免误伤 `algos.`），并扫 `procgen/rng.lua` 不得出现 `pairs(` / `next(`（键序铁律：池序 = 数组声明序）；I4 用「装一个一访问就记账的宿主代理，一次 `run` 零触碰」。**加权面**另有四类断言：确定性（同 seed 两条流 1016 次逐值相等 + KAT16 定值 + 只消费一个 `next32`）、等权特例（`weighted(ones) == int(1,n)` 且句柄状态相等）、零权不可达（首/中/末三种位置各 20,000 次 0 命中，且正权项必须都被抽到）、分布（10⁶ 次 χ² < 43.82 (dof=19, p=0.999) **与**每桶相对偏差 ≤ 5% 双断言）。
- **用例清单（22 条，`pkg-procgen.lua`）**：`kat-xoshiro128ss` / `int-bounds` / `int-uniformity` / `float-range` / `pick-shuffle` / `weighted-bounds` / `weighted-equal-weight` / `weighted-determinism` / `weighted-zero-weight` / `weighted-distribution` / `same-seed-same-hash` / `diff-seed-diff-hash` / `stream-isolation` / `state-resume` / `run-pure` / `algo-version-in-hash` / `unknown-algo` / `params-error` / `static-no-entropy` / `leak-100-runs` / `compose-via-define` / `perf-load`。另有 3 条载体用例在 `pkg-procgen-digest.lua`（`same-seed-same-canon` 与两条跨语言向量 `rng.fnv64-cross-language-vectors` / `rng.derive-state-cross-language-vectors`）。
- **计数**：`--filter pkg-procgen` 的 `cases passed=` = **25**（22 + 3 条载体），全量 = **161**。载体是 `packages/mods/pkg-selftest/`（声明包闭包的那个消费夹具）——生产 Mod（`agent-seed`）不声明包，它的 `cases passed=` 永远是 79/0，这是回归钉。
- **冒烟不计数**：`tests/pkg-procgen-smoke.lua` 与 `tests/pkg-procgen-digest-smoke.lua` 是 **file 级**断言（`error` ⇒ `file FAIL` ⇒ `ok=0`），不进 `cases passed=`；它们接住 `--filter` 的 mod 侧过滤，并钉装载面与 canon 行格式。
- **禁止宽松断言**：只断「两次结果长度相同」不逐值相等，视同没测 I1（Goal P2.5 的验收是字节一致，不是形状一致）。

## 8. 设计依据

**为什么自带 PRNG 而不直接用 `math.random`。** 宿主的 `math.random` 由 `luai_makeseed` 播种，是**一份全局状态**：地图生成消耗几次随机数，怪物生成拿到的序列就会跟着变。于是「调一下地图参数」会静默改变怪物分布，而这类差异在验收里表现为「同一个 seed 产出不同」，最难归因。命名流把全局状态切成按用途独立的序列，代价是几十行纯 Lua。

**反方案一：每个 mod 自带噪声库。** 同一套 Perlin / 房间切分在四个复现里各写一遍，且各自的确定性口径不同——「同 seed 同产物」就变成四份都要单独证明。

**反方案二：把生成写进 `crates/**`。** 需要 Rust 语义时才是对的选择（例如需要真实文件 I/O）；纯 Lua 生成器放在包里，改参数不用重编二进制，也不会把游戏内容推进内核。

**反方案三：把 `weighted` 做成「前缀和快照入参」。** 让调用方传 `{ total, upto }`（或直接传 `total`）可以省一次 `Σw` 遍历，但那多一条契约行、多一份不变量（快照与权重数组可能不一致），而省下的是 ~0.07 µs/次——一次波次只抽 1–2 次，量级完全不在热路径上。手搓前缀和表就是从这个「优化」长出来的：它一开始只是为了避一次求和，最后变成四个复现各一份的采样器。

**反方案四：`{ w, item }` 行形状入参。** 看着自描述，但每次调用要为每个候选项造一张两字段表（实测比权重数组复用慢 **2×**）；而且一旦调用方改用「值 → 权重」的 map（参考实现 `weightedPick(items: Map<T, number>)` 就是这么写的），重复 payload 值会被 key 化吞掉。返回**下标**则把「抽什么」留给调用方，也把「池的顺序」钉在数组声明序上（无键序遍历）。

**反方案五：浮点权重 + 归一化到 `[0,1)`。** 归一化必然引入除法与浮点累加，跨机器可能落在不同的桶边界上（同 seed 同序列在 I1 里必须字面相等，不是近似相等）；而 `int` 的拒绝采样本来就要求整数面。小数权重不是不支持的场景，是「调用方放大成整数」（参考实现同口径：`randSeedInt(totalWeight)`）。

**与参考实现的有意偏差：`Σw == 0` 报错而不回落首项。** PokeRogue 原实现（`src/utils/random.ts` 的 `weightedPick`）在总权重 ≤ 0 时 `console.warn` 后返回第一个键——静默回落。包内不这么做：全零权重只有两种成因（数据缺列 / 阈值筛完），两种都是调用方缺陷；回落到首项会产出「确定性但错误」的抽样，而确定性恰好会把这类错误伪装成可复现行为。fail-closed 让数据缺陷在第一次生成就红。同理，**不新增第七个错误码**（如 `-WEIGHT`）：§4 的「恰好六个键」是已声明的契约面，新增码要连带改 `ERRORS` 副本面与全部码断言，而 `-PARAM`（参数形状不对，包括“没有可抽的东西”）已经足够表达。

**判据（①②③）。** ① 阻断——包内用 `math.random` 或读时钟；生成器里按游戏名分支；`run` 顺手写 KV 或发事件；mod 侧再长出一套前缀和/采样器。② 症状——同一 seed 两次产物不同；改一条流的参数导致另一条流的产物变化；生成完一次，KV 里多出没人认领的键；同一种「按权重抽」在四个复现里有四份实现且口径不同。③ 确认——本文件被索引；`selftest` 含 `pkg.procgen.*`；§5 各项加取数吞吐（含 `rng.weighted`）的数字齐备。
