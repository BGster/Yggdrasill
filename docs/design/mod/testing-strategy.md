# Yggdrasill · Mod 测试策略（纯 Lua 自检）

> **id** mod/testing-strategy · **模块** ④ Mod 与对话层 · **触发** 改 `.ygg/mods/agent-seed/tests/**` 或 `crates/ygg-sandbox/examples/selftest.rs` 之前
> **读不到代价** silent-gap（把「没覆盖」误当成「已覆盖」，或把 driver 层断言写进 Lua 后恒绿）

> **定位**：agent-seed 的测试为什么写在 Lua、跑在哪里、边界在哪，以及**已知不覆盖**的清单。
> **上游**：[../../README.md](../../README.md) · **相关**：[tool-bridge.md](tool-bridge.md) · [mod-package.md](mod-package.md)

## 1. 结论：测试住在 Mod 里，用 Lua 写

| 维度 | 取值 |
| --- | --- |
| 正文位置 | `.ygg/mods/agent-seed/tests/*.lua`（Mod 的资产，随 Mod 一起版本化） |
| 执行入口 | `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed [--filter <substr>]` |
| 宿主替身 | `tests/harness.lua` 提供 mock `ygg`（kv / narrate / emit / on / data / choice / meta / fsm / narrative / ticks / tool） |
| 断言风格 | `test(name, fn)` + `assert_eq` / `assert_true` / `assert_nil` / `assert_contains`；`fire(event, payload)` 直接调已注册 handler |
| 报告 | 每个文件 `file ok` / `file FAIL`；末尾 `cases passed=N failed=M` 与 `ok=0|1`（失败 → 退出码 1） |

**为什么不放在 `crates/**/tests`**：那会让内核 crate 携带单个 Mod 的知识。测试是这个 Mod 的一部分，
改 Mod 的人应当在同一目录里找到它。

## 2. 为什么不直接用内核沙箱

`examples/selftest.rs` 起的是**普通 Lua**，不是沙箱；`mod.lua` 与 `src/**` 通过 `package.path` 加载。
原因：`KernelFacade` 不暴露「在已装载的 VM 里执行一段测试脚本」的入口，而给宿主加这个入口会改
`crates/**/src/**`（宿主契约面）。当前取舍是：**测试环境 = 普通 Lua + mock ygg**，代价见 §3。

## 3. 已知覆盖缺口（不可用 Lua 自检等价覆盖）

以下断言依赖 **driver 层**（宿主），普通 Lua 环境里没有对应物。它们**不再有自动化覆盖**，
真实跑局时按「观测点」列人工确认。

| # | 缺口 | 为什么不可自动化 | 真实跑局时的观测点 |
| --- | --- | --- | --- |
| 1 | **事务回滚**（handler 抛错 → 该次 dispatch 的全部改动回滚） | 回滚由宿主事务包裹实现，Lua 替身没有事务边界 | 制造一次 handler 报错，检查 `autosave.json` 与 `events.jsonl` 是否只多出一条 `ok=false` |
| 2 | **fuel 预算**（`maxSteps` / `stallLimit` / 每步燃料） | 燃料由宿主 hook 计量，Lua 侧无 hook | 长跑一局，看是否在预期步数内触发 `[FUEL]` / 停驻 |
| 3 | **save receipt 结构 / `state_hash`** | `save` 信封与状态哈希是 driver 产物 | `egg open <run> --event save` 的 `kv.entries` 与 `state_hash`；跨机同序比对 |
| 4 | **事件路由 fail-closed**（未知事件不产生副作用） | 路由表在宿主，Lua 只按名字注册 handler | 发一个未声明事件，确认 `ok=false` 且 `save` 无变更 |
| 5 | **双装载面**（项目目录与 `packages/mods/agent-seed` 软链都能装载） | 自检只装载一次，且走普通 Lua 而非 `ModBundle` | `egg new --mod packages/mods/agent-seed/` 能正常起局 |

**纪律**：新增用例时若发现断言的是上表某一类，**不要写进 Lua**（它会恒绿或无法表达），
而是回到这里补一条观测点。

## 4. 白盒边界

`fire(event, payload)` 直接调用 mod 注册的 handler，**不经过** driver 的 FIFO、事务、fuel、
`ygg.choice` 哨兵。因此断言的对象是「handler 与 effect 的行为」，不是「一次真实推进」。

写用例时遵守：

- 断言 **KV 终态**（`kv("req.1.status")`）优先于断言叙述文本（叙述用于 `[REJECTED` 这类前缀）。
- 每个用例开头 `fresh()`：清空 KV 与收集面后重放 `event.start`（等价于 Rust 侧新建一个 `Game`）。
- 不要依赖执行顺序：同一文件内的用例共享一个 Lua 状态，`fresh()` 是唯一重置手段。

## 5. 迁移期约定（临时）

Rust 对照套件仍在 `crates/agent-seed-tests`（纯 manifest 登记 → `.ygg/mods/agent-seed/tests/agent_seed_semantics.rs`）。
每迁一个域：Lua 例数 = 从 Rust 删除的例数，两侧同时跑绿后再提交。
全部迁完后，删除该 `.rs` 与 `crates/agent-seed-tests/`，本节的「迁移期约定」一并移除。
