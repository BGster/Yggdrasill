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

沙箱的确定性由宿主保证（种子走 `luai_makeseed`，见 [../kernel/lua-api.md](../kernel/lua-api.md)），但那是**全局**种子：两个生成器共用会让彼此扰动。本包的命名流就是为把全局状态隔离成按用途独立的序列。

## 2. 数据形状

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `rng.state` | 4 × 32 位无符号整数 | 是 | 由 `(seed, stream)` 派生 | 调用方持有；不写 KV，随存档由 [codec.md](codec.md) 点名导出 |
| `seed` | string 或 number | 是 | — | 调用方给；包内不生成、不取时钟 |
| `stream` | string（命名流） | 否 | `"main"` | 运行时 |
| `defs.<name>` | table `{ algo, params }` | 否 | `{}` | `ygg.kv` 键 `procgen.defs` |

生成器产物形状由 `algo` 决定，包不统一：`algo` 是注册过的 Lua 函数名，`params` 是它自己的参数表。新增形状 = 加一条 `defs` 数据；新增算法 = 加一个 `algo` 实现并回写本契约。

## 3. 不变量

- **I1 确定性**：同一 `(seed, stream, params)` 在同一 `procgen` 版本下产出同一序列，跨 run、跨机器成立。PRNG 为纯 Lua 的 32 位实现（xoshiro128\*\*），只依赖 Lua 5.4 的位运算符，不依赖宿主 PRNG。
- **I2 流隔离**：不同 `stream` 名的序列互不影响——每个流的状态由 `hash(seed .. "/" .. stream)` 独立派生，不共享任何全局计数器。
- **I3 无外部熵**：包内不得出现 `math.random` / `math.randomseed` / `os.` / `io.` 字面量；静态用例逐文件扫。
- **I4 生成器纯函数化**：`procgen.run(name, seed)` 只返回产物表，不改 KV、不发事件；落盘与事件由调用方负责。
- **I5 参数化而非分支**：生成器由 `{ algo, params }` 声明驱动；`algo` 内不得按「哪个游戏」分支——游戏差异全部经 `params` 进来。
- **I6 版本进入哈希**：`procgen.hash` 的输入包含 `algo` 名与算法版本号，所以换算法必然换哈希（不会出现「算法变了但验收仍然通过」）。
- **I7 顶层纯度**：包模块顶层只有纯导出，`defs` 注册一律在 `M.install(deps)`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `procgen.rng` | `(seed, stream?)` | rng 句柄 | 纯（新流） |
| `rng.int` | `(rng, lo, hi)` | number（闭区间） | 纯（推进状态） |
| `rng.float` | `(rng)` | number `[0,1)` | 纯（推进状态） |
| `rng.pick` | `(rng, list)` | 元素 | 纯（推进状态） |
| `rng.shuffle` | `(rng, list)` | 新表（Fisher–Yates） | 纯（推进状态） |
| `procgen.define` | `(name, spec)` | `true` | 写（`procgen.defs`）；异 spec 重复定义报错 |
| `procgen.algo` | `(name, fn)` | `true` | 写；注册 `algo` 实现（`fn(params, rng)`） |
| `procgen.run` | `(name, seed, stream?)` | 产物表 | 纯（不写不 emit） |
| `procgen.hash` | `(value)` | string（稳定十六进制） | 纯；键按字节序的规范化摘要，用于验收 |
| `procgen.state` | `(rng)` | table（4 整数副本） | 纯；用于「存档后从同一位置继续生成」 |

## 5. 性能预算

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`（纯 Lua，`install` 不预热流）。测法同 [entity.md](entity.md) §5。 |
| 实体按 id 查找 / 更新 | `O(1)` | **不适用**：本包不碰实体表；唯一的表访问是 `defs` 直取（`procgen.defs[name]`）。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：产物序列化由 [codec.md](codec.md) §5 承包；本包只保证产物的字节稳定性由 §5「确定性」一行覆盖。 |
| 10k 实体常驻内存 | 给出数字 | 不适用（无实体）。等价口径：一张 64×64 地图产物 `≤ 256 KiB`；硬帽 `16 MiB`。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 确定性替身：同一 `(seed, stream)` 跑 100 轮 `run`，100 次 `hash` 全相等且内存差 `≤ 1%`（生成表可回收）。 |

另加一条本包专属数字：**取数吞吐**目标 `≥ 100,000 次 rng.int/秒`（同机器跑三次取中位），因为地图生成的热路径在这里。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| CDDA | 地图 / 城市场景 | 命名流 + 参数化生成器；同一 seed 复现同一座城 | P3 |
| DCSS | 地牢布局 | 多流（布局 / 怪物 / 物品各一条流），改一条流不扰动另一条 | P3 |
| Saltline | 地形与素材 seed 链 | 一条 seed 派生出纹理 / 几何 / 音频三条流，与 P2.5 `assetgen` 共享同一份 rng 契约 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.procgen.`（例 `pkg.procgen.same-seed-same-hash`、`pkg.procgen.stream-isolation`）。
- **断言面**：I1 用「同 seed 两次 `run` → `hash` 相等」且**两次进程**各跑一次（跨 run 确定性）；I2 用「插入一条新流不改变既有流的序列」；I3 用静态扫字面量；I4 用「`run` 前后 KV 键集合不变、事件计数不变」。
- **计数**：计入 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed` 的 `cases passed=`（发现机制见 [README.md](README.md) §5 未决项）。
- **禁止宽松断言**：只断「两次结果长度相同」不逐值相等，视同没测 I1（Goal P2.5 的验收是字节一致，不是形状一致）。

## 8. 设计依据

**为什么自带 PRNG 而不直接用 `math.random`。** 宿主的 `math.random` 由 `luai_makeseed` 播种，是**一份全局状态**：地图生成消耗几次随机数，怪物生成拿到的序列就会跟着变。于是「调一下地图参数」会静默改变怪物分布，而这类差异在验收里表现为「同一个 seed 产出不同」，最难归因。命名流把全局状态切成按用途独立的序列，代价是几十行纯 Lua。

**反方案一：每个 mod 自带噪声库。** 同一套 Perlin / 房间切分在四个复现里各写一遍，且各自的确定性口径不同——「同 seed 同产物」就变成四份都要单独证明。

**反方案二：把生成写进 `crates/**`。** 需要 Rust 语义时才是对的选择（例如需要真实文件 I/O）；纯 Lua 生成器放在包里，改参数不用重编二进制，也不会把游戏内容推进内核。

**判据（①②③）。** ① 阻断——包内用 `math.random` 或读时钟；生成器里按游戏名分支；`run` 顺手写 KV 或发事件。② 症状——同一 seed 两次产物不同；改一条流的参数导致另一条流的产物变化；生成完一次，KV 里多出没人认领的键。③ 确认——本文件被索引；`selftest` 含 `pkg.procgen.*`；§5 五项加取数吞吐共六个数字齐备。
