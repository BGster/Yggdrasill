# Yggdrasill · packages/assetgen — 素材生成（纹理 / 几何 / 音频，seed 驱动字节一致）

> **id** packages/assetgen · **模块** ⑤ packages · 通用能力层 · **触发** 改素材字节形状、噪声与波形参数、容器头之前
> **读不到代价** irreversible（同一 seed 两次生成的素材字节不同，已交付的贴图 / 网格 / 音效再也复现不出来；改容器头而不升 `v` 会让既有产物静默错位）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [procgen.md](procgen.md)（**唯一**包依赖：只用它的 rng 契约）· [codec.md](codec.md)（产物落盘与整表导出）· [../kernel/lua-api.md](../kernel/lua-api.md)（挂载桥 4 MiB 单件帽、沙箱 globals 白名单）

## 1. 边界（这个包不做什么）

- **不做地图 / 任务 / 实体**：素材的「用途」（哪张贴图给哪个瓦片、哪段音效给哪把枪）是**数据**，归 mod。
- **不落盘**：产物是返回值（字节串 + 结构化字段）。落盘经 `ygg.fs.write`（挂载桥）或 [codec.md](codec.md)，且由调用方先立事件（`AGENTS.md` §1）。
- **不做摘要**：包内**不实现**哈希。字节摘要由宿主给（`ygg.fs.write` 返回 `{path, bytes, sha256}`，见 [../kernel/lua-api.md](../kernel/lua-api.md)）；包内变更检测用 `assetgen.canon(product)`（单行 ASCII 描述串）。理由：仓里已有唯一一份 FNV（`procgen.hash`）与唯一一份 sha256（`pkg.codec.sha256`），第三份就是分叉起点。
- **不复制 PRNG**：xoshiro128\*\*、种子文本口径、流派生规则全部住在 [procgen.md](procgen.md)；本包 `require("pkg.procgen")` 并使用 `procgen.rng(seed, stream)`。静态用例扫「xoshiro 常量 / `fnv64` / 混淆常量」在小包源码中出现即红。
- **不共享生成算法**：噪声 / 图案 / 波形是**形状不同**的东西——procgen 产出 `cells`（0..255 地图格），本包产出 `pixels`（u8 连续字节）与 `pcm`（s16 交织）。共享的是 **rng 契约**，不是算法。
- **不做渲染 / 播放**：三角面是顶点 + 索引表，立体声是 s16 交织；上屏、混音、解码归 mod 与引擎。
- **不做 PNG / JPEG / Ogg**：需要 deflate / 熵编码，超出本需求预算（反方案见 §8）。纹理的**人眼可查**容器是 BMP24。
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

**spec（调用入参，与 procgen 的 def 同形）**：`{ algo = string, params = table }`。`algo` 未知报 `-ALGO`；`params` 逐键校验（值域见 §3 I8）。

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
| `frames` | integer | 每声道采样数，`1..400000`（≈ 8 s @ 48 kHz 单声道 / 4 s 双声道，成品 ≤ 1.6 MB < 单件帽 4 MiB） |
| `peak` | integer | 归一峰值（千分比 `0..1000`）；不变式保证 `\|sample\| ≤ peak*32767//1000` |
| `pcm` | string | `frames*ch*2` 字节，**s16 小端交织** |
| `bytes` | string | **WAV**：44 字节标准头 + `pcm`（`RIFF`/`WAVE`/`fmt `/`data`，PCM=1，16 bit） |

**bundle product**（「一条 seed 派生成套素材」）：`{ v, seed_text, items = { { key, kind, product }, ... } }`——`items` 顺序 = 入参数组顺序（不用 `pairs`），`key` 非空且唯一。

**持久化位置**：全部**不落 KV**。产物由调用方 `ygg.fs.write` 落挂载点，或整表经 [codec.md](codec.md) 导出（届时 `bytes` 由调用方决定是否随存档，包不表态）。

## 3. 不变量

- **I1 同 seed 同字节**：同一 `(seed, stream, algo, params)` 在同一 `v` 下产出**逐字节相同**的 `bytes`，跨 run、跨进程、跨机器。实现前提：产物路径**只有整数运算**（Lua 5.4 `integer`），无 `math` 库、无浮点字面量、无单斜杠除法（`//` 才是除法）。
- **I2 流隔离与缺省流名**：`stream` 缺省 = `"assetgen/<kind>/<algo>"`；`bundle` 内缺省 = `"assetgen/<key>"`。不同流名的序列互不影响（[procgen.md](procgen.md) I2 的推论）；**同一** `(seed, stream)` 在不同 algo 上共享同一随机序列——这是 procgen 的已声明性质，本包不另造命名空间。
- **I3 无外部熵**：包内无 `math.random` / `os.` / `io.` / 时钟 / 环境读；静态用例逐文件扫。
- **I4 纯函数化**：`texture` / `mesh` / `sound` / `raster` / `encode` / `bundle` / `canon` 只读入参、只返回新表；不写 KV、不发事件、不落盘。`install` 之后无模块级可变状态（`deps` 一旦注入即只读）。
- **I5 参数化而非分支**：algo 名决定形状，`params` 决定内容；**不得**按游戏名 / 资源名 / 流名分支。未知 algo = `-ALGO`（不静默回退到缺省算法）。
- **I6 版本进入产物**：`product.v` 是单一常量；字节形状一改就 +1——不存在「只改了 WAV 头但 `v` 没动」的位置。
- **I7 顶层纯度**：包模块顶层只允许 `local` / `require` / 函数与常量定义；`deps` 注入与算法表组装在 `install`（与 [../mod/mod-package.md](../mod/mod-package.md) 的 `mod.pure-export` 同规）。
- **I8 值域三段与硬帽**：数值参数按三段判定——非整数 / **低于下界** = `-PARAM`（形状错）；**高于上界** = `-CAP`（资源帽）；未知算法 = `-ALGO`。越界**不截断**：截断会把半个素材交给调用方。
  - 像素 `0..255`；`ch ∈ {1,3,4}`；`w,h ∈ 1..4096` 且 `w*h*ch ≤ 4194304`。
  - 顶点单位 mm、`|coord| ≤ 1000000`；顶点数 ≤ 65535（OBJ 索引与 u16 索引缓冲双友好）；细分数 `segs,rings ∈ 3..64`。
  - 采样 `-32768..32767`、`|s| ≤ peak*32767//1000`；`frames ≤ 400000`；`sr` 白名单；`ch ∈ {1,2}`。
  - 旋转一律是 `1/256` 转（`turn256 ∈ 0..255`）；缩放与权重是千分比（`1000` = 1.0）。
- **I9 无摘要、无第二份编码**：包内不实现哈希 / 序列化（见 §1）。`canon` 是**描述串**不是摘要：它不含任何压缩或散列运算，只把形状字段与长度写成 `k=v`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `assetgen.install` | `(deps?)` | 包表 | 写（装配期）：`deps = { procgen? }`；重复装幂等（算法表同名覆盖、`deps` 同值幂等） |
| `assetgen.texture` | `(spec, seed, stream?)` | 纹理 product | 纯（新表；不写不 emit） |
| `assetgen.mesh` | `(spec, seed, stream?)` | 几何 product | 纯 |
| `assetgen.sound` | `(spec, seed, stream?)` | 音频 product | 纯 |
| `assetgen.raster` | `(mesh, opts)` | 纹理 product | 纯。`opts = { w, h, scale, ox?, oy?, angle?, fill?, shades? }`——正交投影后按三角面填充（2D 用点、3D 用面） |
| `assetgen.encode` | `(product, container?)` | string（字节） | 纯。容器：`"raw"`（纹理恒等）/ `"bmp24"` / `"wav16"`（音频恒等）/ `"pcm16"` / `"obj"`（网格恒等）/ `"hex"`（小写十六进制，证据面） |
| `assetgen.bundle` | `(entries, seed)` | bundle product | 纯。`entries[i] = { key, kind, algo, params, stream? }` |
| `assetgen.canon` | `(product)` | string（单行 ASCII） | 纯。与 `product.canon` 逐字符相等 |
| `assetgen.streams` | `(seed)` | `{ texture, mesh, sound }` 三个句柄 | 纯。等价于 `procgen.rng(seed, "assetgen/texture"/…)` 三条缺省流 |
| `assetgen.algo_list` | `(kind)` | 稠密数组（algo 名，字节序升序） | 纯（自省面；mod 侧 UI / 校验用） |

`assetgen.ERRORS` 恰好五个键，全部形如 `YGG-PKG-ASSETGEN-*`：`PARAM`（参数形状 / 下界错）、`ALGO`（未注册算法）、`CAP`（硬帽 / 上界越界）、`KIND`（`kind` 不在三值）、`DEPS`（注入的 procgen 面不合法）。与 [entity.md](entity.md) 的 `YGG-PKG-ENTITY-*` 同风格。

## 5. 性能预算

**口径声明（先读再读数字）**：下表数字产在 **selftest 面**（普通 Lua 5.4，全标准库）——沙箱里既没有 `os.clock` 也没有 `collectgarbage`（`crates/ygg-sandbox/src/sandbox.rs` 的白名单测试钉死 `math` / `string` / `table` / `print` / `collectgarbage` 全为 `nil`），所以计时与内存只在测试侧可测，而包本身连 `math` 都不可达。三次取中位、批内三次取中位，`print` 面即口径（同 [procgen.md](procgen.md) §5 的方法论）。

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`（`install` 只做表组装，不注册内置算法）。清掉 `package.loaded["pkg.assetgen*"]` 后 `require` + `install`，5 次取中位 → 打印 `perf.assetgen.load_ms` |
| 实体按 id 查找 / 更新 | `O(1)` | **不适用**：无实体表。唯一的表访问是算法表直取（`ALGOS[name]`）与参数表按名取键，与素材数量无关是构造性的 |
| 序列化吞吐 | 附基准集与耗时 | 等价面：**PCM 装箱 ≥ 300k 样本/秒**（[codec.md](codec.md) 的树内基准是 2.77 MB/s 文本编码；PCM 每样本 2 字节 ≈ 600 KB/s，按 300k 样本/秒设阈留 2× 余量）。10 万帧单声道 `wav16` 三次取中位 → `perf.assetgen.pcm_samples_per_s` |
| 10k 实体常驻内存 | 给出数字 | 不适用（无实体）。等价口径：**64×64 RGBA 纹理 ≤ 128 KiB**（行块拼接：只有成品串 + 64 个行串，不留整张像素数组）、**1 s @22050 单声道 ≤ 2.5 MiB**（s16 数组 44100 个数 + 成品串 44 KB）。`collectgarbage("count")` 差 → `perf.assetgen.tex64_mem_kib` / `perf.assetgen.sound1s_mem_kib` |
| 泄漏检测 | 100 轮不增长 | 同 seed 100 轮：`bytes` 100 次全等 + 内存差 ≤ 1%。`perf.assetgen.leak_100_equal` / `perf.assetgen.leak_gc_pct` |
| **生成耗时 per 类**（P2.5 硬要求） | 无外部阈值，给数字 | 纹理 64×64 `ch=4` fbm 3 倍频 `≤ 10 ms`；几何 torus 24×12 `≤ 15 ms`（含 OBJ 文本）；音频 1 s @22050 osc + 包络 `≤ 12 ms`；16 件 bundle `≤ 200 ms`。各三次取中位 → `perf.assetgen.tex64_ms` / `mesh_torus_ms` / `sound_1s_ms` / `bundle16_ms` |
| **字节一致（真验收）** | 两次独立进程 sha256 相等 | 必达；两进程各跑 `test/pkg-assetgen-digest.lua` → 宿主 `cmp` + `xxd -r -p \| shasum -a 256`（§7） |

> **实现纪律**：目标值在首个实现提交时就地改成实测值；**偏差即回写本表**（不回写 = 文档与实现不符 = Goal 反例 6）。超阈不得靠「调小默认尺寸」蒙过去：阈值的口径是「64×64 / 1 s / 16 件」，尺寸是定义的输入，不是可调旋钮。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| Saltline（单文件 3D FPS，零素材） | 全套：地表 / 金属 / 涂装噪声纹理、道具网格（轮胎堆 / crate 堆 / 油桶 / 围栏 = 基元 + 组合变换）、枪声 / 脚步 / 命中音效 | `texture.fbm` + 调色、`mesh.*` + `mesh.compose`、`sound.*`（振荡器 / 模态 / 分层）+ WAV；一条 seed → 三条流（`assetgen.streams`） | P3 |
| CDDA（字符渲染，JSON 数据驱动，零素材） | 16×16 瓦片（噪声 / 砖 / 渐变底纹）、音效包（打击 / 脚步 / 环境） | `texture.*`（`w=h=16`，7 个 algo 都可用）+ `sound.noise` / `sound.partials` | P3 |
| DCSS（字符界面 + webtiles） | 32×32 地砖与物件底纹、法术音效 | `texture.bricks` / `texture.voronoi` / `texture.rings` + `sound.osc` 扫频 | P3 |
| PokeRogue（TS 图形化） | 道具 / 徽章图标（**几何 → 栅格**）、属性克制 / 命中音效、战斗背景底纹 | `mesh.*` 基元 + `assetgen.raster`（角度 / 缩放 / 填充色）+ `sound.mix` 分层 | P3 |

**几何的第二个用点**（这是包级 ≥2 闸最脆的一环，单列）：3D 网格只有 Saltline 一个确定用点；**第二个用点是 PokeRogue 的 2D 图标**——`assetgen.raster` 把同一套基元与变换正交投影后填充成纹理。这是本包对「≥2 个不同游戏用点」硬闸的答复：几何的**形状**（顶点 + 面 + 包围盒）在两种投影下都是同一份数据，栅格化只是它的一种消费方式。

## 7. 自测与验收

- **用例名前缀**：`pkg.assetgen.`；perf 用例名 `pkg.assetgen.perf`。
- **进哪条命令的计数**：`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-assetgen` 的 `cases passed=` = **28**；全量 = **153**。
- **用例清单（28 条）**：纹理 `texture-same-seed-same-bytes` / `texture-diff-seed-diff-bytes` / `texture-size-and-shape` / `texture-pattern-exact` / `texture-gradient-endpoints` / `texture-voronoi-tiebreak` / `texture-caps`；几何 `mesh-primitives` / `mesh-compose-order` / `mesh-ints-only` / `mesh-obj-text` / `mesh-caps`；栅格 `raster-fill` / `raster-input`；音频 `sound-same-seed-same-bytes` / `sound-wav-header` / `sound-peak-cap` / `sound-envelope-decay` / `sound-mix-sum` / `sound-waveforms` / `sound-caps`；跨类 `bundle-order-and-keys` / `stream-isolation` / `no-float-static` / `rng-not-copied` / `leak-100-rounds` / `perf`；跨进程 `cross-process-canon`（载体 `test/pkg-assetgen-digest.lua`）。
- **两条 mod 侧载体冒烟不计数**（file 级 `error` ⇒ `file FAIL` ⇒ `ok=0`）：`tests/pkg-assetgen-smoke.lua`（钉装载面 / 顶层纯度 / `ERRORS` 码集 / 导出面清单 / 十个 `perf.assetgen.*` 打印键名）与 `tests/pkg-assetgen-digest-smoke.lua`（钉 canon 行格式：`pkg.assetgen.lengths=` 与 `pkg.assetgen.hex=` 各一行、单行、无空值——空值与多行会让 `cmp` 静默通过）。
- **为什么必须有 mod 侧同名文件**：`--filter` 对 mod 的 `tests/*.lua` 与包的 `test/*.lua` **同时**过滤，mod 侧筛完为空即「没有可执行的测试文件」exit 1（[README.md](README.md) §5 的已落地纪律）。
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

  这条链**不写进任何脚本文件**（`AGENTS.md` §1.7 禁「用脚本携带约束」）：它是文档里的一段可粘贴命令，证据是它的输出。
- **禁止宽松断言**：只断「两次长度相同」不逐字节 `==`，视同没测 I1；只断「WAV 头能构造」不逐字段对表，视同没测容器。
- **人工抽验（一次）**：把 digest 链产出的 WAV / BMP 用系统 `file` / `afinfo` 看一眼（`RIFF (little-endian) WAVE audio, 16 bit`、`PC bitmap, Windows 3.x format, 24 bit`），这是「容器算对了」的第三方视角，不替代断言。
- **已知缺口（如实登记，不进本包交付）**：① `pkg.*` 未在真沙箱里跑过，且已有可复现隐患——`packages/libs/codec/init.lua` 与 `packages/libs/entity/init.lua` 调用 `math.huge` / `math.type`（沙箱里 `math` 是 `nil`）。本包只能做到「自己不踩」：`math.` 进静态禁列，整型断言只写在测试侧；② 跨平台（x86_64 ↔ aarch64）字节一致无实测，本阶段只有结构性论证（产物路径纯整数，不依赖 libm / FMA / 舍入模式）。

## 8. 设计依据

**为什么素材生成是包而不是 mod / 内核。** 四个复现的共同需求是「零外部素材」；各自实现 = 四份噪声、四份 WAV 头、四份波形表（Goal 反例 2 判定这种「复现里自造基建」为未完成）。而它**不能**进内核：纹理 / 音效的参数化只需要整数与字节，没有 I/O、没有事件，落进 `crates/**` 只会让「调一个 octaves」变成重新编译。

**为什么与 procgen 共享的是 rng 而不是算法。** [procgen.md](procgen.md) §1 已把素材二进制划给本包，同时声明「共享 rng 契约」。两者的产物形状证明这条划线是对的：`cells` 是 0..255 的地图格（带自己的 `algo`/`v` 与加权调度），`pixels` 是 `w*h*ch` 的连续字节、`pcm` 是 s16 交织——把 `terrain.noise` 拉进纹理管线要么多一层 cells→pixels 转换，要么把纹理参数塞进 def 注册表（参数随素材变化 → 同名 def 冲突报 `-DEF`）。共享 PRNG 是零成本的（一行 `require`），共享算法是负成本的。

**为什么产物路径必须整段整数域。** 浮点跨进程一致性无法结构性保证（编译器 FMA 收缩、libm 尾位差、`-ffast-math`），而「同 seed 同产物字节一致」是本阶段的硬验收。整数域下这条不变式是**构造性成立**的，不需要任何论证成本。四条支撑：① 沙箱 globals 里 `math` / `string` / `table` 全为 `nil`（`crates/ygg-sandbox/src/sandbox.rs` 的逐名断言），三角与开方只能用自备整数核（`mathx.lua`：Q15 有理近似 + 整数牛顿开方）；② Lua 5.4 的 `+ - * // % & | ~ << >>` 在 `integer` 上精确，`/` 是唯一的浮点除法，静态用例把它禁掉；③ 三道检查（静态扫字面量、运行时 `math.type` 断言、两进程 sha256）；④ 残余风险如实声明——跨平台一致在本阶段是**论证**不是实测。

**反方案一：用 `math.sin` / `math.sqrt` / `math.floor`。** 沙箱里 `math` 是 `nil`，包代码必须在沙箱方言里可跑。副作用是**好事**：这条约束顺手把浮点赶出了产物路径。

**反方案二：宿主提供 PNG / 音频编码 API。** 那要动 `crates/**`（内核能力），且宿主目前没有任何图像 / 音频面；44 字节 WAV 头与 54 字节 BMP 头写几十行纯 Lua 就够，还顺带拿到「字节即证据」。

**反方案三：`codec.encode` 造字节。** JSON 文本不是素材容器；`codec` 的编码口径是为**可读存档**设计的（键序、转义）——二进制进它会走控制字符转义（每字节最多六个字符），低字节直通又依赖解码侧容忍度。本包不复用它，也不新增第二份 JSON 编码。

**反方案四：把正弦表写成 256 项字面量。** 可行但吃掉源码预算、且没有任何确定性收益（Q15 有理近似同样整数精确、误差 ≤ 0.2%，在 8–48 kHz 的 SFX 上不可听）。选近似。

**判据（①②③）。** ① 阻断——把游戏规则（哪张贴图给哪个瓦片）写进包；用 `math.random` 或时钟取熵；产物路径出现浮点；包内实现第三份哈希 / 编码；按资源名分支。② 症状——同 seed 两次生成的文件 sha256 不同；改一个纹理参数导致音效字节变化（流未隔离）；`.wav` 打不开或时长错（头字段算错）；`pkg.assetgen` 在沙箱里 `nil`-index 崩（用了 `math.`）。③ 确认——本文件被 [../README.md](../README.md) 索引；`--filter pkg-assetgen` 的 `cases passed=28`；§5 的 `perf.assetgen.*` 键齐备；两进程 canon / `shasum` 相等。
