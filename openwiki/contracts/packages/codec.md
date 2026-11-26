---
type: "Reference"
title: "codec — 确定性 JSON 编解码与存档往返"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c29ecd25f3a58b8ec34190bd
    resource: repo://crates/ygg-sandbox/src/codec_face/digest.rs
  - id: openwiki-source-cd807cbd878562b9c3de0ae9
    resource: repo://packages/libs/codec/bundle.lua
  - id: openwiki-source-a290300e3bf74f830e8042ef
    resource: repo://packages/libs/codec/importing.lua
  - id: openwiki-source-91627f49d3cbeae3f3410363
    resource: repo://packages/libs/codec/jsonenc.lua
  - id: openwiki-source-987463d7e7c2db1a48eebd3f
    resource: repo://packages/libs/codec/runtime.lua
  - id: openwiki-source-22a115f47f80affe925e654e
    resource: repo://packages/libs/codec/sha256.lua
  - id: openwiki-source-5e2fbc6bc363b78613c0fbfb
    resource: repo://packages/libs/codec/test/pkg-codec.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# codec — 确定性 JSON 编解码与存档往返

> **id** packages/codec · **模块** ⑤ packages · 通用能力层 · **触发** 改 bundle 形状、键序规则、导入冲突策略或 checksum 口径之前
> **读不到代价** irreversible（存档字节一旦不可复现，旧档无法还原，且无法证明「同输入同产物」）
> **replaces**：DESIGN §4 codec（搬运基线 `3a2536f`）
> **上游**：[README.md](README.md) · **相关**：[entity.md](entity.md)（记录形状）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.kv` / `ygg.fs.write` / `ygg.codec`）· [../kernel/state-surface.md](../kernel/state-surface.md)（§2.8 存储契约）

## 1. 边界（这个包不做什么）

- **不定义存储层**：实体持久化只有一条路——mod KV 命名空间 + codec，不引入第二套存储。
- **不定义导出落点**：写盘经 `ygg.fs.write`（挂载桥，路径狱在宿主侧）；包不拼路径、不绕过挂载点。
- **不含游戏规则**：什么时候存档、存哪几个前缀、重命名后谁引用谁——是 mod 的语义。
- **不做版本迁移**：跨 `codec_version` 的字段搬迁归 mod 数据或一次性补丁；本包只**拒绝**未知版本，不猜。
- **不复制实体表示**：记录形状以 [entity.md](entity.md) §2 为唯一来源。

两条宿主事实决定了本包的形状：① 宿主**没有** `ygg.json.*`，所以确定性 JSON 编码器写在包里；② `ygg.kv` **只有 `get` / `set`**，没有遍历也没有删除（[../kernel/state-surface.md](../kernel/state-surface.md) §2.8），所以全量导出依赖调用方**显式点名**的键列表——包不扫键名，也不接受「前缀」这个说法（前缀是遍历的别名，而宿主没有这把刀）。

## 2. 数据形状

bundle（导出物，也是导入物的唯一形状）：

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `codec_version` | string（`"1"`，编码算法随版本） | 是 | — | bundle 顶层 |
| `exported_from` | string（**调用方传入的 `ns.mod`**） | 是 | `""` | bundle 顶层 |
| `entities` | array<record>（record 见 [entity.md](entity.md) §2） | 是 | `{}` | bundle 顶层 |
| `kv` | table<key, scalar> | 否 | `{}` | bundle 顶层；**只含调用方显式点名的键**（D9） |
| `checksum` | string（规范化字节的 sha256 hex；`""` = 未封签） | 是 | `""` | bundle 顶层 |

`namespace` 入参（导出侧）：`{ entities = true\|false, ids = <id 列表\|"index">, keys = {…}\|nil, mod = "<名>" }`。`ids = "index"` 表示全量（走 `pkg.entity` 的 `ids()` 面，升序）；给数组则只导那些 id（仍排序）。

**两个字段为何不能照 P0 版写（D9 / D10）**：

- `exported_from` 写成「mod 名 + pack 内容哈希前 8 位」不可实现：宿主没有「当前 mod 名」面，也没有「pack 内容哈希」面。改成**调用方传入** `ns.mod`，缺省空串。
- `kv` 写成 `table<prefix, table<key, value>>` 也不可实现：读一个前缀的子表需要**遍历键名**，而 `ygg.kv` 只有 `get` / `set`。改成扁平 `table<key, scalar>` + 调用方点名的 `keys` 数组；未点名的键不进档。

## 3. 不变量

- **I1 确定性字节**：同一组记录两次导出逐字节一致——对象键按字节序排序、数字用最短往返表示、无多余空格、无尾随逗号。
- **I2 幂等导入**：同一 bundle 导入两次，实体数与属性值不变（`overwrite` 下第二次恒等，`skip` 下第二次零写入）。返回体 `created` 数必须为 0。
- **I3 冲突策略显式且必选**：`import(bundle, { on_conflict = "skip"\|"overwrite"\|"rename" })`。**没有缺省值**：`opts` 缺失、`on_conflict` 缺失或取值非法一律 `YGG-PKG-CODEC-CONFLICT`。（P0 版同时写「缺省 `skip`」与「缺失即报错」，取后者——退路默认值会让一次误调用静默丢掉一整份档的差异。）
- **I4 校验和先行**：封签的 bundle（`checksum` 非空）导入前先算 checksum，不符即拒 `YGG-PKG-CODEC-CHECKSUM`，且**不做部分导入**——要么全部落地，要么一条不写（用例用 mock 的写计数为证）。
  - **未封签是显式第二态**：`checksum = ""` 表示「从未封过」（`export` 刚产出的内存态），不是「校验通过」。封签不在 `export` 里做：那会把整份 bundle 再哈希一遍——导出侧最贵的一步。越过信任边界的档一律经 `read`（从盘上读，必封必校）。`import` 返回体的 `verified` 把这个区别显式交给调用方，而不是让空串默默当「验过了」。
- **I5 不做删除**：codec 不清除既有记录，也不发明 tombstone；记录清除属宿主 KV 契约。
- **I6 引用不重写**：`rename` 策略下 `type = "ref"` 的属性**不**自动重映射（谁引用谁是游戏语义）；codec 只在返回体 `refs` 里报「N 个 ref 指向被重命名的 id」。
- **I7 语料边界**：`encode` 拒绝 `function` / `thread` / `userdata` / 非法 `nil` 洞 / 数组与字符串键混用 / 非整数或非正下标 / NaN / ±∞ / 循环引用，报 `YGG-PKG-CODEC-TYPE`；不静默降级为 `null`。带元表的表可以编码（元表不进字节）。
- **I8 顶层纯度**：包模块顶层只有纯导出；`install` 不预读、不预写任何 KV 键。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `codec.encode` | `(value)` | string（确定性 JSON） | 纯 |
| `codec.decode` | `(text)` | value（非法输入报错，带 `at byte N`） | 纯 |
| `codec.sha256` | `(bytes)` | string（64 位小写 hex） | 纯 |
| `codec.export` | `(namespace)` | bundle（`checksum = ""` = 未封签） | 纯读 |
| `codec.checksum` | `(bundle)` | string | 纯 |
| `codec.write` | `(path, bundle)` | `{ bytes, sha256 }`（sha256 = 包内算法） | 写盘（`ygg.fs.write`） |
| `codec.read` | `(path)` | bundle（必校 checksum） | 纯读（`ygg.fs.read`） |
| `codec.import` | `(bundle, opts)` | `{ created, skipped, overwritten, renamed, refs, verified, kv }` | 写 + 事件（经 [entity.md](entity.md) API） |
| `codec.roundtrip` | `(path)` | `{ bytes_in, bytes_out, equal }` | 写盘 + 纯读；自测与运维用 |

`import` 返回体的四个非计数项：`renamed`（旧 id → 新 id）、`refs`（指向被重命名 id 的 ref 属性数）、`verified`（本次是否真校过 checksum）、`kv`（bundle 的 `kv` 段**原样带回**——单写者纪律归 mod，包不写回）。

**sha256 来源（D11）**：包内自算（`pkg.codec.sha256`，纯 Lua FIPS 180-4），**不是**宿主值。宿主 `ygg.fs.write` 回的 `sha256` 是它对写出文本的记录，**不做互校**（S16）——`checksum(bundle)` 是对 `checksum=""` 的规范化编码求的，写出文本含 `checksum=<digest>`，两份字节永不相同；写侧只校**回执字节数**（截断 / 半写 → `YGG-PKG-CODEC-CHECKSUM`），内容面由 `read` 的强制 checksum（I4）在越信任边界时复核。不重哈希写出文本互校的理由：纯 Lua sha256 ≈0.8 MB/s，每写一遍多 90 ms / 66 KiB，等于把存档耗时翻倍。

### 4.1 下沉边界（原生加速面与回退）

序列化的**字节产出**（`encode` / `decode`）与 `sha256` 下沉到宿主面 `ygg.codec.*`（[../kernel/lua-api.md](../kernel/lua-api.md) §3.1.4，实现 `crates/ygg-sandbox/src/codec_face/`）。包内 Lua 实现**不下沉、不删**：它是**规范执行体**，原生面是它的**符合实现**。唯一判据是**逐字节相等**，不是「功能一致」。

| 面 | 去向 | 理由 |
| --- | --- | --- |
| `encode` / `decode` 的字节产出 | **下沉**（面在场时由原生算） | 逐节点成本 `964 ns/节点`（10k 实体实测），是解释器常数项；原生同遍历 `22 ns/节点` |
| `sha256` | **下沉**（S19，`ygg.codec.sha256`） | 另一条热路径；**按键独立退化**——包侧只探 `face.sha256` 为 function，旧宿主只有两键时自动落回纯 Lua |
| `export` | 不动 | 成本在逐实体 `pkg.entity` 面调用，属 Tier 2 布局 |
| `checksum` / `write` / `read` / `import` / `roundtrip` | 不动（内部那次 `encode` 顺带变快） | S16 已修 `write` 的 sha256 互校 → 校回执字节数 |

**委托与回退**：面在场 ⇒ `pcall` 调原生，成功即返回；面缺席 ⇒ 直接走 Lua 路径。**缺席**是既有契约（裸沙箱只有 `ygg = { engine_version }`，`packages_sandbox.rs` 断言该路径字节）；**在场**由 `--test codec_face` 与 `--test codec_bench` 断言（键集闭集、同面 before/after、差分字节相等）。

**两个错误码的边界**：`YGG-CODEC-UNSUPPORTED`（值域拒绝，或值里出现带元方法的表）⇒ 包**回退 Lua 重跑**，于是错误文案只有一处来源；`YGG-CODEC-INTERNAL`（原生内部失败）⇒ 包报 `YGG-PKG-CODEC-HOST` 并**不回退**（缺陷必须响，静默降级会把 bug 藏成「性能偶尔不达标」）。带元方法的表整值回退是刻意的：Lua 的 `pairs` 与 `t[i]` 分别会走 `__pairs` / `__index`，复刻它们等于「编码过程中执行 Lua 代码」。

**差分门禁（改任一侧都必须跑）**：`cargo test -p ygg-sandbox --test codec_face`。语料 = 行为用例的全部值形状 + 边界集（`1` vs `1.0`、`-0.0`、NaN/±inf、深度 64/65、环、`\u0000`/`\u007f`、代理对、非法转义、残留字节）+ 10⁴ 实体集 + 2 000 个 fuzz 值；判据**双向**：`native_encode(v) == lua_encode(v)` 逐字节，且两条 decode 深度等价（含数字子类型）。附加：合法语料直调 `codec_face::encode/decode` 必须 `Ok`——否则「把合法值误判为非法」会让包静默回退，字节相等看不出来，只丢性能。

**与 Tier 1 快照编码器的关系**：复用它的**规范序纪律**（键 / 名按 UTF-8 字节序升序），**不**复用容器：`ygg-sim` 无 mlua、`src/**` 禁浮点 token（JSON 数字最短往返必须有 `f64`）、快照是带 magic 与尾 digest 的二进制列帧（本包是 JSON 文本，是存档兼容面）。两者一致性用**同一份 fixture** 钉住（`crates/ygg-sim/fixtures/canonical-order.json`，两个消费者各断言一次）。

**判据**：① 阻断——只改一侧就交付；用二进制容器换吞吐（破存档兼容）；把 `UNSUPPORTED` 改成静默返回半截字节；② 症状——同输入两条路径字节不同、带 `__pairs` 的表编码漂移、旧档 `read` 报 `CHECKSUM` 不符；③ 确认——`--test codec_face`（差分全绿 + 无误拒）、`--test codec_bench`（`codec_native.encode_mbps ≥ 30` 且同面打印 `codec_lua.*`）、`pkg-selftest` 全量绿。

## 5. 性能预算

**本包是序列化吞吐的唯一口径定义处**：基准集 = 10k 实体（每实体 8 属性 + 3 tag）+ KV 前缀 `quest` / `dlg` 各 1k 条；同机器、跑三次取中位。所有数字产在 **selftest（普通 Lua）**——沙箱内既无 `collectgarbage` 也无 `os.clock`，所以内存行是**包侧 Lua 堆上界**（含 mock 开销），不是宿主 Rust 堆。复算：`--filter pkg-codec`（三次取中位）。

| 指标 | 阈值 | 本包实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 实测 **1.14 ms**。测法：两个文件各 `loadfile` ×100 + `install`，`os.clock` 计时取每轮均值。 |
| 实体按 id 查找 / 更新 | `O(1)` | 不适用：本包不做 id 索引，写实体一律经 [entity.md](entity.md) 的 O(1) 面。 |
| 序列化吞吐（实体/秒，双向） | 附基准集与耗时 | 导出 **146,090 实体/秒**（68.5 ms）、导入 **3,573 实体/秒**（2799 ms，其中约 1900 ms 是全量 sha256 复核——I4 的代价）。encode 单独计时 **1.593 MB / 574.8 ms = 2.77 MB/s**。门禁阈值：导入 ≥ **1500 实体/秒**（`pkg-codec-perf`，S27 由 3000 有据放宽）。 |
| 10k 实体常驻内存 | 给出数字 | 导出侧 **11,917.1 KiB**（上界）+ bundle 文本 **1,670,876 字节**。目标写的 `≤ 8 MiB` **超了**：上界里两侧都是 Lua 表 / 字符串（mock KV 的 20 万标量键 + 中间表 + 1.6 MB 文本），沙箱硬帽是 64 MiB（`SANDBOX_MEMORY_LIMIT`）。这个数与「沙箱内 Rust 侧峰值」不可比。 |
| 泄漏检测 | 100 轮不增长 | 100 轮 `export → import(skip)` 后 `count` / `#ids` / KV 键数**全等**，内存 △**0.001%**（断言 ≤ 1%）。`skip` 是零写路径，所以结构全等是能断的硬不等式。 |

**before/after（首版两个缺陷都是自己写的，不是环境）**：

| 环节 | 首版 | 修正后 | 倍率 | 根因 |
| --- | --- | --- | --- | --- |
| `encode` | 0.04 MB/s（43111 ms / 1.6 MB） | **2.77 MB/s**（574.8 ms） | **69×** | ① 拼接用逐段 `..`（O(n²)）；② 转义逐字节 `sub(i, i)` 产出百万级片段 |
| `import` | 173 实体/秒 | **3,573 实体/秒** | **20×** | 同上（checksum 先 encode 一遍，两缺陷叠加） |

**下沉前后的同面 before/after**（两个测面必须分清，否则数字没有意义）：

| 测面 | 口径 | Lua 径 | 原生径 | 倍率 |
| --- | --- | --- | --- | --- |
| **契约面**（普通 Lua，`--example selftest`，debug 构建） | 本页基准集 | encode **2.48 MB/s**（642.8 ms）、decode **1.91 MB/s**（832.3 ms） | 无（面只在宿主里） | — |
| **运行面**（真沙箱，`cargo test --release -p ygg-sandbox --test codec_bench`） | 同 10⁴ 实体 + 2k KV、同进程、三次中位 | encode 7.79 / decode 7.19 MiB/s | encode **60.92** / decode **59.57 MiB/s** | **7.82× / 8.29×** |
| **sha256** | 运行面同口径；契约面 = `PERF codec.sha256`（64 KiB 样本） | 运行面 **4.20 MiB/s**（379.0 ms）；契约面 0.771 / 0.779 MB/s | 运行面 **239.12 MiB/s**（6.7 ms） | **56.87×**（运行面） |

三条必须一起读的结论：① 绝对判据过线（原生对 ≥30 MiB/s 各留 2.0× 余量）；②「≥10×」的基准面要说清——2.77 MB/s 是 **debug** 读数，同一实现 release 下本来就 7.79 MiB/s，所以对 debug 基线是 22×、对同面基线是 7.8×/8.3×；③ 回退路径没有退化（契约面 encode 2.34 → 2.48，来自 `sort_names` 的 O(n²)→O(n log n)）。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| CDDA | JSON 定义 → 实体表 | `decode` 把数据文件读成实体表并 `spawn`，同一个编解码器读旧档 | P3 |
| PokeRogue | 存档 / 读档 | `export` + `write` 落挂载点；`read` + `import` 还原（幂等，读档两次不翻倍） | P3 |
| DCSS | 存档跨 run | `checksum` 作为「档没被改过」的证据，不符即拒而不是半读 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.codec.`；perf 用例名前缀 `pkg.codec.perf-`。
- **断言面**：I1 用**逐字节相等**断言两次 `encode` 输出（且用两种插入序的 100 键对象对拍）；I2 用两次导入后的记录快照相等；I4 / I5 / I7 断拒绝路径；I6 用「重命名后 ref 仍指向旧 id + 返回体计数正确」；sha256 用**已知答案向量**（空串 / "abc" / 55 / 56 / 64 / 65 字节 / 448-bit 双块），向量与 `shasum -a 256` / `hashlib` 对拍。
- **计数**：`--filter pkg-codec` → **21** 条（2026-11-26 实测）；pkg-selftest 全量 **199**。生产 Mod（agent-seed）不声明包，计数保持 **79/0**（D14）——这就是回归钉。
- **perf 文件不计数**：`pkg-codec-perf.lua` 用 file 级断言（`error` ⇒ `ok=0`），不进 `cases passed=`。
- **`--filter` 的硬约束**：它同时对 mod 侧与包侧文件过滤，「一处都不匹配」会以「没有可执行的测试文件」直接 exit 1。所以按包跑只能用 `--filter pkg-codec`（由 `pkg-codec-smoke.lua` 接住）。
- **禁止宽松断言**：只断「能解析」不断字节相等，视同没测 I1；只断「算出 64 位 hex」不向量对拍，视同没测 sha256（首版两处 bug 都是向量抓到的：长度尾字节生成写错、K 表第 5 个常量手抄掉位）。
- **真沙箱面不计入 `cases passed=`**：包能否在**无 `inject`** 的裸沙箱里装载并编码，由 `--test packages_sandbox` 的用例 ② 钉住（输出与 driver 路径同字节）。

## 8. 设计依据

**为什么编解码是包而不是塞进 entity。** 序列化要回答两个独立问题：形状（谁是字段、谁是引用）与字节（同样输入能否产出同样输出）。前者归 entity，后者归 codec；混在一起时，任何为了「好看」的键序调整都会变成实体契约的变更。

**反方案**：① 引入宿主 JSON API——宿主当前没有 `ygg.json.*`，加一个就是改 `crates/**` 的内核能力；包内实现还让确定性规则可被逐字节断言。② 二进制序列化——更快更小，但不可 diff、不可人工查错，而存档排查现场往往只有一份文件。

**沙箱方言纪律（不回写就等着真实跑局炸）**：沙箱 globals 里没有 `string` / `table` / `utf8` / `coroutine` 表，而 `selftest` 跑在**普通 Lua** 上——「selftest 绿、沙箱 nil-index 崩」的写法测不出来。两条硬纪律：① 字符串只能走**方法语法**，排序 / 拼接手写；② 交付门禁带静态 grep（`packages/libs/codec/*.lua` 里 `string.` 与 `table.sort` 之类必须 0 行，注释里也不写）。`utf8` 不可达还多一条具体后果：`\uXXXX` 的 UTF-8 编码（含代理对）得手写四档。

**性能硬教训（写在这里是因为它会重犯）**：Lua 字符串不可变，`out = out .. part` 写在循环里就是 O(n²)：1.6 MB 的 bundle 跑了 **43 秒**。逐字节 `sub(i, i)` 拼接则为同一份文本产出**百万级片段**。两处改掉之后 encode 到 2.77 MB/s（69×）。结论：在这个沙箱里，「看上去无害的字符串相加」是本包第一个要怀疑的对象。

**为什么「同一份实现」变成两份而规范仍只有一份（P-K A2，2026-11）**：下沉载体是宿主面 `ygg.codec.*` 而不是 Tier 1 快照编码器（`ygg-sim` 无 mlua、禁浮点 token、快照是二进制列帧）。「Rust 一份 + Lua 一份」是结构事实，不构成第二条规范靠三条纪律：① 规范唯一处仍是本页 I1 与 `init.lua` 导出的四函数（原生面被定义为**符合实现**）；② 判据是逐字节相等且由差分用例钉住；③ 原生面拒绝的输入回退 Lua 重跑，错误文案只有一处来源。换二进制容器的反方案在计划稿里算过账：内存里再快 2–3×，但存档格式换代、旧档全部不可读，**不值得**。

**Lua 侧模块化拆分（S-25 批十一）**：原单文件 `init.lua` 1206 行（800 上限的 1.5×）拆为 6 文件：`init.lua`（装配与转口，12 键逐键不变）+ `runtime`（错误码 / `fail` / `CODEC_VERSION` / deps 记位，共享单源叶子）+ `jsonenc` + `jsondec` + `bundle`（checksum / export / write / read / roundtrip）+ `importing`（两阶段 import），`sha256.lua` 先例不动；依赖无环 `runtime ← {jsonenc, jsondec} ← bundle ← importing`。这是**纯搬移**（函数体逐字节，残差 9 行全是授权差异）。两处装配层器件：deps 记位落 `runtime` 的 `installed` 座 + `M.deps` 别名；`bundle` / `importing` 的 `pkg.entity` 引用改为**调用期转发代理**，保住「清两键即重捕获」的消费者契约。

**sha256 原生下沉与按键独立退化（S19）**：① 落 codec_face 域而非新 face——摘要语义属于 codec 契约域，消费者全在这里；② 复用 `sha2`（RustCrypto）与 `hash.rs::hex_sha256`，不手写第二份；③ 包侧委托**按键独立**——只探 `face.sha256` 为 function，「半个面退成没有面」原则扩展到键粒度。正确性三层：NIST 向量 + 填充边界钉值 + 双径逐字节差分。**反方案**：返回 raw 32 字节（包侧还得补 hex，热路径白下沉）、streaming API（批式铁律：整串进出）。

**导入吞吐门禁 3000 → 1500（S27，有据放宽）**：① 指标构成——导入耗时由纯 Lua sha256 复核主导（是机器敏感而非算法敏感）；② 实证方差——跨时代漂移 3573 → 3189（−10.7%），30 样本区间 [3051, 3323]，对 3000 最差余量仅 **1.7%**，等于把门禁做成机器状态探测器；③ 侦测力保留——1500/s ⟺ 6667 ms 窗，codec-logic 段（~900–1200 ms）需退化 ≥4.5× 才漏检，而真退化形状（键序回退 O(n²)、逐实体 KV 往返）典型 ≥10×。取数法：**阈值 = max(基线最差观测 ÷ 2, 真退化侦测下限)**。其余 PERF 阈值余量 ≥1.86×，全部不动。

维护责任 = 改 `packages/libs/codec/**` 的执行者；触发 = 任一相关 commit；检查 = `codec_face` / `codec_bench` 差分 + `--filter pkg-codec` 计数。
