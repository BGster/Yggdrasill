---
type: "Reference"
title: "规则侧 API — world.* 与命令契约"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-6cc1016021f1acaff9cc0b8b
    resource: repo://crates/ygg-core/src/kv.rs
  - id: openwiki-source-ce6aebf2c3323e209b2938c2
    resource: repo://crates/ygg-sandbox/src/host_api/world/install.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-86c13e4901257c152b5f1bfe
    resource: repo://crates/ygg-sim/src/fixed.rs
  - id: openwiki-source-b888e3dc95ca0a07465ab295
    resource: repo://crates/ygg-sim/src/snapshot.rs
  - id: openwiki-source-533eaac139cebd8db482c862
    resource: repo://packs/example/src/command.lua
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 规则侧 API — world.* 与命令契约

> **id** `contracts/kernel/lua-api` · **模块** kernel · **触发** 写/改 Lua 规则、加 `world.*` 成员、排查「规则里这个方法不存在」
> **读不到代价** `hard-refusal`（按错的签名写规则 → 装载期或 tick 期抛错，且错误点常在很远的地方）

## 1. 面在哪、谁装

`crates/ygg-sandbox/src/host_api/world/install.rs` 装全局表 `world`（17 个键）：`component` / `spawn` / `despawn` / `insert` / `set` / `get` / `remove` / `each` / `each_mut` / `ids` / `tick` / `rng` / `isqrt` / `log` / `system` / `seal` + `kv`（与 `ygg.kv` 同一张表，装 `world` 之前必须先装 `ygg.kv`）。

没有 `ygg.entity.*`、没有 `ygg.sim.*`、没有 `ygg.on/ygg.emit` —— 旧宿主面（事件、叙事、选择、工具、FS）随 Agent 骨架一起删除。

## 2. 每个成员的签名与语义

| 签名 | 语义 |
| --- | --- |
| `world.component{ name, kind } -> cid` | 声明一个组件；`kind ∈ {"i64","u64","u8","bytes"}`；同名重复声明即报 `YGG-SIM-SCHEMA` |
| `world.spawn{ [name] = value, … } -> id` | 造一个实体并写列；值必须与声明的 `kind` 相符 |
| `world.despawn(id) -> n` | 销毁实体，`n` = 改动单元格数 |
| `world.insert(id, { … }) -> n` | 给已存在实体补列（目标不存在报 `YGG-SIM-ENTITY`） |
| `world.set(id, { … }) -> n` | 覆写已有列的值 |
| `world.get(id, name) -> value \| nil` | 读一格；实体或列不存在返回 `nil` |
| `world.remove(id, { "hp", … }) -> n` | 摘列；实体因此失去全部列时由内核决定是否消失 |
| `world.each({ "pos", … }, fn(id, pos, …)) -> n` | 只读迭代匹配全部列的实体；返回迭代条数 |
| `world.each_mut({ "pos", … }, fn(id, pos, …) -> new_pos)` | 写迭代：**回调返回值**回写该列（改几列就返回几个值） |
| `world.ids({ "pos", … }) -> { id, … }` | 只要 id（数组，1 基，规范序） |
| `world.tick() -> u64` | 当前 tick |
| `world.rng(stream) -> u32` | 从命名流取一个 32 位；流按名派生，见 `contracts/kernel/determinism` §6 |
| `world.isqrt(x) -> Fixed` | Q32.32 定点平方根；**定量距离比较的唯一入口**（`math.sqrt` 不存在） |
| `world.log(fmt, …)` | 追加一行日志；随本 tick 的 `events` 帧回到宿主 |
| `world.system{ name, stage, run, reads, writes, after }` | 注册系统；`stage ∈ {"pre","rules","post"}` |
| `world.seal()` | 封闭调度器；之后再 `world.system` 报 `YGG-SIM-SEALED`，未知组件引用在此刻被解析 |

`world.each_mut` 的回调是**纯函数式写回**：内核在回调返回后写列，回调体内再调 `world.set` 会与写回顺序打架——需要多列写时一次返回多值，不要中途 `set`。

## 3. 命令面（宿主 ↔ 规则的对公接口）

入口 chunk 的返回值里若带 `commands` 表，这个包就有命令面：

```lua
return { commands = {
  move   = function(args) … end,   -- args 是 JSON 对象解出来的表
  attack = function(args) … end,
} }
```

契约：

- **成功只能返回 `true`**；拒绝返回 `false, "<原因>"`（原因会出现在 `E_STEP_REFUSED` 的 message 里）；返回其他任何值 = `E_STEP_COMMAND_VALUE`；
- 拒绝必须**不改状态**：`Loaded::command` 在调用前后各做一次复原（undo 日志 + 快照），规则作者不需要自己回滚；
- 命令在 tick 之间执行，**不推进时间**；宿主想看到效果再发 `submit` 或 `step`；
- `args` 是 `serde_json::Value` → Lua 表的直接转换；空对象与 `null` 都当 `{}`；
- 没有命令面的包，宿主面入口直接答 `E_STEP_NO_COMMANDS`，不静默降级成空面。

## 4. KV 的边界

`world.kv`（= `ygg.kv`）存**不属于列存世界**的标量：宿主注入的环境量、规则自己的簿记。

- 不进快照、不进 `state_hash`、不被 `replay` 复算；
- 因此**任何影响玩法的状态都不能放 KV** —— 放了就是「同一份日志重放出不同结果」；
- `world.kv` 的键是扁平字符串（`"ai.speed"`），值只收四型标量。

## 5. 三类常见违规与症状

| 症状 | 原因 |
| --- | --- |
| `attempt to index a nil value (global 'math')` | 用了 `math.sin/sqrt/pow` —— 面里没有；距离用 `world.isqrt` |
| `world.component` 报「已声明过」 | 同一个组件在两次装载/两个模块里各声明一次；声明只许一处 |
| 命令返回后世界没动 | 返回了 `nil` 或 `false` 而不带原因；只有 `true` 算成功 |

## 6. 维护责任

- **触发**：加/改 `world.*` 成员、改命令返回值契约、改 KV 语义。
- **检查**：`cargo test -p ygg-sandbox`（`world_facade` / `command` 面用例）、`cargo run -q --example selftest -p ygg-sandbox -- --mod packs/example`、`cargo run -q --example golden -p ygg-sandbox -- packs/example --check packs/example/golden.json`。
