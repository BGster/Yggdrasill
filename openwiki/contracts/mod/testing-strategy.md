---
type: "Reference"
title: "Mod 测试策略（纯 Lua 自检）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-73f6fc2d3c89c9cda0cae6e4
    resource: repo://.ygg/mods/agent-seed/tests/fixtures.lua
  - id: openwiki-source-b28cbd2e7e58503c61a79e3b
    resource: repo://.ygg/mods/agent-seed/tests/gates.lua
  - id: openwiki-source-07e129cf68e9b586a944b0e8
    resource: repo://.ygg/mods/agent-seed/tests/harness.lua
  - id: openwiki-source-f9972c2cebb57d40e0aa06d9
    resource: repo://crates/ygg-sandbox/examples/egg/verify.rs
  - id: openwiki-source-f4f3c3407ada59508fa211cb
    resource: repo://crates/ygg-sandbox/examples/selftest.rs
  - id: openwiki-source-d3c50681436901908c5fe37c
    resource: repo://crates/ygg-sandbox/tests/agent_seed_semantics.rs
  - id: openwiki-source-a996133fa8a3add569c065f8
    resource: repo://packages/libs/entity/test/pkg-entity-paths.lua
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# Mod 测试策略（纯 Lua 自检）

> **id** mod/testing-strategy · **模块** ④ Mod 与对话层 · **触发** 改 `.ygg/mods/agent-seed/tests/**`、`packages/**/test/**` 或 `crates/ygg-sandbox/examples/selftest.rs` 之前
> **读不到代价** silent-gap（把「没覆盖」误当成「已覆盖」，或把 driver 层断言写进 Lua 后恒绿）
> **replaces**：DESIGN §6.x 测试策略节（搬运基线 `3a2536f`）
> **相关**：[mod-package.md](mod-package.md) · [tool-bridge.md](tool-bridge.md)

## 1. 结论：测试住在 Mod 里，用 Lua 写

| 维度 | 取值 |
| --- | --- |
| 正文位置 | `.ygg/mods/agent-seed/tests/*.lua`（Mod 的资产，随 Mod 一起版本化）；包自测在 `<包根>/<包名>/test/*.lua` |
| 执行入口 | `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed [--filter <substr>] [--packages <包根>]` |
| 宿主替身 | `tests/harness.lua` 提供 mock `ygg`（kv / narrate / emit / on / data / choice / meta / fsm / narrative / ticks / tool） |
| 断言风格 | `test(name, fn)` + `assert_eq` / `assert_true` / `assert_nil` / `assert_contains`；`fire(event, payload)` 直接调已注册 handler |
| 报告 | 每文件 `file ok` / `file FAIL`；末尾 `cases passed=N failed=M` 与 `ok=0|1`（失败 → 退出码 1） |

**为什么不放 `crates/**/tests`**：那会让内核 crate 携带单个 Mod 的知识。测试是这个 Mod 的一部分，改 Mod 的人应在同目录找到它。

## 2. 为什么不直接用内核沙箱

`examples/selftest.rs` 起的是**普通 Lua**，不是沙箱；`mod.lua` 与 `src/**` 经 `package.path` 加载。`KernelFacade` 不暴露「在已装载 VM 里执行测试脚本」的入口，而加这个入口要改宿主契约面。

**包模块（`pkg.*`）走另一条路**：`package.path` 只认文件路径，而包索引是「名字 → 包内相对路径」，需要一次查表，所以 `--packages <包根>` 把 `package.searchers` 的**追加项**装进 prelude。索引由宿主按与沙箱装载器**同一份**解析数据（`ResolvedPackages::chunk_paths`）生成——不可能漂移。不声明包的 mod 跑 0 条包用例。

## 3. 已知覆盖缺口（不可用 Lua 自检等价覆盖）

以下断言依赖 **driver 层**，普通 Lua 无对应物；**不再有自动化覆盖**，真实跑局时按「观测点」人工确认。

| # | 缺口 | 为什么不可自动化 | 观测点 |
| --- | --- | --- | --- |
| 1 | **事务回滚**（handler 抛错 → 该次 dispatch 全部改动回滚） | 回滚由宿主事务包裹，Lua 替身无事务边界 | 制造一次 handler 报错，检查 `autosave.json` 与 `events.jsonl` 只多一条 `ok=false` |
| 2 | **长跑停驻**（`maxSteps` / `stallLimit`） | 原燃料由宿主 hook 计量，S34 已整体退役 | 长跑一局，确认不再有宿主截停；Mod 自身逻辑照测 |
| 3 | **save receipt 结构 / `state_hash`** | `save` 信封与状态哈希是 driver 产物 | `egg open <run> --event save` 的 `kv.entries` 与 `state_hash`，跨机同序比对 |
| 4 | **事件路由 fail-closed**（未知事件零副作用） | 路由表在宿主，Lua 只按名字注册 handler | 发未声明事件，确认 `ok=false` 且 `save` 无变更 |
| 5 | **双装载面**（项目目录与 `packages/mods/agent-seed` 软链都能装载） | 自检只装载一次，且走普通 Lua 而非 `ModBundle` | `egg new --mod packages/mods/agent-seed/` 能正常起局 |
| 6 | **`playtest` 引擎内部量**（`report.steps` / `reached` / `warnings` / `errors`） | 宿主自动走局引擎自身的计量 | `cargo run -q --example playtest -- .ygg/mods/agent-seed` 的 `steps` 与 `warnings: []`；走局边界语义仍由 Lua 用例承担 |

**纪律**：新增用例若断言的是上表某一类，**不要写进 Lua**（会恒绿或无法表达），回到这里补一条观测点。

## 4. 白盒边界

`fire(event, payload)` 直接调 mod 注册的 handler，**不经过** driver 的 FIFO、事务、`ygg.choice` 哨兵——断言对象是「handler 与 effect 的行为」，不是「一次真实推进」。

- 断言 **KV 终态**（`kv("req.1.status")`）优先于断言叙述文本（叙述只用于 `[REJECTED` 这类前缀）。
- 每个用例开头 `fresh()`：清空 KV 与收集面后重放 `event.start`。
- 不依赖执行顺序：同文件共享一个 Lua 状态，`fresh()` 是唯一重置手段。

## 5. 迁移已完成：Rust 行为套件已删，只留门禁薄壳

```bash
cargo run -q --example selftest -- --mod .ygg/mods/agent-seed                     # 默认用例
cargo run -q --example selftest -- --mod .ygg/mods/agent-seed --fixture armed     # 开闸副本
# 另四个 tag：bare / plan2 / inert / nostrict
```

`--fixture <tag>` 由宿主复制 mod 到 `$TMPDIR/ygg-selftest-<tag>` 并改写 `data/config.json`；用例侧 `test(name, fn, fixture)` 的第三参决定它在哪个 tag 下执行。**一次只跑一个 tag**，所以「全绿」= 无 fixture 一次 + 每个 tag 各一次。

Rust 侧只留一个**门禁薄壳** `crates/ygg-sandbox/tests/agent_seed_semantics.rs`：不重复任何行为断言，只断言 Lua 套件仍存在且非空——因为宿主三套门禁的第三条被硬编码为 `cargo test -p ygg-sandbox --test agent_seed_semantics`（见 `crates/ygg-sandbox/examples/egg/verify.rs` 的 `verify_gates`），壳与门禁命令**同包同名**，不存在包名不符。

## 6. 设计依据

**外部仓装载：可选注入优先于全局前置**。`install_assets` 对 `docs/context/scenes.json` 采用「**有则注入、无则跳过**」，不把本仓的 doc 索引当成所有仓的装载前置；其余注入面（`__ASSETS` / `__MANIFEST` / `__PACK` / `__REPO_ROOT`）仍从 `<mod>` 与 `--mod` 锚点派生。外部仓（如装载别人 Mod 的 `PokeRogue-YGG`）没有本仓的 `scenes.json`，而该文件的唯一消费方是 agent-seed 的读签用例，外部仓不会跑它。**「没有」与「坏了」必须分开**：`is_file()` 为假 → 跳过并打印一行 `scenes: 跳过（… 不存在）`；存在却读不动或非法 JSON → 仍然 `资源装载失败`。

三问判据：① 阻断——把该文件写成装载期必读（回归 S14：外部仓一跑就退出码 1）；② 症状——外部仓 selftest 停在 `资源装载失败：read …/scenes.json`，Lua 一条用例没跑；③ 确认——`cargo run -q --example selftest -- --mod <外部仓>/mod --packages <本仓>/packages/libs` 打印 `scenes: 跳过` 且自有测试全 `file ok`，而写成非法 JSON 仍报装载失败。

**拆分先例（纯搬移零逻辑变更，判据 = 用例数 / 名守恒）**：

| 拆分 | 形态 | 判据 |
| --- | --- | --- |
| egg 宿主目录化（`examples/egg/`：生产 30 + 测试 14；serve 面再拆 11 文件） | 单文件 13619 行 = 800 行上限的 17 倍 → 子系统目录 | 用例数守恒（egg 158 / workspace 560 / pkg 199 / agent-seed 79）；`use super::*` 链不断 |
| `tests/host_contract.rs`（1089 行 22 用例） | `tests/host_contract/main.rs` + 五契约面文件，全 ≤500 行 | 22 用例逐名守恒（`--list` 剥前缀后集合 diff = ∅）；单测试目标名 `host_contract` 不变 |
| 七包自测主件（837~1129 行）× 八件 | `test/pkg-<pkg>/<theme>.lua` 兄弟件 + 主件尾部 `dofile` 装配（执行序 = 装配序） | Lua 静态 `run("` 名 multiset diff=∅（185 注册面）+ 运行时 `cases passed=199 failed=0` 前后相等 |
| `tests/kernel_facade.rs`（1071 行）→ 七文件 | `tests/<name>/main.rs` 保单目标名 | `--list` 前后集合 diff=∅，34 用例守恒 |

**selftest 红的取证规约（S27）**：门禁 / 复跑必须**全量落盘**（stdout+stderr 重定向到文件）；判红时从全量 log 提取三段——① 全部 `FAIL ` 行；② 全部 `file FAIL` 行；③ 末三行（`---` / `cases passed=` / `ok=`）。**禁止只留计数行**：两种失败形状都会打印名字（行为用例 `FAIL <name>: <detail>`，文件跑完；perf 文件 `file FAIL <file>: <err>`，计数行仍 `failed=0`）。187 次观测到的「只剩计数、无名字」是采集端把 stdout 过滤到末两行所致，不是 runner 缺陷——失败例名是定向修复的唯一入口证据。两种形状不可混淆：`cases passed=N failed=M (M>0)` = 行为用例；`failed=0` + `file FAIL` + `ok=0` = perf 阈值被打穿。

维护责任 = 改 mod / 包测试面的执行者；触发 = 任一相关 commit；检查 = `selftest` 全绿（含各 fixture tag）+ CI 的 `cargo test --workspace`。
