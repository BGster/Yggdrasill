# Yggdrasill · packages/codec — 确定性 JSON 编解码与存档往返

> **id** packages/codec · **模块** ⑤ packages · 通用能力层 · **触发** 改 bundle 形状、键序规则、导入冲突策略或 checksum 口径之前
> **读不到代价** irreversible（存档字节一旦不可复现，旧档无法还原，且无法证明「同输入同产物」）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [entity.md](entity.md)（记录形状）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.kv` / `ygg.fs.write`）· [../kernel/state-surface.md](../kernel/state-surface.md)（§2.8 存储契约）

## 1. 边界（这个包不做什么）

- **不定义存储层**：实体持久化只有一条路——mod KV 命名空间 + codec，不引入第二套存储（Goal §六）。
- **不定义导出落点**：写盘经 `ygg.fs.write`（P1 的 fs 挂载桥，路径狱在宿主侧）；包不拼路径、不绕过挂载点。
- **不含游戏规则**：什么时候存档、存哪几个前缀、重命名后谁引用谁——是 mod 的语义。
- **不做版本迁移**：跨 `codec_version` 的字段搬迁归 mod 数据或一次性补丁；本包只**拒绝**未知版本，不猜。
- **不复制实体表示**：记录形状以 [entity.md](entity.md) §2 为唯一来源。

两条宿主事实决定了本包的形状，取自有命令的取证：① 宿主**没有** `ygg.json.*`（`grep -n "ygg.json" docs/design/kernel/lua-api.md` 无命中），所以确定性 JSON 编码器写在包里；② `ygg.kv` **只有 `get` / `set`**，没有遍历也没有删除（[../kernel/state-surface.md](../kernel/state-surface.md) §2.8），所以全量导出依赖调用方**显式点名**的键列表——包不扫键名，也不接受「前缀」这个说法（前缀是遍历的别名，而宿主没有这把刀）。

## 2. 数据形状

bundle（导出物，也是导入物的唯一形状）：

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `codec_version` | string（`"1"`，编码算法随版本） | 是 | — | bundle 顶层 |
| `exported_from` | string（**调用方传入的 `ns.mod`**） | 是 | `""` | bundle 顶层 |
| `entities` | array<record>（record 见 [entity.md](entity.md) §2） | 是 | `{}` | bundle 顶层 |
| `kv` | table<key, scalar> | 否 | `{}` | bundle 顶层；**只含调用方显式点名的键**（D9） |
| `checksum` | string（规范化字节的 sha256 hex；`""` = 未封签） | 是 | `""` | bundle 顶层 |

`namespace` 入参（导出侧）：`{ entities = true|false, ids = <id 列表\|"index">, keys = {…}\|nil, mod = "<名>" }`。`ids = "index"` 表示全量（走 `pkg.entity` 的 `ids()` 面，升序）；给数组则只导那些 id（仍排序）。

**两个字段为何不能照 P0 版写（D9 / D10）**：

- `exported_from` 写成「mod 名 + pack 内容哈希前 8 位」不可实现：宿主没有「当前 mod 名」面，也没有「pack 内容哈希」面（`lua-api.md` §3.1 的表里不存在这两项）。包不能发明它们，否则导出物的字段会变成调用方的口算题——改成**调用方传入** `ns.mod`，缺省空串。
- `kv` 写成 `table<prefix, table<key, value>>` 也不可实现：读一个前缀的子表需要**遍历键名**，而 `ygg.kv` 只有 `get` / `set`。改成扁平的 `table<key, scalar>` + 调用方点名的 `keys` 数组；未点名的键不进档。

## 3. 不变量

- **I1 确定性字节**：同一组记录两次导出逐字节一致——对象键按字节序排序、数字用最短往返表示、无多余空格、无尾随逗号。这是 P2.5「同 seed 同产物」在序列化面的同一条纪律。
- **I2 幂等导入**：同一 bundle 导入两次，实体数与属性值不变（`overwrite` 下第二次是恒等操作，`skip` 下第二次零写入）。返回体的 `created` 数必须为 0。
- **I3 冲突策略显式且必选**：`import(bundle, { on_conflict = "skip"|"overwrite"|"rename" })`。**没有缺省值**：`opts` 缺失、`on_conflict` 缺失或取值非法一律 `YGG-PKG-CODEC-CONFLICT`。（P0 版同时写了「缺省 `skip`」与「策略缺失即报错」，两句矛盾；取后者——退路默认值会让一次误调用静默丢掉一整份档的差异。）
- **I4 校验和先行**：封签的 bundle（`checksum` 非空）导入前先算 checksum，不符即拒 `YGG-PKG-CODEC-CHECKSUM`，且**不做部分导入**——要么全部落地，要么一条不写（校验先于任何写，用例用 mock 的写计数为证）。
  - **未封签是显式第二态**：`checksum = ""` 表示「从未封过」（`export` 刚产出的内存态），不是「校验通过」。封签不在 `export` 里做：那会把整份 bundle 再哈希一遍——导出侧最贵的一步，而导出常常只为了看一眼或转存；越过信任边界的档一律经 `read`（从盘上读，必封必校）。`import` 返回体的 `verified` 把这个区别显式交给调用方，而不是让一个空串默默当“验过了”。
- **I5 不做删除**：codec 不清除既有记录，也不发明 tombstone；记录清除属宿主 KV 契约（[../kernel/state-surface.md](../kernel/state-surface.md) §2.8）。
- **I6 引用不重写**：`rename` 策略下 `type = "ref"` 的属性**不**自动重映射（谁引用谁是游戏语义）；codec 只在返回体 `refs` 里报「N 个 ref 指向被重命名的 id」，由 mod 决定怎么办。
- **I7 语料边界**：`encode` 拒绝 `function` / `thread` / `userdata` / 非法 `nil` 洞（数组中间缺项）/ 数组与字符串键混用 / 非整数或非正的下标 / NaN / ±∞ / 循环引用，报 `YGG-PKG-CODEC-TYPE`；不静默降级为 `null`。带元表的表可以编码（元表不进字节）。
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

`import` 返回体的四个非计数项：`renamed`（旧 id → 新 id）、`refs`（指向被重命名 id 的 ref 属性数）、`verified`（本次是否真的校过 checksum）、`kv`（bundle 的 `kv` 段**原样带回**——单写者纪律归 mod，包不写回，见 D9）。

**sha256 来源（D11 已定稿）**：包内自算（`pkg.codec.sha256`，纯 Lua FIPS 180-4），**不是**宿主值。P0 写的 FNV-1a 降级路径**退役**：纯 Lua sha256 已就位（已知答案向量见 §7），且 P1 挂载桥已落地。宿主 `ygg.fs.write` 回的 `sha256` 由「来源」降为「互校」——两者不一致即 `YGG-PKG-CODEC-CHECKSUM`，不选一个信。为何不能拿宿主值当来源：它只能**落盘之后**得到，而 `checksum(bundle)` 必须在落盘前就能算（导出侧要把校验和写进 bundle 本体），且 `read` 要在纯读路径上复核「盘上没被改过」。

### 4.1 下沉边界（原生加速面与回退）

序列化的**字节产出**（`encode` / `decode`）下沉到宿主面 `ygg.codec.{encode,decode}`（契约 [kernel/lua-api.md §3.1.4](../kernel/lua-api.md#314-codec-确定性-json-编解码面宿主注入)，实现 `crates/ygg-sandbox/src/codec_face.rs`）。包内 Lua 实现**不下沉、不删**：它是**规范执行体**（本文 I1 的可执行形态），原生面是它的**符合实现**。两者之间唯一的判据是**逐字节相等**，不是「功能一致」。

| 面 | 去向 | 理由 |
| --- | --- | --- |
| `encode` / `decode` 的字节产出 | **下沉**（面在场时由原生计算） | 逐节点成本 `964 ns/节点`（10k 实体实测），是解释器常数项；原生侧同遍历 `22 ns/节点` |
| `sha256` | 不动 | 另一条热路径（纯 Lua FIPS 180-4），已排期 S19 |
| `export` | 不动 | 成本在逐实体 `pkg.entity` 面调用，属 Tier 2 布局，已排期 S20 |
| `checksum` / `write` / `read` / `import` / `roundtrip` | 不动（内部那次 `encode` 顺带变快） | 与 S16 的 `write` 自校缺陷同一改动面；本包只承诺 `encode`/`decode` 两行数字 |

**委托与回退（包侧唯一改动，`init.lua` 的 `M.encode` / `M.decode` 两处）**：面在场 ⇒ `pcall` 调原生，成功即返回；面缺席 ⇒ 直接走 Lua 路径。两条都由测试钉住：
- **缺席**是既有契约（裸沙箱只有 `ygg = { engine_version }`，`crates/ygg-sandbox/tests/packages_sandbox.rs` 用例 ② 断言该路径的字节）；
- **在场**由 `cargo test -p ygg-sandbox --test codec_face` 与 `--test codec_bench` 断言（键集闭集、同面 before/after、差分字节相等）。

**两个错误码的边界**：`YGG-CODEC-UNSUPPORTED`（值域拒绝，或值里出现带元方法的表）⇒ 包**回退 Lua 路径重跑**，于是错误文案只有一处来源（本包的 `fail(...)`），不维护第二套字符串契约；`YGG-CODEC-INTERNAL`（原生实现内部失败）⇒ 包报 `YGG-PKG-CODEC-HOST` 并**不回退**（缺陷必须响，静默降级会把 bug 藏成「性能偶尔不达标」）。带元方法的表整值回退是刻意的：Lua 的 `pairs` 与 `t[i]` 分别会走 `__pairs` / `__index`，复刻它们等于「编码过程中执行 Lua 代码」。

**差分门禁（改任一侧都必须跑）**：`cargo test -p ygg-sandbox --test codec_face`。语料 = `pkg-codec.lua` 行为用例的全部值形状 + 边界集（`1` vs `1.0`、`-0.0`、NaN/±inf、深度 64/65、环、`\u0000`/`\u007f`、代理对、非法转义、残留字节）+ 10⁴ 实体集 + 2 000 个 fuzz 值；判据是**双向**：`native_encode(v) == lua_encode(v)` 逐字节，且 `native_decode(t)` 与 `lua_decode(t)` 深度等价（含数字子类型）。附加一条：合法语料直调 `codec_face::encode/decode` 必须 `Ok`——否则「把合法值误判为非法」会让包静默回退，字节相等看不出来，只丢性能。

**与 Tier 1 快照编码器的关系**：复用它的**规范序纪律**（键/名按 UTF-8 字节序升序，[kernel/sim-kernel.md §2](../kernel/sim-kernel.md) 的「复用排序规范 ≠ 复用文本容器」），**不**复用它的容器：`ygg-sim` 无 mlua（拿不到 Lua 值）、`src/**` 禁浮点 token（`f64` 数字格式化放不进去）、快照是带 magic 与尾 digest 的二进制列帧（本包的对外格式是 JSON 文本，是存档兼容面）。两者的一致性用**同一份 fixture** 钉住（`crates/ygg-sim/fixtures/canonical-order.json`，两个消费者各断言一次），而不是互相引用说法。

**判据**：① 阻断——只改一侧（原生或 Lua）就交付；用二进制容器换吞吐（破存档兼容）；把 `UNSUPPORTED` 改成静默返回半截字节；② 症状——同一输入两条路径字节不同、带 `__pairs` 的表编码结果漂移、旧档 `read` 报 `CHECKSUM` 不符；③ 确认——`cargo test -p ygg-sandbox --test codec_face`（差分全绿 + 无误拒）、`--test codec_bench`（`codec_native.encode_mbps ≥ 30` 且同面打印 `codec_lua.*`）、`pkg-selftest` 的 `cases passed=155 failed=0`。


## 5. 性能预算

**本包是序列化吞吐的唯一口径定义处**：基准集 = 10k 实体（每实体 8 属性 + 3 tag）+ KV 前缀 `quest` / `dlg` 各 1k 条；同机器、跑三次取中位。

**口径声明（先读再读数字）**：所有数字产在 **selftest（普通 Lua）**——沙箱内既无 `collectgarbage` 也无 `os.clock`（`sandbox.rs` 白名单测试钉死），所以内存行是**包侧 Lua 堆上界**（含 mock 开销），不是宿主 Rust 堆的读数。复算命令：
`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-codec`（三次取中位）。

| 指标 | 阈值（Goal 原则二） | 本包实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 实测 **1.14 ms**（目标 `≤ 1ms` 差同一量级）。测法（D13）：两个文件各 `loadfile` ×100 + `install`，`os.clock` 计时取每轮均值——P0 写的「空 run vs 含包起局的墙钟差」信号（≤ 1ms）低于整条链的启动抖动。 |
| 实体按 id 查找 / 更新 | `O(1)` | **不适用**：本包不做 id 索引，写实体一律经 [entity.md](entity.md) 的 O(1) 面。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | 实测导出 **146,090 实体/秒**（68.5 ms）、导入 **3,573 实体/秒**（2799 ms）——导出是目标的 **29×**，导入刚过线。**导入的构成必须看清楚**：2799 ms 里约 **1900 ms 是全量 sha256 复核**（I4 的代价），真正的落地动作只占 ~900 ms；所以这条数字的上限由**纯 Lua 哈希**决定（0.825 MB/s），而不是包的分派逻辑。完整的 encode 单独计时：**1.593 MB / 574.8 ms = 2.77 MB/s**。 |
| 10k 实体常驻内存 | 给出数字 | 实测导出侧 **11,917.1 KiB**（上界）+ bundle 文本 **1,670,876 字节**。目标写的是 `≤ 8 MiB`：**超了**，且超的原因可计——上界里两侧都是 Lua 表/字符串（mock KV 的 20 万标量键 + 中间表 + 1.6 MB 文本），而沙箱实际硬帽是 `64 MiB`（`SANDBOX_MEMORY_LIMIT`，P0 此处写的 16 MiB 已被 P1 调整）。这个数与「沙箱内 Rust 侧峰值」不可比；若要答那个问题，得在 `crates/**` 加观测面（不在本需求改动面）。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 实测 100 轮 `export → import(skip)` 后：`count` / `#ids` / KV 键数**全等**，内存 △**0.001%**（断言 ≤ 1%）。`skip` 是零写路径，所以结构全等是能断的硬不等式（与 entity 靠墓秃不同，这里没有 unavoidable 写）。 |

**before/after（Goal 反例 4 要求的前后对比）**：首版实现导出/导入均不达标，两个缺陷都是自己写的（不是环境）：

| 环节 | 首版 | 修正后 | 倍率 | 根因 |
| --- | --- | --- | --- | --- |
| `encode` | 0.04 MB/s（43111 ms / 1.6 MB） | **2.77 MB/s**（574.8 ms） | **69×** | ① 拼接用逐段 `..`（O(n²)）；② 字符串转义逐字节 `sub(i, i)` 产出百万级片段 |
| `import` | 173 实体/秒 | **3,573 实体/秒** | **20×** | 同上（校验和要先 `encode` 一遍，两个缺陷叠加在 import 路径上） |

修正手法：拼接改**两两合并**（对数轮）、转义改 pattern 一次跳到下一个需转义字节（C 层扫描）+ 整段原字一次取出、浮点精度试探的格式串提到模块级。

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| CDDA | JSON 定义 → 实体表 | `decode` 把数据文件读成实体表并 `spawn`，同一个编解码器读旧档 | P3 |
| PokeRogue | 存档 / 读档 | `export` + `write` 落挂载点；`read` + `import` 还原（幂等，读档两次不翻倍） | P3 |
| DCSS | 存档跨 run | `checksum` 作为「档没被改过」的证据，不符即拒而不是半读 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.codec.`（例 `pkg.codec.encode-deterministic-bytes`、`pkg.codec.import-idempotent`）；perf 用例名前缀 `pkg.codec.perf-`。
- **断言面**：I1 用**逐字节相等**断言两次 `encode` 输出（且用两种插入序的 100 键对象对拍）；I2 用两次导入后的记录快照相等；I4 / I5 / I7 断拒绝路径；I6 用「重命名后 ref 仍指向旧 id + 返回体计数正确」；sha256 用**已知答案向量**（空串 / "abc" / 55 / 56 / 64 / 65 字节 / 448-bit 双块），向量取独立实现（同机 `shasum -a 256` 与 `hashlib` 对拍）。
- **计数**：20 条行为用例计入 `cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-` 的 `cases passed=`（发现机制见 [README.md](README.md) §5）。生产 Mod（`agent-seed`）不声明包，计数保持 79/0（D14）。
  - **perf 文件不计数**：`pkg-codec-perf.lua` 用 file 级断言（`error` ⇒ `file FAIL` ⇒ `ok=0`），不进 `cases passed=`——这样「40 条」这个可复算计数不被 perf 污染。
  - **`--filter` 的硬约束**：它同时对 mod 侧与包侧文件过滤，「一处都不匹配」会以 `没有可执行的测试文件` 直接 exit 1。所以按包跑只能用 `--filter pkg-codec`（由 `pkg-codec-smoke.lua` 接住），不能单跑一个 perf 文件。
- **禁止宽松断言**：只断「能解析」不断字节相等，视同没测 I1；只断「算出 64 位 hex」不向量对拍，视同没测 sha256（本实现首版两处 bug 都是向量抓到的：长度尾的字节生成写错、K 表第 5 个常量手抄掉位）。
- **真沙箱面不计入 `cases passed=`**：包能不能在**无 `inject`** 的裸沙箱里装载并编码，由 `cargo test -p ygg-sandbox --test packages_sandbox` 的用例 ② 钉住（输出与 driver 路径同字节）；`cases passed=` 只数 Lua 用例，两者的口径不同，别拿一个的数字代替另一个（[README.md](README.md) §5「沙箱可装载面」）。

## 8. 设计依据

**为什么把编解码做成包而不是塞进 entity。** 序列化要回答两个独立问题：形状（谁是字段、谁是引用）与字节（同样的输入能否产出同样的输出）。前者归 entity，后者归 codec；混在一起时，任何为了「好看」的键序调整都会变成实体契约的变更。

**反方案一：引入宿主 JSON API。** 宿主当前没有 `ygg.json.*`，加一个就是改 `crates/**` 的内核能力——那是 P1 的载体选择问题（内核 vs 包），不该由序列化顺手推动。包内实现还带来一个好处：确定性规则（键序、数字表示）写在契约里，可被用例逐字节断言。**反方案二：二进制序列化。** 更快更小，但不可 diff、不可人工查错，而存档问题的排查现场往往只有一份文件——JSON 的字节数换可读性，是这一层的正确取舍。

**实现纪律（沙箱方言，不回写就等着真实跑局炸）。** 沙箱 globals 里没有标准库的 `string` / `table` / `utf8` / `coroutine` 表（`sandbox.rs` 白名单测试钉死），而 `selftest` 跑在**普通 Lua** 上——所以「在 selftest 里绿、在沙箱里 nil-index 崩」的写法测不出来。两条硬纪律：① 字符串只能走**方法语法**（`("x"):sub(1, 1)`），排序/拼接手写；② 交付门禁带静态 grep（`packages/libs/codec/*.lua` 里 `string.` 与 `table.sort` 之类必须 0 行，注释里也不写）。`utf8` 不可达还多一条具体后果：`\uXXXX` 的 UTF-8 编码（含代理对）得手写四档。

**性能上的一条硬教训（写在这里是因为它会重犯）。** Lua 字符串不可变，`out = out .. part` 写在循环里就是 O(n²)：1.6 MB 的 bundle 跑了 **43 秒**（0.04 MB/s）。同时，逐字节 `sub(i, i)` 拼接会为同一份文本产出**百万级片段**，内存与拼接两头都掌不住。两处都改掉之后 encode 到 **2.77 MB/s**（69×）。结论：在这个沙箱里，「看上去无害的字符串相加」是本包第一个要怀疑的对象；拼接只写在两两合并或先批量再合并的形态里。

**判据（①②③）。** ① 阻断——同一实体在两个游戏里用两套序列化；导入冲突策略隐式（靠 id 大小碰运气）；checksum 缺失导致半读的坏档被接受。② 症状——读档两次实体翻倍；同一份 bundle 在两台机器上字节不同导致「存档校验失败」查不出原因；坏档只报「某个字段是 nil」。③ 确认——本文件被索引；`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-codec` 的 `cases passed=` 含 20 条 `pkg.codec.*`；§5 吞吐表有 before/after 数字与复算命令。
