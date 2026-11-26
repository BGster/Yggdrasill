---
type: "Reference"
title: "错误码 — 三层边界与映射"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-682e02a9e393e886de83488d
    resource: repo://crates/ygg-host/src/session.rs
  - id: openwiki-source-df8bdde0755ee05f46dd5db6
    resource: repo://crates/ygg-sandbox/src/host_api/world/values.rs
  - id: openwiki-source-fef267a23f756637e121d083
    resource: repo://crates/ygg-sandbox/src/sandbox.rs
  - id: openwiki-source-0fbf09adade60282738c3788
    resource: repo://crates/ygg-sim/src/error.rs
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 错误码 — 三层边界与映射

> **id** `contracts/kernel/errors` · **模块** kernel · **触发** 加错误分支、写宿主侧错误话术、排查「同一个症状给出两种码」
> **读不到代价** `hard-refusal`（错误的层被混淆 → 宿主对「存档要升级」与「存档坏了」说同一句话，处置动作全错）

## 1. 三层各管一段

| 层 | 码族 | 生产者 | 消费者拿到它之后该做什么 |
| --- | --- | --- | --- |
| 内核 | `YGG-SIM-*`（8 枚） | `crates/ygg-sim` | 判「输入不合法」还是「实现有缺陷」；前者改输入，后者报缺陷 |
| 规则面 | `E_STEP_*`（7 枚） | `crates/ygg-sandbox` | 判「调用与包对不上」还是「规则拒绝」；前者改名字/参数，后者看规则作者的拒绝原因 |
| 协议 | `E_*`（6 枚） | `crates/ygg-host`（解码层与会话） | 宿主唯一的处置依据：改帧、换包、升级存档、修规则 |

铁律：**码只在产生它的那一层声明，上层只做映射不改写**。`message` 的第一段永远是码本身，后面才是人话——这样日志里 `grep '^E_'` 就能分流。

## 2. 内核码（`YGG-SIM-*`）

| 码 | 含义 | 典型触发 |
| --- | --- | --- |
| `YGG-SIM-UNMOUNTED` | 宿主没注入实例 | 忘了 `set_sim` |
| `YGG-SIM-FIXED` | 定点面拒绝 | 非整数值过面、除零 |
| `YGG-SIM-FRAME` | 帧格式拒绝 | magic / 长度 / 尾 digest / 列类型不符（坏快照） |
| `YGG-SIM-FORMAT` | 帧版本**不被支持** | `fmt_version` 或 flags 不是这一版能读的 |
| `YGG-SIM-SCHEMA` | 组件 schema 拒绝 | 未知组件名、同帧重复组件名、类型与列不符 |
| `YGG-SIM-ENTITY` | 实体面拒绝 | 未知实体 id、`insert` 目标不存在、`next_id` 溢出 |
| `YGG-SIM-STATE` | 内部不变量破坏 | `entity_index` 与列长不符——出现即缺陷 |
| `YGG-SIM-SEALED` | `seal` 之后又注册系统 | 追加系统而不改声明 |

`FRAME` 与 `FORMAT` 分开的唯一理由就是宿主话术不同：前者「文件坏了」，后者「升级存档」。合并会把这个区别抹掉。

## 3. 规则面码（`E_STEP_*`）

| 码 | 含义 |
| --- | --- |
| `E_STEP_TICKS` | `ticks == 0` |
| `E_STEP_NO_COMMANDS` | 包没有导出命令面 |
| `E_STEP_UNKNOWN_COMMAND` | 名字不在命令面上 |
| `E_STEP_REFUSED` | 命令返回 `(false, 原因)` —— 规则作者拒绝 |
| `E_STEP_RAISED` | 规则运行期报错（Lua 抛错） |
| `E_STEP_BUDGET` | 单次入口超过指令预算 |
| `E_STEP_COMMAND_VALUE` | 命令返回值不是 `true` 或 `(false, 原因)` |

内核码在这里**原样上浮**（`StepError::Sim(err)` 保留 `YGG-SIM-*` 原文），规则面只加自己的那七枚——不把内核码翻译成 `E_STEP_*`，否则「到底是世界拒绝还是规则拒绝」就分不出来了。

## 4. 协议码（`E_*`）与映射

| 协议码 | 谁的错 | 宿主处置 |
| --- | --- | --- |
| `E_PROTOCOL` | 帧本身 | 改帧：未知字段、浮点、未知 tag、`ticks=0`、命令返回值不合契约、`snapshot` 的 `reply` 方向、时序错（`Fresh` 下非 `hello`） |
| `E_PACK` | 包 | 换包或修 manifest 摘要：镜像/清单/装载期失败 |
| `E_RULE` | 规则 | 改规则或改输入：`SEALED`/`SCHEMA`/`ENTITY`/`FIXED`/`REFUSED`/`RAISED`、未 `seal` 就 `step` |
| `E_BUDGET` | 规则（但由宿主决策） | 调预算或拆规则 |
| `E_STATE` | 会话/内核状态 | 重建会话：已关闭、未知命令、`FRAME`/`STATE`/`UNMOUNTED`、沙箱与 chunk 装载失败 |
| `E_STORE_FORMAT` | 存档 | 升级存档：`YGG-SIM-FORMAT` |

映射写在 `crates/ygg-host/src/session.rs` 的三个私有函数里（`step_code` / `load_code` / `sim_code`），是**唯一**的翻译点。加一个新内核码必须同时改 `sim_code`，否则它会被 `match` 漏掉——这是编译器会抓的那种遗漏，别用通配符兜底。

## 5. 三条不得违反的写法

1. **不许吞**：任何 `Err` 都必须变成一帧 `error` 或一条 Lua 错误，不许 `let _ =`。
2. **不许猜**：映射不到的情况（本次 `match` 穷尽，不会发生）宁可答 `E_STATE` + 原文，不要造新码。
3. **不许重复声明**：同一个语义只在一层有码。需要新语义时，先选层，再在该层的枚举里加。

## 6. 维护责任

- **触发**：加错误分支、改映射、改宿主错误话术、加新层。
- **检查**：`cargo test -p ygg-host`（`session` 的负例逐条钉住码与原文）、`cargo test -p ygg-sim`（内核码的 `Display` 形状）、`cargo test -p ygg-sandbox`。
