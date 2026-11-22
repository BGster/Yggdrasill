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

**包模块（`pkg.*`）走另一条路**：`package.path` 只认文件路径，而包索引是 `名字 → 包内相对路径`，
需要一次查表，所以 `--packages <包根>` 把 `package.searchers` 的**追加项**装进 prelude。
索引由宿主按与沙箱装载器**同一份**包解析数据生成（`ResolvedPackages::chunk_paths`），
不是第二套规则——两则不可能漂移。包自测在 `<包根>/<包名>/test/*.lua`，与 mod 自测进入
同一个 `__SELFTEST` 计数面（不声明包的 mod 跑 0 条）。

## 3. 已知覆盖缺口（不可用 Lua 自检等价覆盖）

以下断言依赖 **driver 层**（宿主），普通 Lua 环境里没有对应物。它们**不再有自动化覆盖**，
真实跑局时按「观测点」列人工确认。

| # | 缺口 | 为什么不可自动化 | 真实跑局时的观测点 |
| --- | --- | --- | --- |
| 1 | **事务回滚**（handler 抛错 → 该次 dispatch 的全部改动回滚） | 回滚由宿主事务包裹实现，Lua 替身没有事务边界 | 制造一次 handler 报错，检查 `autosave.json` 与 `events.jsonl` 是否只多出一条 `ok=false` |
| 2 | **长跑停驻**（`maxSteps` / `stallLimit`） | 原燃料由宿主 hook 计量；S34 燃料已整体退役，宿主不再计量 | 长跑一局，确认不再有宿主截停；Mod 自身的 maxSteps/stallLimit 逻辑照测 |
| 3 | **save receipt 结构 / `state_hash`** | `save` 信封与状态哈希是 driver 产物 | `egg open <run> --event save` 的 `kv.entries` 与 `state_hash`；跨机同序比对 |
| 4 | **事件路由 fail-closed**（未知事件不产生副作用） | 路由表在宿主，Lua 只按名字注册 handler | 发一个未声明事件，确认 `ok=false` 且 `save` 无变更 |
| 5 | **双装载面**（项目目录与 `packages/mods/agent-seed` 软链都能装载） | 自检只装载一次，且走普通 Lua 而非 `ModBundle` | `egg new --mod packages/mods/agent-seed/` 能正常起局 |
| 6 | **`playtest` 引擎的内部量**（`report.steps` / `reached` / `warnings` / `errors`） | 这些是宿主自动走局引擎自身的计量，Lua 侧只能手动发 `choice_selected`，没有「第几步、停在哪个微状态、引擎报了什么警告」这些量 | `cargo run -q --example playtest -- .ygg/mods/agent-seed` 的 `steps` 与 `warnings: []`；走局边界语义仍由 Lua 用例承担（如「未完成批次 ⇒ 退休不可达」查行首 `[ENDING`） |

**纪律**：新增用例时若发现断言的是上表某一类，**不要写进 Lua**（它会恒绿或无法表达），
而是回到这里补一条观测点。

## 4. 白盒边界

`fire(event, payload)` 直接调用 mod 注册的 handler，**不经过** driver 的 FIFO、事务、
`ygg.choice` 哨兵。因此断言的对象是「handler 与 effect 的行为」，不是「一次真实推进」。

写用例时遵守：

- 断言 **KV 终态**（`kv("req.1.status")`）优先于断言叙述文本（叙述用于 `[REJECTED` 这类前缀）。
- 每个用例开头 `fresh()`：清空 KV 与收集面后重放 `event.start`（等价于 Rust 侧新建一个 `Game`）。
- 不要依赖执行顺序：同一文件内的用例共享一个 Lua 状态，`fresh()` 是唯一重置手段。

## 5. 迁移已完成（对照 Rust 套件已删除）

Mod 的行为断言**全部**在 `.ygg/mods/agent-seed/tests/*.lua`（`harness.lua` + 10 个域文件），
由宿主的 `crates/ygg-sandbox/examples/selftest.rs` 执行：

```bash
cargo run -q --example selftest -- --mod .ygg/mods/agent-seed            # 默认用例
cargo run -q --example selftest -- --mod .ygg/mods/agent-seed --fixture armed   # 开闸副本
# 另四个 tag：bare / plan2 / inert / nostrict
```

`--fixture <tag>` 由宿主复制 mod 到 `$TMPDIR/ygg-selftest-<tag>` 并改写 `data/config.json`；
用例侧 `test(name, fn, fixture)` 的第三参决定它只在哪个 tag 下执行。**一次运行只跑一个 tag**，
所以「全绿」= 无 fixture 一次 + 每个 tag 各一次。

Rust 侧只留一个**门禁薄壳**（`crates/ygg-sandbox/tests/agent_seed_semantics.rs`，属包 `ygg-sandbox`）：它不重复
任何行为断言，只断言 Lua 套件仍存在且非空——因为宿主三套门禁的第三条被硬编码成
`cargo test -p ygg-sandbox --test agent_seed_semantics`（见 `examples/egg/` 的 `verify_gates`），
而 egg 宿主（`examples/egg/` 目录：`main.rs` 装配序 + egg/43 + serve/11 个按子系统切分的兄弟模块）是 non-negotiable
host——目录化是纯搬移拆分（r-20261121-045703-10d8，44 文件、全部 ≤800 行），门禁与用例数零变化。
壳与门禁命令**同包同名**，不存在包名不符。

## 6. 设计依据（外部仓装载：可选注入优先于全局前置）

**规范**：`install_assets` 对 `docs/context/scenes.json` 采用「**有则注入、无则跳过**」，不把本仓的
doc 索引当成所有仓的装载前置。注入的其余面（`__ASSETS` / `__MANIFEST` / `__PACK` / `__REPO_ROOT`）
仍从 `<mod>` 与 `--mod` 锚点派生，语义不变。

**依据**：`repo_root` 由 `ancestor_with_git(--mod)` 求出，外部仓（装载别人的 Mod，如 `PokeRogue-YGG`）
自然没有本仓的 `docs/context/scenes.json`；该文件的唯一消费方是 agent-seed 的读签用例
（`tests/surface-tree.lua` 的 `scenes 表 reference 段` 断言；S32 拆分后自 `tests/surface.lua` 迁入），外部仓不会跑 agent-seed 的测试。
「没有」与「坏了」必须分开：`is_file()` 为假 → 跳过并打印一行 `scenes: 跳过（… 不存在）`；
文件存在却读不动或不是合法 JSON → 仍然 `资源装载失败`，否则真损坏会被静默跳过。

**三问判据**：① 阻断——把该文件写成装载期必读（回归 S14：外部仓一跑就 `No such file or directory`，
退出码 1）；② 症状——外部仓 selftest 停在 `资源装载失败：read …/docs/context/scenes.json`，
Lua 一条用例都没跑；③ 确认——`cargo run -q --example selftest -- --mod <外部仓>/mod --packages <本仓>/packages/libs`
打印 `scenes: 跳过（…）` 且 mod 自有测试文件全 `file ok`；把该文件写成非法 JSON 则仍报
`资源装载失败：scenes.json: …`。

**egg 宿主目录化（2026-11-21，r-20261121-045703-10d8）**：宿主单文件 13619 行 = 800 行硬上限的
17.0 倍，按原则六用户授权「多做模块化拆分」拆为 `examples/egg/` 目录（生产 30 + 测试 14；
S-25 批二 serve 面再拆 11 文件，egg/43 + serve/11 全部 ≤800 行，见 `host/serve.md` 设计依据）。
**依据**：纯搬移零逻辑变更——测试 mod 跟随所属子系统入同目录（`identity/tests.rs` 等），
`use super::*` 语义链不断（子模块经父 glob 取根名集），`use super::{…}` 显式清单原样可解析；
拆分前后用例数守恒（egg 158 / workspace 560 / pkg selftest 199 / agent-seed 79）即门禁判据。

**宿主集成测试面目录化（2026-11-21，r-20261121-142302-3a26）**：`tests/host_contract.rs` 单文件
1089 行 = 22 个用例跨面杂居，按契约面拆为 `tests/host_contract/` 目录（main.rs 装配 + 五契约面：
narrative 输出面 / adhoc 即席写 / gates 拒绝路径 / entity_face 14 键闭集+batch+lifecycle /
entity_query selector 语法面，全部 ≤500 行）；`tests/<name>/main.rs` 官方规则保单测试目标，
目标名 `host_contract` 不变，23 个 tests 目标计数守恒（22 单文件 + 1 目录化）。
**依据**：纯搬移零逻辑变更——22 用例逐名守恒（`--list` 前后剥模块前缀集合 diff = ∅）即门禁判据；
根 use 头逐字节不动，子文件 `use super::*;` 一行即足（玩具 crate 复刻实验：rustc 不报根
unused_imports，glob 转发语义链与 egg 目录化同理，先例 `src/driver/tests.rs`）。

**测试件连拆八件归位（2026-11-21，r-20261121-170022-4615）**：七包自测主件 837~1129 行越 800
硬上限（原则六授权 500/800），用例段按主题连续切片搬入 `test/pkg-<pkg>/<theme>.lua` 兄弟件——
形态 `return function(C) local … = C.… <连续行段原字节> end`，主件尾部 `local C = {…}` +
`dofile(PKG_ROOT .. …)(C)` 装配，执行次序 = 装配次序；主件路径不变，runner 对 `test/` 的
非递归采集面不扰（兄弟件不单独入册）。同法目录化 `tests/kernel_facade.rs`（1071 行 →
`tests/kernel_facade/` 七文件，`tests/<name>/main.rs` 保单目标名，34 用例逐名守恒）。
**依据**：纯搬移零逻辑变更——Lua 静态 `run("` 名 multiset 逐包 diff=∅（185 注册面）+ 运行时
`cases passed=199 failed=0` 前后相等即判据；跨片 helper 上移主件经 C 注入（dialogue/quest 的
pkg_sources+count_in_sources、entity 的 basic_spec、assetgen 的 BUNDLE_ENTRIES/OSC/samples_of），
骑片 helper 随主题走；Rust 侧 `--list` 前后剥模块段前缀集合 diff=∅、件头「32 测」陈旧计按
before 实测纠偏为 34。

**selftest 红的取证规约（S27，r-20261121-200642-ddb1）。** 门禁/复跑的 selftest 输出必须**全量落盘**
（完整 stdout+stderr 重定向到文件）；判红时从全量 log 提取三段——① 全部 `FAIL ` 行（行为用例名+详情）；
② 全部 `file FAIL` 行（perf 文件+error）；③ 末三行（`---` / `cases passed=` / `ok=`）。
**禁止只留计数行**：runner 两种失败形状都会打印名字（行为用例 `FAIL <name>: <detail>`，非中止式、
文件跑完；perf 文件 `file FAIL <file>: <err>`，计数行仍是 `failed=0`）——S27 之前两次 182/1 观测
「只剩计数、无名字」的成因是采集端把 stdout 过滤到只剩末两行，不是 runner 缺陷；失败例名是定向修复的
唯一入口证据，缺它只能停在「环境嫌疑」定性。另注：两种失败形状不可混淆——`cases passed=N failed=M
(M>0)` = 行为用例；`failed=0` + `file FAIL` + `ok=0` = perf 文件阈值被打穿。
