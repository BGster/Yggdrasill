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
