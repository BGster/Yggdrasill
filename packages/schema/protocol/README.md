# `ygg/1` 线协议

宿主与内核之间只有这一层信封。它**搬运世界状态，但不理解世界**：所有 `if` 都在内核与
沙箱里，协议层只做「字段在这、宽度合不合、判别键认不认」三件事（计划稿 §0 决策 4）。

同一份契约有两个载体，缺一不可：

| 载体 | 位置 | 谁用 |
| --- | --- | --- |
| JSON Schema（draft 2020-12） | `packages/schema/protocol/ygg-1.schema.json` | 非 Rust 宿主（C ABI、WASM、H5 侧手写） |
| Rust 类型 | `crates/ygg-host/src/proto.rs` | 内核侧实现与所有 Rust 宿主 |

两者不靠人工比对保持同步：`crates/ygg-host/tests/schema_conformance.rs` 把每个变体编成
线上字节喂给 schema，schema 说「不认」就是分叉。

## 判别面

一条消息只有**一根**判别轴，避免两套判别互相打架：

| 层 | 判别键 | 取值 |
| --- | --- | --- |
| 消息 | `type` | `hello` `ready` `open` `submit` `command` `step` `events` `delta` `snapshot` `replay` `replay_result` `error` |
| `snapshot` | `direction` | `request`（无载荷） / `reply`（带 `tick`/`hash`/`bytes`） |
| 操作 | `op` | `spawn` `despawn` `set` `insert` `remove` |
| 世界值 | 唯一的键 | `int` `uint` `str` `bool`（外部标签：键既是判别式又是载荷） |

`snapshot` 与 `replay` 各有两个方向，但两者的线上形态不同：`snapshot` 是**一条消息名**的
两个方向（靠 `direction` 分），`replay` 的回执必须换标签成 `replay_result`（同一个枚举里
两个变体不能共用一个 `type`）。

## 消息表

方向：H→K 宿主发给内核，K→H 内核回宿主。

### `hello` H→K

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `protocol` | `const "ygg/1"` | 版本面；别的值在解码处即拒 |
| `codec` | `const "json"` | v1 只有 JSON；CBOR 走同一套协商（计划稿 §0 决策 2） |
| `pack_id` | string | 包 id（`ygg:…` 或包名） |
| `pack_hash` | 64 位小写十六进制 | 包内容摘要，与 `manifest.json` 的内容寻址同口径 |
| `seed` | u64（可选） | 创世随机流种子；缺省沿用内核默认种子，显式提供则在进入 `ready` 前按该种子重建干净世界谱系 |

`pack_hash` **格式**不合 → `E_PROTOCOL`（写错了字节）；格式合但与本地包不符 → `E_PACK`
（拿错了包）。这道闸防的是「A 包的存档读进 B 包」。

`seed` **不在** `required` 里：字段不在 = 沿用默认种子（老宿主一行不改）。显式 `null`
不是缺省而是错（与 `replay_result.first_bad_tick` 同口径）；浮点 / 负数 / 超 `u64` 一律
解码期拒为 `E_PROTOCOL`。

### `ready` K→H

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `tick` | u64 | 当前 tick |
| `hash` | u64 | 当前状态 hash |
| `sealed` | bool | 世界是否已封版（封版后才能推进） |
| `components` | `[{id:u32, name:string, kind:kind}]` | 组件表，`id` 是后文 `cid` 的唯一来源 |
| `systems` | `[{name:string, stage:stage}]` | 系统表；序即执行序 `pre → rules → post` |

`kind` 取值与 `ygg_sim::AttrType::label` 逐字相同：`number` `string` `boolean` `ref`
`tag` `f64bits`。宿主据它决定往 `values` 里塞哪种编码；`open` 成功后回同一条形状。

### `open` H→K

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `bytes` | base64 string | 存档字节，协议层不解码，只搬运 |

### `submit` H→K

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `ops` | `[op]` | 见下；**整批原子**（不变式 2） |

| `op` | 其余字段 | 语义 |
| --- | --- | --- |
| `spawn` | `n:u32 ≥ 1`、`comps:[{cid,values}]` | 生成 `n` 个实体，共用同一套组件值 |
| `despawn` | `ids:[u64]` | 按 id 删除 |
| `set` | `id:u64`、`comps` | 改已有实体的组件值 |
| `insert` | `id:u64`、`comps` | 给已有实体加组件 |
| `remove` | `id:u64`、`cids:[u32]` | 从实体上摘组件 |

`comps[].values` 的行数必须等于本次操作的行数：`spawn` 是 `n`（每个实体一行），
`set` / `insert` 是 `1`。少一行不是「广播到每一行」，而是 `E_PROTOCOL`。

回执：答**一条 `delta`**（本次写入的组件终值）。`spawn` 出来的新 id 不在任何专属回执里——
宿主从这条 `delta` 的 `eid` 认领。整批失败时零写入，答 `error`，世界 hash 不变（不变式 2）。
v1 没有独立的 `submit_result` 标签：`delta` 本来就是「本次被写过的组件」这一个面。

### `command` H→K

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `name` | string | 命令名：包入口 chunk 返回值里 `commands` 表的键 |
| `args` | object | 命令参数：任意 JSON 对象，原样转成 Lua 表递给命令函数 |

**这一帧是「意图」过界的唯一通道。** 它与 `submit` 的分界只有一条：谁决定规则。

| 帧 | 宿主给出什么 | 规则在谁手里 |
| --- | --- | --- |
| `submit` | 已经算好的结构变更（`ops`） | 宿主 |
| `command` | 只想做的一件事（`name` + `args`） | 包（命令函数由包作者写） |

**内核不解释 `name`**：它只是命令面这张表的键。协议层与内核都不认识任何具体命令名，也不读
`args` 里的任何键。命令集合、参数形状、允不允许，全在包里；`packs/example/src/command.lua`
的 `move` / `attack` / `end_turn` 是**示例**，不是协议的一部分。

回执：成功答**一条 `delta`**（与 `submit` 同一份 `Changed` 面），失败答 `error`：

| 情形 | `code` |
| --- | --- |
| 命令返回 `false, <原因>`（Mod 拒绝） | `E_RULE` |
| 命令抛出 Lua 错误（含参数转 Lua 失败） | `E_RULE` |
| 名字不在命令面上 | `E_STATE` |
| 包没有导出命令面（入口返回值的 `commands` 不是表） | `E_STATE` |
| 返回值既不是 `true` 也不是 `(false, 原因)` | `E_PROTOCOL` |

每次调用**一个事务**：失败时 `tick`、`hash` 与世界逐字节不变——拒绝不会写一半。

`args` 是**不透明参数袋**：信封只要求它是一个 JSON 对象，键与值都是包的词汇。转换用的是
`ygg.data` 同一份 JSON→Lua 映射（对象按键序插入、数组 1..n、整数优先 i64），因此同一段
`args` 在任何宿主上都得到同一张 Lua 表。

### `step` H→K

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `ticks` | u32 ≥ 1 | 推进几步；`0` 是无效指令，两侧都拒 |
| `dt` | u32 | 每 tick 步长（定点的整数原值，不是浮点秒）；`0` 合法（暂停推进） |

### `events` K→H

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `tick` | u64 | 推进后的 tick |
| `hash` | u64 | 推进后的状态 hash |
| `log` | `[{level, text}]` | `level` ∈ `debug` `info` `warn` `error`；只映射世界的 `world.log` |
| `errors` | `[{code, msg}]` | 本步里的规则错误（`E_RULE` / `E_BUDGET`），不中断推进 |

### `delta` K→H

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `tick` | u64 | 本批结果的 tick |
| `changed` | `[{eid:u64, comps:[{cid, values}]}]` | **只列本步被写过的组件**（不变式 3） |

### `snapshot`（双向同名）

| 方向 | 字段 |
| --- | --- |
| `direction:"request"`（H→K） | 只有 `type` 与 `direction`，多一个字段即拒 |
| `direction:"reply"`（K→H） | `tick`、`hash`、`bytes`（base64） |

### `replay` H→K / `replay_result` K→H

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `log.header.seed` | u64 | 世界种子 |
| `log.header.start_tick` | u64 | `0` = 从创世回放；否则 = 快照所在 tick |
| `log.frames[].tick` | u64 | 该帧到期后的 tick |
| `log.frames[].dt` | u32 | 该帧步长。**不能省**：回放要复现这一 tick 就必须复现它 |
| `log.frames[].ops` | `[op]` | 该帧的操作 |
| `log.frames[].hash` | u64 | 该帧期望的 hash |
| `ok` | bool | 整段回放是否逐帧对上 |
| `first_bad_tick` | u64，可缺省 | 首个不符的 tick；`ok` 为真时**整个字段不写**（不写 `null`） |

逐帧比对、首个不符即停：`first_bad_tick` 是定位用的第一个分歧点，不是全部分歧点。

### `error`（双向同形）

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `code` | `E_PROTOCOL` `E_PACK` `E_BUDGET` `E_RULE` `E_STATE` | 决定宿主的反应 |
| `msg` | string | 人类可读；不参与任何判定 |

五类错误码对应五种宿主动作，合并会把「重连」「换包」「报给包作者」「回滚重试」搅成一句
「出错了」：`E_PROTOCOL` 是宿主自己的 bug，`E_PACK` 换包或换档，`E_BUDGET`/`E_RULE` 报给
包作者，`E_STATE` 回滚并重试。

## 绑定层的回执形状（一请求一数组）

C ABI 与 WASM 的边界只过字符缓冲区：一次 `handle(msg_json)` 进去、一段 JSON 出来，没有
「一帧一行」的流式通道。所以绑定面把**帧流**包成一个 JSON 数组（顺序即线上顺序）：

```text
handle('{"type":"step","ticks":1,"dt":1}')
→ [{"type":"events",…},{"type":"delta",…}]      // step 两帧
handle('{"type":"submit","ops":[…]}')
→ [{"type":"delta",…}]                            // 其余一帧
handle('not json')
→ [{"type":"error","code":"E_PROTOCOL",…}]       // 错也是帧，不是异常/NULL
```

- **数组不改变逐消息语义**：它只是把同一段帧序装进一段文本；每条消息答什么仍由上一节
  的消息表决定。
- **打包规则住在内核侧**（`crates/ygg-host/src/proto.rs` 的 `encode_frames`），绑定层只搬
  字符串——它若自己拼数组，就等于多一份与 Rust 宿主的帧序实现。
- **三条载体逐帧等价**：Rust `Session`、WASM `YggSession`、C ABI 跑同一份脚本
  `packages/schema/protocol/parity-script.json`（hello → ready、submit → delta、
  command → delta、step → events + delta、snapshot → reply），回执规范化后逐字节相同
  （`crates/ygg-capi/tests/parity.rs`、`crates/ygg-wasm/tests/parity.rs`）。

## 三条不变式

### 1. 世界值不含浮点

`values` 只有 `int` / `uint` / `str` / `bool` 四种编码。定点数按其整数原值走 `int`
（`Fixed` = Q32.32 的 `i64`），`f64bits` 组件按 IEEE-754 的位型走 `uint`——位型是整数，
浮点是解释。

理由是同步性：浮点求和在不同 ISA、不同优化级别、不同 libm 下结果不同，两个宿主各算一遍
就会漂移，而漂移在回放里表现为「同一份日志两个 hash」，无从定位。

**这一格只有边界能守。** JSON Schema 的数据模型只有一种数，「`1.0`」与「`1`」在它眼里
同为整数，所以 schema 收 `1.0` 而 crate 拒——`schema_cannot_tell_one_from_one_point_zero`
把这条不对称显式钉住，免得日后被当成 bug 去「修」。防线在 `crates/ygg-host/src/proto.rs`
的 `strict` 模块：它按 `serde_json::Number` 的**形态**判（`as_u64`/`as_i64` 命中才算整数），
f64 一律拒，且**不四舍五入**——四舍五入会把「对端算错了」变成「对端算得差不多」。

### 2. `submit` 整批原子

一批 `ops` 要么全落，要么全不落：中途任何一个 op 被内核拒（`E_STATE`）或规则报错
（`E_RULE`），整批回滚，世界回到提交前的 tick 与 hash。宿主因此不需要知道「前三条成了、
第四条没成」这种中间态。

### 3. `delta` 只报被写过的组件

`changed[].comps` 不是实体的全部组件，而是本步**被写过**的那些。全量 delta 会让
「本步没碰的东西」与「本步碰成原样的东西」无法区分，也让带宽随实体宽度而不是随变更量增长。
宿主侧因此必须用 `insert`/`set` 的语义**合并**，不能整实体覆盖。

## 与计划稿消息表的偏差

以下字段/形状是表里没写、实现时必须定的（都在 `crates/ygg-host/src/proto.rs` 的 doc 里
就地说明）：

| 偏差 | 为什么 |
| --- | --- |
| `spawn.n` | 内核 `spawn_columns(n, …)` 本来就是批量；50 个棋子用 50 条 op 是 50 倍信封开销 |
| `frames[].dt` | `ygg_sim::input::TickRecord` 是 `{tick, dt, hash, ops}`；不带 `dt` 回放不出这一 tick |
| `log.header.{seed, start_tick}` | 与 `ReplayLog` 的字段一一对应；`start_tick` 区分「从创世」与「从快照」 |
| `LogLevel` 枚举 | 表里只写 `{level, text}`；级别不枚举的话宿主无法按级别过滤 |
| `changed[].comps` 而非 `{cids, values}` | 一个组件有多列，平行数组无法表达「第 i 个值属于哪个 cid」；JSON Schema 也难便宜地约束两数组等长 |
| `replay_result` 独立标签 | serde 的 `type` 判别要求同枚举内标签唯一，两个方向不能共用 `replay` |
| `ProtocolError` 既是消息也是 `errors` 条目 | 两个位置的形状本来就相同，分成两个类型等于允许它们慢慢分叉 |

## 改这份契约的流程

1. 先改 `ygg-1.schema.json` 与 `proto.rs`（同一笔改动）；
2. `cargo test -p ygg-host` —— 样本往返、schema 校验、成对拒收三条会同时表态；
3. schema 的 `$defs` 名、判别面、`PROTOCOL`/`CODEC` 常量都已被测试钉住，改它们就得动测试，
   动测试的人会看见这张表。

`packages/schema/protocol/` 只放**线协议**。包清单（`pack.json`）、存档格式这类「盘上字节」
的 schema 仍在 `packages/schema/schemas/`，两者不共用 `$id` 空间。
