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

两条宿主事实决定了本包的形状，取自有命令的取证：① 宿主**没有** `ygg.json.*`（`grep -n "ygg.json" docs/design/kernel/lua-api.md` 无命中），所以确定性 JSON 编码器写在包里；② `ygg.kv` **只有 `get` / `set`**，没有遍历也没有删除（[../kernel/state-surface.md](../kernel/state-surface.md) §2.8），所以全量导出依赖调用方点名的索引键（`entity.index`），包不扫键名。

## 2. 数据形状

bundle（导出物，也是导入物的唯一形状）：

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `codec_version` / `exported_from` | string（`"1"`，编码算法随版本）/ string（mod 名 + pack 内容哈希前 8 位） | 是 | — | bundle 顶层 |
| `entities` | array<record>（record 见 [entity.md](entity.md) §2） | 是 | — | bundle 顶层 |
| `kv` | table<prefix, table<key, value>> | 否 | `{}` | bundle 顶层；只含调用方点名的前缀白名单 |
| `checksum` | string（规范化字节的 sha256 hex） | 是 | — | bundle 顶层 |

`namespace` 入参（导出侧）：`{ entities = true|false, prefixes = {"quest", "dlg"}, ids = <id 列表\|"index"> }`。`ids = "index"` 表示读 `entity.index` 全量；给列表则只导这些 id 的子树（细粒度导出）。

## 3. 不变量

- **I1 确定性字节**：同一组记录两次导出逐字节一致——对象键按字节序排序、数字用最短往返表示、无多余空格、无尾随逗号。这是 P2.5「同 seed 同产物」在序列化面的同一条纪律。
- **I2 幂等导入**：同一 bundle 导入两次，实体数与属性值不变（`overwrite` 下第二次是恒等操作，`skip` 下第二次零写入）。返回体的 `created` 数必须为 0。
- **I3 冲突策略显式且必选**：`import(bundle, { on_conflict = "skip"|"overwrite"|"rename" })`，缺省 `skip`（最保守）；`rename` 返回 `旧 id → 新 id` 映射，策略缺失即报错 `YGG-PKG-CODEC-CONFLICT`。
- **I4 校验和先行**：导入前先算 checksum，不符即拒 `YGG-PKG-CODEC-CHECKSUM`，且**不做部分导入**——要么全部落地，要么一条不写。
- **I5 不做删除**：codec 不清除既有记录，也不发明 tombstone；记录清除属宿主 KV 契约（[../kernel/state-surface.md](../kernel/state-surface.md) §2.8）。
- **I6 引用不重写**：`rename` 策略下 `type = "ref"` 的属性**不**自动重映射（谁引用谁是游戏语义）；codec 只在返回体里报「N 个 ref 指向被重命名的 id」，由 mod 决定怎么办。
- **I7 语料边界**：`encode` 拒绝 `function` / `thread` / `userdata` / 非法 `nil` 洞（数组中间缺项），报 `YGG-PKG-CODEC-TYPE`；不静默降级为 `null`。
- **I8 顶层纯度**：包模块顶层只有纯导出；`install` 不预读、不预写任何 KV 键。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `codec.encode` | `(value)` | string（确定性 JSON） | 纯 |
| `codec.decode` | `(text)` | value（非法输入报错） | 纯 |
| `codec.export` | `(namespace)` | bundle | 纯读 |
| `codec.checksum` | `(bundle)` | string | 纯 |
| `codec.write` | `(path, bundle)` | `{ bytes, sha256 }` | 写盘（`ygg.fs.write`） |
| `codec.read` | `(path)` | bundle（含 checksum 校验） | 纯读（`ygg.fs.read`） |
| `codec.import` | `(bundle, opts)` | `{ created, skipped, overwritten, renamed }` | 写 + 事件（经 [entity.md](entity.md) API） |
| `codec.roundtrip` | `(path)` | `{ bytes_in, bytes_out, equal }` | 写盘 + 纯读；自测与运维用 |

`checksum` 的 sha256 来源：优先取 `ygg.fs.write` 的返回值（宿主实现，同一份算法）。若 P1 挂载桥尚未就绪，退化为包内自算的 FNV-1a 64 位十六进制，并把 `codec_version` 置为 `"1-fnv1a"` 以示算法不同——这是显式降级，不是隐式兼容。

## 5. 性能预算

**本包是序列化吞吐的唯一口径定义处**：基准集 = 10k 实体（每实体 8 属性 + 3 tag）× KV 前缀 `quest` / `dlg` 各 1k 条；同机器、跑三次取中位。

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`（纯 Lua，`install` 无初始化工作）。测法同 [entity.md](entity.md) §5。 |
| 实体按 id 查找 / 更新 | `O(1)` | **不适用**：本包不做 id 索引，写实体一律经 [entity.md](entity.md) 的 O(1) 面。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | 目标导出 `≥ 5,000`、导入 `≥ 3,000` 实体/秒；给出 before/after 对比表（Goal 反例 4：无数字不算结论）。 |
| 10k 实体常驻内存 | 给出数字 | 目标导出侧峰值 `≤ 8 MiB`（bundle 文本 + 中间表，硬帽 `16 MiB`）。口径：导出前 `collectgarbage("count")` 与 bundle 长度之和。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 100 轮 `export → import(skip)` 后 `entity.index` 条目数不变，`collectgarbage("count")` 差 `≤ 1%`。 |

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| CDDA | JSON 定义 → 实体表 | `decode` 把数据文件读成实体表并 `spawn`，同一个编解码器读旧档 | P3 |
| PokeRogue | 存档 / 读档 | `export` + `write` 落挂载点；`read` + `import` 还原（幂等，读档两次不翻倍） | P3 |
| DCSS | 存档跨 run | `checksum` 作为「档没被改过」的证据，不符即拒而不是半读 | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.codec.`（例 `pkg.codec.deterministic-bytes`、`pkg.codec.import-idempotent`）。
- **断言面**：I1 用**逐字节相等**断言两次 `encode` 输出；I2 用两次导入后的记录快照相等；I4 / I5 / I7 断拒绝路径；I6 用「重命名后 ref 仍指向旧 id + 返回体计数正确」。
- **计数**：计入 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed` 的 `cases passed=`（发现机制见 [README.md](README.md) §5 未决项）。
- **禁止宽松断言**：只断「能解析」不断字节相等，视同没测 I1。

## 8. 设计依据

**为什么把编解码做成包而不是塞进 entity。** 序列化要回答两个独立问题：形状（谁是字段、谁是引用）与字节（同样的输入能否产出同样的输出）。前者归 entity，后者归 codec；混在一起时，任何为了「好看」的键序调整都会变成实体契约的变更。

**反方案一：引入宿主 JSON API。** 宿主当前没有 `ygg.json.*`，加一个就是改 `crates/**` 的内核能力——那是 P1 的载体选择问题（内核 vs 包），不该由序列化顺手推动。包内实现还带来一个好处：确定性规则（键序、数字表示）写在契约里，可被用例逐字节断言。**反方案二：二进制序列化。** 更快更小，但不可 diff、不可人工查错，而存档问题的排查现场往往只有一份文件——JSON 的字节数换可读性，是这一层的正确取舍。

**判据（①②③）。** ① 阻断——同一实体在两个游戏里用两套序列化；导入冲突策略隐式（靠 id 大小碰运气）；checksum 缺失导致半读的坏档被接受。② 症状——读档两次实体翻倍；同一份 bundle 在两台机器上字节不同导致「存档校验失败」查不出原因；坏档只报「某个字段是 nil」。③ 确认——本文件被索引；`selftest` 含 `pkg.codec.*`；§5 吞吐表有 before/after 数字。
