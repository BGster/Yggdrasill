---
type: "Reference"
title: "规则运行时 — Lua 沙箱、白名单与指令预算"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T07:10:30.723Z
sources:
  - id: openwiki-source-f5131ab478fa718e924dbf48
    resource: repo://crates/ygg-sandbox/src/inject.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-fef267a23f756637e121d083
    resource: repo://crates/ygg-sandbox/src/sandbox.rs
generated: { by: "pi", at: "2026-11-26T07:10:30.723Z" }
---

# 规则运行时 — Lua 沙箱、白名单与指令预算

> **id** `contracts/kernel/sandbox` · **模块** kernel · **触发** 改白名单、调预算、排查「规则里某个全局是 nil」或「tick 卡死」
> **读不到代价** `hard-refusal`（把 `os`/`io` 之类重新露出来，或把预算调成实质无限，都是确定性/可用性的直接破坏）

## 1. 三层环境，逐层放行

| 层 | 谁装 | 放了什么 |
| --- | --- | --- |
| base 子集 | `crates/ygg-sandbox/src/sandbox.rs`（`BASE_WHITELIST`，14 项） | `assert` `error` `getmetatable` `ipairs` `next` `pairs` `rawequal` `rawget` `rawlen` `select` `setmetatable` `tonumber` `tostring` `type` |
| 标准库表 | 同上（`LIBRARY_WHITELIST`，3 项） | `string` `table` `utf8` |
| 确定性注入 | `crates/ygg-sandbox/src/inject.rs` | `math`（**从零构造**的精确子集）+ `math.random`/`randomseed` 改写（`random` 绑世界流，`randomseed` **缺席**） |

显式排除（是决策，不是遗漏）：`dofile`/`loadfile`（文件系统）、`load`（装载期后动态生成代码会破坏「规则面是装载的纯函数」）、`print`（输出只走日志面）、`rawset`（绕过元语义写）、`collectgarbage`（GC 干预面与内存上限语义交叉）。`coroutine` 也不放：协作式并发会让「一 tick 内世界改到哪一步」不可判定。

`pcall` / `xpcall` 由 `install_catch_shims` 换成内核 shim：语义不变，但**预算哨兵不可被脚本吞掉**。

`math` 精确集：`abs` `ceil` `floor` `fmod` `modf` `tointeger` `ult` `min` `max` `type` + 常量 `huge` `maxinteger` `mininteger` + `random`。**没有** `sqrt`（距离比较用 `world.isqrt`）与超越函数（`sin`/`cos`/`exp`/`log`/`pow`）——它们在定点面上没有可移植的精确实现，露出来就是把确定性交给平台 `libm`。

## 2. 指令预算

| 项 | 值 | 为什么是这个值 |
| --- | --- | --- |
| 计量粒度 `BUDGET_QUANTUM` | 1000 条字节码一次 hook | 小到 `while true do end` 毫秒级被截断，大到 hook 开销摊薄到可忽略 |
| 单次入口默认 `DEFAULT_INSTRUCTION_BUDGET` | 2,000,000 | 比合法单 tick 宽两个数量级（10k 次循环约 1e5 条），又能在数十毫秒内截断失控循环 |
| 硬上限 `MAX_INSTRUCTION_BUDGET` | 64,000,000 | 宿主也不许把预算调到实质无限 |
| 重置时机 | 每个宿主入口开始前（`step`、`command`、装载入口 chunk） | 预算是**按入口**的，不是按 tick 累计的；100k tick 的长跑因此不会在中途撞预算 |

**"不可捕获"是结构性保证，不是约定**：预算耗尽抛的是内核拥有的 `BudgetExhausted`（Lua 侧是不透明 userdata），`pcall`/`xpcall` 的 shim 按预算状态**无条件重抛**——脚本既认不出也伪造不出这个哨兵，「捕获后换个写法重试」在 VM 里没有落点。宿主拿到的是类型化错误，`Display` 首段是稳定码 `E_BUDGET`。

## 3. 内存上限

`SANDBOX_MEMORY_LIMIT = 64 MiB`，在 `Lua::new` 之前由 `Sandbox::new()` 交给 `set_memory_limit`。超限触发分配失败（Lua 侧表现为 `not enough memory`），宿主看到的是规则报错而不是进程崩溃。

## 4. 错误边界

| 现象 | 码 | 归属 |
| --- | --- | --- |
| 调了不存在的全局 | Lua 运行期错误 → `E_STEP_RAISED` | 规则作者 |
| 超出预算 | `E_BUDGET`（不可捕获） | 规则作者，但处置（调预算/拆规则）由宿主决定 |
| 内存超限 | 规则报错 → `E_STEP_RAISED` | 规则作者 |
| 模块顶层写世界 | `ShadowBanned` 遮蔽 → 装载失败 `E_PACK` | 包结构 |

## 5. 三条常见误判

1. **"`math.sqrt` 怎么会没有"** —— 定点面上没有精确定义；`world.isqrt` 就是替代，且比较结果与平台无关。
2. **"预算是不是累计的"** —— 不是，按入口重置；但**装载**入口也一样从满额起算（见 `contracts/kernel/packaging` §4 步⑨）。
3. **"`pcall` 能兜住一切"** —— 兜不住预算：shim 会在预算耗尽时重抛。

## 6. 维护责任

- **触发**：改 `BASE_WHITELIST` / `LIBRARY_WHITELIST` / `math` 精确集、改预算常量、改 `pcall` shim、改内存上限。
- **检查**：`cargo test -p ygg-sandbox --test budget`（预算截断与不可捕获，8 例）、`cargo test -p ygg-sandbox --lib sandbox::tests`（白名单逐名断言）、`cargo test -p ygg-sandbox --lib inject::tests`（`math` 精确集与 rng 绑定）、`bash scripts/persist-matrix.sh`（长跑不撞预算）。
