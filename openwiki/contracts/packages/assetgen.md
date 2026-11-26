---
type: "Reference"
title: "assetgen — 素材生成（纹理 / 几何 / 音频，seed 驱动字节一致）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-fef267a23f756637e121d083
    resource: repo://crates/ygg-sandbox/src/sandbox.rs
  - id: openwiki-source-7eae95479f2663371d4eed2c
    resource: repo://packages/libs/assetgen/bytes.lua
  - id: openwiki-source-f267d664eaa2d2362f839f28
    resource: repo://packages/libs/assetgen/encode.lua
  - id: openwiki-source-aa44f13aff21eb0dca42ea0c
    resource: repo://packages/libs/assetgen/init.lua
  - id: openwiki-source-7590626fcb363e2175925d42
    resource: repo://packages/libs/assetgen/mathx.lua
  - id: openwiki-source-3452921d36e4e0b8299fe452
    resource: repo://packages/libs/assetgen/sound.lua
  - id: openwiki-source-3de1f189234643847a84ec0b
    resource: repo://packages/libs/assetgen/spec.lua
  - id: openwiki-source-0cefdbdcfaf2fe4ef7804419
    resource: repo://packages/libs/assetgen/test/pkg-assetgen-digest.lua
  - id: openwiki-source-d72d551ce8e63f99a16e4a6e
    resource: repo://packages/libs/assetgen/test/pkg-assetgen.lua
  - id: openwiki-source-c0164a565d51a1dcbd709820
    resource: repo://packages/libs/assetgen/texture.lua
  - id: openwiki-source-a0892a98ca143451ee73aebb
    resource: repo://packages/libs/procgen/pkg.json
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# assetgen — 素材生成（纹理 / 几何 / 音频，seed 驱动字节一致）

> **id** packages/assetgen · **模块** ⑤ packages · 通用能力层 · **触发** 改素材字节形状、噪声与波形参数、容器头之前
> **读不到代价** irreversible（同一 seed 两次生成的素材字节不同，已交付的贴图 / 网格 / 音效再也复现不出来；改容器头而不升 `v` 会让既有产物静默错位）
> **replaces**：DESIGN §4 assetgen（搬运基线 `3a2536f`）
> **上游**：[README.md](README.md) · **相关**：[procgen.md](procgen.md)（**唯一**包依赖：只用它的 rng 契约）· [codec.md](codec.md)（产物落盘与整表导出）· [../kernel/lua-api.md](../kernel/lua-api.md)（挂载桥 4 MiB 单件帽、沙箱 globals 白名单）

## 1. 边界（这个包不做什么）

- **不做地图 / 任务 / 实体**：素材的「用途」（哪张贴图给哪个瓦片、哪段音效给哪把枪）是**数据**，归 mod。
- **不落盘**：产物是返回值。落盘经 `ygg.fs.write`（挂载桥）或 [codec.md](codec.md)，由调用方先立事件。
- **不做摘要**：包内**不实现**哈希。字节摘要由宿主给（`ygg.fs.write` 返回 `{path, bytes, sha256}`）；包内变更检测用 `assetgen.canon(product)`（单行 ASCII 描述串）。仓里已有唯一一份 FNV（`procgen.hash`）与唯一一份 sha256（`pkg.codec.sha256`），第三份就是分叉起点。
- **不复制 PRNG**：xoshiro128\*\*、种子文本口径、流派生规则全部住 [procgen.md](procgen.md)；本包 `require("pkg.procgen")` 并用 `procgen.rng(seed, stream)`。静态用例扫「xoshiro 常量 / `fnv64` / 混淆常量」在小包源码中出现即红。
- **不共享生成算法**：procgen 产 `cells`（0..255 地图格），本包产 `pixels`（u8 连续字节）与 `pcm`（s16 交织）。共享的是 **rng 契约**，不是算法。
- **不做渲染 / 播放**：三角面是顶点 + 索引表，立体声是 s16 交织；上屏、混音、解码归 mod 与引擎。
- **不做 PNG / JPEG / Ogg**：需要 deflate / 熵编码，超出本需求预算。纹理的**人眼可查**容器是 BMP24。
- **不读外部素材、不读时钟、不读环境**：零素材输入，产物只由 `(seed, stream, algo, params)` 决定。
- **不碰宿主面**：不写 KV、不发事件、不注册 handler；`install` 只接 `deps.procgen` 注入（测试替身用），连 `host` 都不接。

## 2. 数据形状

**三个通用字段**（三类产物共有，形状唯一）：

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `kind` | string | 是 | 由生成函数附着 | 返回值；`"texture"` / `"mesh"` / `"sound"` |
| `algo` | string | 是 | 由 `spec.algo` 附着 | 返回值；产生它的算法名（`texture.fbm` 等） |
| `v` | integer | 是 | 单一常量 `1` | 返回值；产物契约版本（I6） |
| `bytes` | string | 是 | 由生成函数附着 | 返回值；**canonical 字节**，落盘就是它 |
| `canon` | string | 是 | 由生成函数附着 | 返回值；单行 ASCII 描述串（`k=v` 单空格分隔，无换行），`assetgen.canon(product)` 与它逐字符相等 |
| `seed` / `stream` | string | 是 | 由调用参数派生 | 返回值；参与 `canon`（`canon` 是「同 seed 同字节」的**变更检测**载体，不是哈希——I9） |

**spec（调用入参，与 procgen 的 def 同形）**：`{ algo = string, params = table }`。`algo` 未知报 `-ALGO`；`params` 逐键校验（值域见 I8）。

**(a) 纹理 product**

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `w`, `h` | integer | `1..4096`；且 `w*h*ch ≤ 4194304`（= 挂载桥单件帽 4 MiB） |
| `ch` | integer | 1（灰度）/ 3（RGB）/ 4（RGBA） |
| `bytes` | string | `w*h*ch` 个 u8，**行主序、无行内边距、无文件头**（就是贴图上传要的那段连续内存） |

**(b) 几何 product**

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `verts` | 稠密数组，3×N 个 **integer** | 单位 = 毫米（1/1000）；右手坐标系，Y 上 |
| `faces` | 稠密数组，3×M 个 integer | 三角形索引，**1 基**（与 OBJ 同口径） |
| `size` | 3 个 integer | 轴对齐包围盒尺寸（mm），`max-min` |
| `bytes` | string | OBJ 文本（`# assetgen mesh v1 <algo>` + `v` 行 + `f` 行，十进制固定 3 位） |

**(c) 音频 product**

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `sr` | integer | 采样率，白名单 `{8000,11025,16000,22050,32000,44100,48000}` |
| `ch` | integer | 1 / 2 |
| `frames` | integer | 每声道采样数，`1..400000`（≈ 8 s @ 48 kHz 单声道 / 4 s 双声道） |
| `peak` | integer | 归一峰值（千分比 `0..1000`），不变式保证 `\|sample\| ≤ peak*32767//1000` |
| `pcm` | string | `frames*ch*2` 字节，**s16 小端交织** |
| `bytes` | string | **WAV**：44 字节标准头 + `pcm`（`RIFF`/`WAVE`/`fmt `/`data`，PCM=1，16 bit） |

**bundle product**（一条 seed 派生成套素材）：`{ v, seed_text, items = { { key, kind, product }, ... } }`——`items` 顺序 = 入参数组顺序（不用 `pairs`），`key` 非空且唯一。

**持久化位置**：全部**不落 KV**。产物由调用方 `ygg.fs.write` 落挂载点，或整表经 [codec.md](codec.md) 导出。

## 3. 不变量

- **I1 同 seed 同字节**：同一 `(seed, stream, algo, params)` 在同一 `v` 下产出**逐字节相同**的 `bytes`，跨 run / 进程 / 机器。实现前提：产物路径**只有整数运算**（Lua 5.4 `integer`），无 `math` 库、无浮点字面量、无 `/`（`//` 才是除法）。
- **I2 流隔离与缺省流名**：`stream` 缺省 = `"assetgen/<kind>/<algo>"`；`bundle` 内缺省 = `"assetgen/<key>"`。不同流名的序列互不影响；**同一** `(seed, stream)` 在不同 algo 上共享同一随机序列——这是 procgen 的已声明性质，本包不另造命名空间。
- **I3 无外部熵**：包内无 `math.random` / `os.` / `io.` / 时钟 / 环境读；静态用例逐文件扫。
- **I4 纯函数化**：`texture` / `mesh` / `sound` / `raster` / `encode` / `bundle` / `canon` 只读入参、只返回新表；不写 KV、不发事件、不落盘。`install` 之后无模块级可变状态。
- **I5 参数化而非分支**：algo 名决定形状，`params` 决定内容；**不得**按游戏名 / 资源名 / 流名分支。未知 algo = `-ALGO`（不静默回退到缺省算法）。
- **I6 版本进入产物**：`product.v` 是单一常量；字节形状一改就 +1——不存在「只改了 WAV 头但 `v` 没动」的位置。
- **I7 顶层纯度**：包模块顶层只允许 `local` / `require` / 函数与常量定义；`deps` 注入与算法表组装在 `install`。
- **I8 值域与硬帽（两档）**：非整数 = `-PARAM`；未知算法 = `-ALGO`。数值参数再分两类：**资源维度**（像素数、细分数、站点数、帧数）低于下界 = `-PARAM`、**高于上界 = `-CAP`**；**语义值**（`ch` / 角度 / 占空比 / 千分比 / 坐标偏移 / 光照）两侧越界都是 `-PARAM`——超帽要调用方减尺寸，写错值只要他改数字。越界**不截断**：截断会把半个素材交出去。
  - 像素 `0..255`；`ch ∈ {1,3,4}`；`w,h ∈ 1..4096` 且 `w*h*ch ≤ 4194304`。
  - 顶点单位 mm、`|coord| ≤ 1000000`；顶点数 ≤ 65535；细分数 `segs,rings ∈ 3..64`。
  - 采样 `-32768..32767`、`|s| ≤ peak*32767//1000`；`frames ≤ 400000`；`sr` 白名单；`ch ∈ {1,2}`。
  - 旋转一律 `1/256` 转（`turn256 ∈ 0..255`）；缩放与权重是千分比（`1000` = 1.0）。
- **I9 无摘要、无第二份编码**：包内不实现哈希 / 序列化。`canon` 是**描述串**不是摘要：不含压缩或散列运算，只把形状字段与长度写成 `k=v`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `assetgen.install` | `(deps?)` | 包表 | 写（装配期）：`deps = { procgen? }`；重复装幂等 |
| `assetgen.texture` | `(spec, seed, stream?)` | 纹理 product | 纯（新表；不写不 emit） |
| `assetgen.mesh` | `(spec, seed, stream?)` | 几何 product | 纯 |
| `assetgen.sound` | `(spec, seed, stream?)` | 音频 product | 纯 |
| `assetgen.raster` | `(mesh, opts)` | 纹理 product | 纯。`opts = { w, h, scale, ox?, oy?, angle?, fill?, shades? }`——正交投影后按三角面填充 |
| `assetgen.encode` | `(product, container?)` | string（字节） | 纯。容器：`"raw"` / `"bmp24"` / `"wav16"` / `"pcm16"` / `"obj"` / `"hex"`（证据面） |
| `assetgen.bundle` | `(entries, seed)` | bundle product | 纯。`entries[i] = { key, kind, algo, params, stream? }` |
| `assetgen.canon` | `(product)` | string（单行 ASCII） | 纯。与 `product.canon` 逐字符相等 |
| `assetgen.streams` | `(seed)` | `{ texture, mesh, sound }` 三个句柄 | 纯。等价于三条缺省流的 `procgen.rng` |
| `assetgen.algo_list` | `(kind)` | 稠密数组（字节序升序） | 纯（自省面） |
| `assetgen.perf_keys` | `()` | 稠密数组（§5 的十个键名，不含前缀） | 纯（自省面；「十个键齐备」因此是机检） |

`assetgen.ERRORS` 恰好五个键，形如 `YGG-PKG-ASSETGEN-*`：`PARAM`（参数形状 / 下界错）、`ALGO`（未注册算法）、`CAP`（硬帽 / 上界越界）、`KIND`（`kind` 不在三值）、`DEPS`（注入的 procgen 面不合法）。

## 5. 性能预算

**口径声明**：下表数字产在 **selftest 面**（普通 Lua 5.4，全标准库）——沙箱里既没有 `os.clock` 也没有 `collectgarbage`，包本身连 `math` 都不可达。三次取中位、批内三次取中位，`print` 面即口径。

| 指标 | 阈值 | 实测值 | 测量方法 / 打印键 |
| --- | --- | --- | --- |
| 包加载耗时 | `< 10ms` | **1.8 ms**（`install` 只做表组装） | 清 `package.loaded["pkg.assetgen*"]` 后 `require` + `install`，5 次取中位 → `perf.assetgen.load_ms` |
| 实体按 id 查找 / 更新 | `O(1)` | 不适用：无实体表；唯一表访问是算法表直取与参数表按名取键 | — |
| 序列化吞吐 | 附基准集与耗时 | **935,820 样本/秒**（阈值 300k，3.1× 余量）。口径 = 「10 万帧单声道 wav16 的生成 + 交织 + 装箱」整条路径 | 100 800 帧单声道跑三次取中位 → `perf.assetgen.pcm_samples_per_s` |
| 10k 实体常驻内存 | 给出数字 | 等价口径：**64×64 RGBA 纹理 16.7 KiB**（阈值 128 KiB）、**1 s @22050 单声道 86.8 KiB**（阈值 2.5 MiB） | 生成前后两次 `collectgarbage("count")` 差 → `perf.assetgen.tex64_mem_kib` / `sound1s_mem_kib` |
| 泄漏检测 | 100 轮不增长 | 同 seed 100 轮：`bytes` 100 次全等（`leak_100_equal=1`），内存差 **0.25%**（阈值 1%） | `perf.assetgen.leak_100_equal` / `leak_gc_pct` |
| **生成耗时 per 类** | 无外部阈值，给数字 | 纹理 64×64 `ch=4` fbm 3 倍频 **17.5 ms**；几何 torus 24×12 **1.24 ms**；音频 1 s @22050 osc+包络 **28.7 ms**；16 件 bundle **35.3 ms** | 各三次取中位 → `perf.assetgen.tex64_ms` / `mesh_torus_ms` / `sound_1s_ms` / `bundle16_ms` |
| **字节一致（真验收）** | 两次独立进程 sha256 相等 | 必达：两进程各跑 `test/pkg-assetgen-digest.lua` → 宿主 `cmp` + `xxd -r -p \| shasum -a 256` | — |

**实现纪律**：目标值在首个实现提交时就地改成实测值，偏差即回写本表（不回写 = 文档与实现不符）。超阈不得靠「调小默认尺寸」蒙过去：阈值口径是「64×64 / 1 s / 16 件」，尺寸是定义的输入，不是可调旋钮。回归阈设在实测值的 2×（`tex64_ms ≤ 40` / `sound_1s_ms ≤ 60`），这样抓的是「代码变慢了」而不是「机器抖了一下」。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| Saltline（单文件 3D FPS，零素材） | 全套：地表 / 金属 / 涂装噪声纹理、道具网格（轮胎堆 / crate 堆 / 油桶 / 围栏）、枪声 / 脚步 / 命中音效 | `texture.fbm` + 调色、`mesh.*` + `mesh.compose`、`sound.*` + WAV；一条 seed → 三条流 | P3 |
| CDDA（字符渲染，JSON 数据驱动，零素材） | 16×16 瓦片、音效包 | `texture.*`（`w=h=16`）+ `sound.noise` / `sound.partials` | P3 |
| DCSS（字符界面 + webtiles） | 32×32 地砖与物件底纹、法术音效 | `texture.bricks` / `texture.voronoi` / `texture.rings` + `sound.osc` 扫频 | P3 |
| PokeRogue（TS 图形化） | 道具 / 徽章图标（**几何 → 栅格**）、音效、战斗背景底纹 | `mesh.*` 基元 + `assetgen.raster` + `sound.mix` 分层 | P3 |

**几何的第二个用点**（包级 ≥2 闸最脆的一环，单列）：3D 网格只有 Saltline 一个确定用点；**第二个用点是 PokeRogue 的 2D 图标**——`assetgen.raster` 把同一套基元与变换正交投影后填充成纹理。几何的**形状**（顶点 + 面 + 包围盒）在两种投影下都是同一份数据，栅格化只是它的一种消费方式。

## 7. 自测与验收

- **用例名前缀**：`pkg.assetgen.`；perf 用例名 `pkg.assetgen.perf`。
- **计数**：`--filter pkg-assetgen` → `cases passed=` **28**（实测 2026-11-26）；pkg-selftest 全量 **199**。
- **用例清单（28）**：纹理 `texture-same-seed-same-bytes` / `texture-diff-seed-diff-bytes` / `texture-size-and-shape` / `texture-pattern-exact` / `texture-gradient-endpoints` / `texture-voronoi-tiebreak` / `texture-caps`；几何 `mesh-primitives` / `mesh-compose-order` / `mesh-ints-only` / `mesh-obj-text` / `mesh-caps`；栅格 `raster-fill` / `raster-input`；音频 `sound-same-seed-same-bytes` / `sound-wav-header` / `sound-peak-cap` / `sound-envelope-decay` / `sound-mix-sum` / `sound-waveforms` / `sound-caps`；跨类 `bundle-order-and-keys` / `stream-isolation` / `no-float-static` / `rng-not-copied` / `leak-100-rounds` / `perf`；跨进程 `cross-process-canon`。
- **两条 mod 侧载体冒烟不计数**（file 级 `error` ⇒ `ok=0`）：`tests/pkg-assetgen-smoke.lua`（钉装载面 / 顶层纯度 / `ERRORS` 码集 / 导出面清单 / 十个 `perf.assetgen.*` 键名）与 `tests/pkg-assetgen-digest-smoke.lua`（钉 canon 行格式；空值与多行会让 `cmp` 静默通过）。
- **为什么必须有 mod 侧同名文件**：`--filter` 对 mod 的 `tests/*.lua` 与包的 `test/*.lua` **同时**过滤，mod 侧筛完为空即「没有可执行的测试文件」exit 1。
- **跨进程字节一致门禁链（可直接粘贴）**：

  ```sh
  RUN='cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-assetgen-digest'
  for pass in a b; do
    $RUN 2>&1 | grep '^pkg\.assetgen\.' > "/tmp/p25-assetgen-$pass.txt"
  done
  cmp /tmp/p25-assetgen-a.txt /tmp/p25-assetgen-b.txt && echo 'canon identical'
  sed -n 's/^pkg\.assetgen\.hex=//p' /tmp/p25-assetgen-a.txt | xxd -r -p > /tmp/p25-assetgen.bin
  shasum -a 256 /tmp/p25-assetgen.bin
  ```

  这条链**不写进任何脚本文件**（AGENTS.md §1.7 禁「用脚本携带约束」）：它是文档里的一段可粘贴命令，证据是它的输出。
- **禁止宽松断言**：只断「两次长度相同」不逐字节 `==`，视同没测 I1；只断「WAV 头能构造」不逐字段对表，视同没测容器。
- **人工抽验（一次）**：把 digest 链产出的 WAV / BMP 用系统 `file` / `afinfo` 看一眼，这是「容器算对了」的第三方视角，不替代断言。
- **已知缺口（如实登记）**：① `pkg.*` 未在真沙箱里跑过，且已有可复现隐患——`packages/libs/codec/init.lua` 与 `packages/libs/entity/init.lua` 调用 `math.huge` / `math.type`（沙箱里 `math` 是 `nil`）；本包只能做到「自己不踩」。② 跨平台（x86_64 ↔ aarch64）字节一致无实测，只有结构性论证。

## 8. 设计依据

**为什么素材生成是包而不是 mod / 内核。** 四个复现的共同需求是「零外部素材」；各自实现 = 四份噪声、四份 WAV 头、四份波形表。而它**不能**进内核：参数化只需要整数与字节，没有 I/O、没有事件，落进 `crates/**` 只会让「调一个 octaves」变成重新编译。

**为什么与 procgen 共享的是 rng 而不是算法。** `cells` 是 0..255 的地图格（带自己的 `algo`/`v` 与加权调度），`pixels` 是连续字节、`pcm` 是 s16 交织——把 `terrain.noise` 拉进纹理管线要么多一层 cells→pixels 转换，要么把纹理参数塞进 def 注册表（参数随素材变化 → 同名 def 冲突报 `-DEF`）。共享 PRNG 是零成本的（一行 `require`），共享算法是负成本的。

**为什么产物路径必须整段整数域。** 浮点跨进程一致性无法结构性保证（编译器 FMA 收缩、libm 尾位差、`-ffast-math`），而「同 seed 同产物字节」是硬验收。整数域下这条不变式是**构造性成立**的。四条支撑：① 沙箱包里 `math` 仍为 `nil`（`math` 是禁止的二级面），三角与开方只能自备整数核（`mathx.lua`：Q15 有理近似 + 整数牛顿开方）；② Lua 5.4 的 `+ - * // % & | ~ << >>` 在 `integer` 上精确，`/` 是唯一浮点除法，静态用例禁掉；③ 三道检查（静态扫字面量、`math.type` 断言、两进程 sha256）；④ 残余风险如实声明——跨平台一致是**论证**不是实测。

**为什么纹理与音频实测比初估慢（17.5 ms vs 10、28.7 ms vs 12）。** 两处都是「每像素 / 每样本一次到几次整除 + 一次 `("%c%c"):format`」的量级：64×64×4 三倍频是 49 万次插值，1 s@22050 是 2.2 万次查表 + 包络整除 + 4.4 万次 s16 装箱。沙箱里既没有 JIT，也没有 `string.char` / `table.concat`——这两个数字是**这个方言下的下限**而不是实现缺陷。故选**记录而不优化**，回归阈设 2× 实测值。

**反方案**：① 用 `math.sin` / `math.sqrt`——沙箱里 `math` 是 `nil`，且这条约束顺手把浮点赶出了产物路径。② 宿主提供 PNG / 音频编码 API——要动 `crates/**`，而 44 字节 WAV 头与 54 字节 BMP 头纯 Lua 就够。③ 用 `codec.encode` 造字节——JSON 文本不是素材容器，二进制进它会走控制字符转义。④ 正弦表写成 256 项字面量——吃掉源码预算且无确定性收益（Q15 近似误差 ≤ 0.2%，SFX 上不可听）。

**r1-F8（CLI 写盘不落）定论：与 fs 桥同根因，随 S-1 关闭（2026-11-21）。** 真 CLI 曾因宿主未接桥而全体 `YGG-FS-UNMOUNTED`；S-1 在 egg.rs 两内核构造点接上挂载桥后，用声明私有可写挂载的冒烟 mod 实测 `ygg.fs.write("mounts/scratch/hello.txt")` 回执 `{bytes=15, sha256:…}`、宿主侧同路径 `shasum -a 256` 相等、`fs.jsonl` 审计行在盘、Lua 侧回读相等断言真。写穿链路已在真 CLI 验证，本包无单独改动。

**判据（①②③）。** ① 阻断——把游戏规则写进包；用 `math.random` 或时钟取熵；产物路径出现浮点；包内实现第三份哈希 / 编码；按资源名分支。② 症状——同 seed 两次生成文件 sha256 不同；改一个纹理参数导致音效字节变化（流未隔离）；`.wav` 打不开或时长错；`pkg.assetgen` 在沙箱里 nil-index 崩。③ 确认——本页被 [README.md](README.md) 索引；`--filter pkg-assetgen` 的 `cases passed=28`；§5 的 `perf.assetgen.*` 键齐备；两进程 canon / `shasum` 相等。
