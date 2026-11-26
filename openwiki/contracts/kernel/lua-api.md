---
type: "Reference"
title: "Lua 沙箱宿主 API 契约（`ygg.*`）"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c29ecd25f3a58b8ec34190bd
    resource: repo://crates/ygg-sandbox/src/codec_face/digest.rs
  - id: openwiki-source-853ffa1c94a7f110b92b5cee
    resource: repo://crates/ygg-sandbox/src/codec_face/mod.rs
  - id: openwiki-source-4e18c4b009a3ab754759e346
    resource: repo://crates/ygg-sandbox/src/host_api/mod.rs
  - id: openwiki-source-4b8f115a26c87f1fd42c9902
    resource: repo://crates/ygg-sandbox/src/host_api/sim.rs
  - id: openwiki-source-f5131ab478fa718e924dbf48
    resource: repo://crates/ygg-sandbox/src/inject.rs
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-fef267a23f756637e121d083
    resource: repo://crates/ygg-sandbox/src/sandbox.rs
  - id: openwiki-source-25d364180d1aad1cf34b4353
    resource: repo://crates/ygg-sandbox/tests/sim_face.rs
  - id: openwiki-source-676ac1cfb5831b29b199cae0
    resource: repo://crates/ygg-store/src/mounts/declare.rs
  - id: openwiki-source-3d69fe5377a4bdc3315a6fb1
    resource: repo://crates/ygg-store/src/mounts/store.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# Lua 沙箱宿主 API 契约（`ygg.*`）

> **id** kernel/lua-api · **模块** ① ygg 内核 · **触发** 新增或修改 `ygg.*` 宿主 API 之前
> **读不到代价** hard-refusal（未白名单 API 在加载或调用时被拒）
> **replaces**：DESIGN §3 章题 + §3.1（L383–402）（搬运基线 `3a2536f`）
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[mod/mod-package.md](../mod/mod-package.md) · [mod/action-faces.md](../mod/action-faces.md) · [packages/README.md](../packages/README.md)

## 1. 沙箱面与两级可用面

Lua 5.4 沙箱的 globals 是 deny-by-default 的全新表：14 个 base 纯计算函数（`assert` / `error` / `getmetatable` / `ipairs` / `next` / `pairs` / `rawequal` / `rawget` / `rawlen` / `select` / `setmetatable` / `tonumber` / `tostring` / `type`）、两枚**内核 `pcall` / `xpcall` shim**（见本节末「指令预算」）、三张计算库表 `string`（已摘 `dump`）/ `table` / `utf8`、字符串方法（经 `getmetatable("").__index`：`("ab"):upper()` 可达）、受控 `require` 与冻结的 `ygg` 全局表。`math` / `coroutine` / `os` / `io` / `package` / `debug` 与 `dofile` / `loadfile` / `load` / `print` / `rawset` / `collectgarbage` 仍为 `nil`（逐名断言见 `crates/ygg-sandbox/src/sandbox.rs` 的 `whitelisted_environment_exposes_only_the_allow_list`）。

**两级可用面**：第一级 = 裸沙箱面（`BASE_WHITELIST` + `LIBRARY_WHITELIST`），`Sandbox::new()` 就有；第二级 = **只有 driver 路径**额外注入的 `inject::install`——14 个 `math` 成员（`abs` / `floor` / `ceil` / `fmod` / `modf` / `tointeger` / `ult` / `min` / `max` / `random` / `huge` / `maxinteger` / `mininteger` / `type`）与 `ygg.ticks`。**包只许用第一级**（越级即裸沙箱装载期炸），**mod 两级都可用**。`math.sqrt` 刻意不在面内（`x^0.5` 与 libm 逐位不可复现，定量改用定点整数开方 `world.isqrt`），`math.random` 的序列来自世界内核流 `math.random`（非进程级 C RNG），所以随机数在快照 / 回放里可复现。

> **这条边界是更正的结论，不是转述**：旧文献写的「只暴露安全标准库（`base`、`table`、`string`、`math`、`utf8`、`coroutine`）」有四个方向与实测不符——`coroutine` 全局缺席，`math` 只在 driver 注入后在场，`math` 是精确子集（无 `sqrt` / 超越函数），而 `string` / `table` / `utf8` 属于**第一级**（包与 mod 都能用）。按旧句写会在装载期或调用期 nil-index 崩，而 selftest（普通 Lua）看不出来，所以包侧另有一道静态门禁（边界表归 [packages/README.md](../packages/README.md)）。**判据**：① 阻断——以为「包可以用 `math`」，或按旧文献用 `coroutine` / `math.sin`；② 症状——裸沙箱装载期 `attempt to index a nil value (global 'math')`；③ 确认——`cargo test -p ygg-sandbox --test packages_sandbox --test stdlib_surface`（driver / 裸沙箱 / 源码禁名列三例）。

**指令预算（M2）**：`Sandbox::build` 装一个 `set_global_hook`（每 `BUDGET_QUANTUM` = 1000 条字节码扣一次），默认 `DEFAULT_INSTRUCTION_BUDGET` = 2_000_000 条、上限 `MAX_INSTRUCTION_BUDGET` = 64_000_000 条；耗尽以 `Error::external(BudgetExhausted)` 抛出，脚本侧只能看到一个不透明 userdata，宿主侧用 `budget_exhausted(&err)` 取回并映射成 `E_BUDGET`。预算是**宿主入口的燃料**：`Sandbox::reset_budget()` 在每个入口（模块 chunk 执行、每次 dispatch）重置，所以派生级联共享一份预算。`pcall` / `xpcall` 被换成内核 shim——它们转调真实实现后检查「预算已耗尽」并原样抛回哨兵，因此**拿 pcall 吞掉超时再重试无法续命**。

## 2. API 面与副作用类别

`YGG-MOD-EFFECT` 把每个面划进「模块顶层允许」或「模块顶层禁用」——**顶层副作用在加载期就拒**，不留到运行期。

| API 面 | 签名与返回契约 | 副作用类别 |
| --- | --- | --- |
| `ygg.on(name, fn)` | 注册事件处理器；`mod.lua` 加载完成后封存，运行期或子模块顶层调用即错。 | 顶层禁用 |
| `ygg.emit(name, payload)` | 入队内部派生事件（FIFO，受 `max_cascade_depth` 约束）；同一处理器内 `ygg.choice` 之后调用被丢弃（`YGG-R1`）。 | 顶层禁用 |
| `ygg.narrate(text)` | 向当前迁移的临时叙述缓冲追加**一行**；载荷内嵌换行会物理断行、破坏逐行解析，调用方自行拆行。 | 顶层禁用 |
| `ygg.choice(prompt, options)` | 写 `pending_choice` 并以哨兵错误**立即**终止处理器；玩家选择以新外部事件 `ygg.choice_selected`（1 基索引）到达。 | 顶层禁用 |
| `ygg.meta(key, json)` | 向当前 dispatch 回执的临时 `meta` 映射写结构化元数据（`summary` / `tool_calls` / `harness_actions`）。 | 顶层禁用 |
| `ygg.data(resource_id)` | 纯读取 `pack.json` `entries` 声明的资源，返回深度 Lua 表副本。 | 顶层允许 |
| `ygg.kv.get` / `.set` | 持久 KV，可写值只有四个标量（`nil` / `boolean` / `number` / `string`），表一律拒；复合结构 = 一个键 + 一条确定性编码字符串（口径归 `pkg.codec`）。处理器失败时回滚。 | `get` 允许 / `set` 顶层禁用 |
| `ygg.entity.*` | **单条六面** `spawn` / `get_attribute` / `set_attribute` / `add_tag` / `remove_tag` / `has_tag`；**批式五面** `spawn_batch` / `attr_apply` / `attr_fetch` / `tag_batch` / `scan`；**控制面两键** `exists` / `destroy`（幂等、零事件）；**查询面** `query(selector)`（`tag` / `where` / `select` / `order` 四键，缺 `order` 时 id 升序，不分页不 JOIN）。键集闭集 = 14，**一个逐实体 getter 都没有**。错误码三族只属批式面：`YGG-ENTITY-FRAME`（形状）/ `YGG-ENTITY-VALUE`（非有限值或非标量）/ `YGG-ENTITY-ID`（未知 id）——整批先校验后改，拒绍零副作用。 | 查询允许 / 变更顶层禁用 |
| `ygg.fsm.*` / `ygg.narrative.*` | 内嵌辅助库：`fsm.define` / `fsm.current` / `fsm.on_event`，`narrative.node` / `narrative["goto"]`（点形式是 Lua 5.4 语法错误，由 `YGG-R3` 捕获）。 | 顶层禁用 |
| `ygg.fs.{write,read,stat,list}` | 一个面两个根：`artifacts/<file>`（内存 VFS，dispatch 级回滚）与 `mounts/<name>/<rel>`（挂载桥，写穿磁盘）。见 §3。 | 变更调用顶层禁用 |
| `ygg.sim.*` | 原生模拟内核面（Tier 1），恰好 10 键，见 §4。 | 查询允许 / 变更顶层禁用 |
| `ygg.codec.*` | 确定性 JSON 编解码面（Tier 2 加速面），恰好 3 键，见 §5。 | 纯 |
| `ygg.tool.call(tool_id, args)` | 可选同步宿主工具桥；挂载时内联执行白名单工具并返回 `{ok, data}` / `{ok, error}`，未挂载返回 `{ok = false, unmounted = true}` 让 Mod 回退异步 `meta.tool_calls`。 | 顶层禁用 |

**批式铁律**：数组进出、单帧单次调用承载整批。但批式不是无条件划算——表形数组面的实测成本（485 ns/8 值）**高于** 8 次逐项穿越（≈320 ns），所以批式只在「原生侧能吞掉整批」时才用（见 [packages/entity.md](../packages/entity.md) §9.7）。**不提供 `despawn_batch`**：没有消费者就不该有面（单条真删已由 `destroy` 覆盖）。

## 3. `mounts/` 挂载面（宿主注入的盘上文件桥）

`ygg.fs.*` 是**一个面、两个根，按首段分派**：`artifacts/` 走内存 VFS（逐字节不变），`mounts` / `mounts/` 走本节，其余首段仍归既有 VFS 门。挂载路径只有两种形状：`mounts/<name>`（根本身）与 `mounts/<name>/<rel>`（`<rel>` = 1..=24 个 `/` 分隔段），`<name>` 取自 `pack.json` 的 `mounts[].name`。

**三道边界，缺一即不得上线**：**路径狱**（`YGG-FS-JAIL`）、**尺寸帽**（`YGG-FS-CAP`，拒绍不截断）、**审计行**（`<run-dir>/fs.jsonl`，成功与被拒都记，写成功而审计失败报 `YGG-FS-AUDIT`）。

**拒绝条件（任一命中即 `YGG-FS-JAIL`，Lua 侧可读报错、磁盘零写入）**：① 既不是 `mounts/<name>` 也不以其 `/` 起头（`mountsfoo`、裸 `mounts` 均拒）；② `<name>` 不在声明集内；③ `<rel>` 任一段为空 / `.` / `..` / 含 NUL / 含 ASCII 控制字符；④ 全路径 > 512 字节，或 `<rel>` 段数 > 24；⑤ **包含检查**：`canonicalize` 后的目标必须落在冻结挂载根内，按**路径分量**比较（`Path::starts_with`），绝不做字符串前缀比较（`/a/bc` 与 `/a/b` 的经典漏洞）；⑥ **末段永不跟随**：已存在且 `symlink_metadata` 报软链即拒；⑦ 需要父目录的调用父目录不存在即拒。

**符号链接三句**：挂载根**本身**可以是宿主建的软链，attach 期 `canonicalize` **一次并冻结**、之后不再解析；**中间段**允许软链但其 canonical 结果必须仍在冻结根内（由 ⑤ 保证）；**末段永不允许是软链**（由 ⑥ 保证）——「读穿」与「写穿」两条逃逸同时关闭。`<rel>` 为空只被 `stat` / `list` 接受；`read` / `write` 对空 `rel` 报 `YGG-FS-JAIL`。

**尺寸帽**：`MOUNT_FILE_MAX_BYTES` 4 MiB（读写对称）、`MOUNT_RUN_WRITE_BUDGET` 64 MiB（每次 attach 的累计写预算，**进程级不落盘**，续局拿到新预算）、`MOUNT_PATH_MAX_BYTES` 512、`MOUNT_PATH_MAX_SEGMENTS` 24（词法上界，不是安全边界）、`MOUNT_LIST_MAX_ENTRIES` 16384（无分页：超大目录不可 list，逐路径 `read` / `stat` 不受影响）。

**四个调用的返回形状**：`write` → `{path, bytes, sha256}` 并追加审计行（`mode = ro` 的挂载点报 `YGG-FS-MOUNT`）；`read` → string 或 `nil`（miss 不 throw）；`stat` → `{exists, bytes, sha256, kind}`（目录 `bytes = 0` / `sha256 = ""`）；`list` → `{name, path, bytes, sha256, kind}` 的稠密数组（1..=n，按 `name` 字节序、**不递归**、**不算摘要**——`sha256` 恒为 `""`，要摘要走 `fs.stat`；跳过软链与非 UTF-8 名）。`fs.list()` 无参仍是 `artifacts/` 旧面；有参而不以 `mounts` 起头 → `YGG-FS-JAIL`（不提供按前缀列 artifacts 的第二套遍历）。

**审计行形状**（`seq` 文件内 1 起单调、**无墙钟**，行序即执行序）：`{"bytes":7,"event":"fs.write","mount":"work","path":"mounts/work/a.txt","seq":1,"sha256":"…"}`；被拒行 `event":"fs.write.rejected"` 带 `reason`。被拒也登记——审计的价值一半在「谁试图做了什么」。r1 不并入 `events.jsonl`（那里一行 = 一次 dispatch，`seq` 由提交临界区唯一分配）。

**确定性与回滚**：`artifacts/` 面保持 dispatch 级事务；`mounts/` 面**写穿、不回滚**——外部已发生的写无法撤销，撤销本身是一次未审计的变更。canonical state 不含挂载内容，`state_hash` 与回放都不读它；代价诚实声明：失败 dispatch 内的 `fs.write` 会留痕。

**未挂载桥**：宿主没挂 `MountFs` 时访问 `mounts/…` 抛 `YGG-FS-UNMOUNTED`（不是 `nil` / `false`）——`nil` 与「文件不存在」不可分辨。**明确不做**：目录递归、通配、rename / delete、mtime / 权限面、流式大文件、并行 I/O、挂载点内 mkdir。

**错误码**（一律经 `Error::runtime` 浮出，message 首段即码；信封码仍是 `VALIDATION`）：`YGG-FS-JAIL` / `YGG-FS-CAP` / `YGG-FS-MOUNT` / `YGG-FS-UNMOUNTED` / `YGG-FS-AUDIT`。

## 4. `ygg.sim.*`（Tier 1 原生模拟内核，宿主注入）

`ygg.sim.*` 是 `crates/ygg-sim` 在 Lua 侧的**唯一**进出口：库侧 `set_sim(...)` 注入，CLI 激活触点在 `egg.rs`（宿主面改动走用户授权账）。本面交付「表始终注册 + 未挂载降级」，不在宿主里注入实例。

**恰好 10 个键，一个逐键 getter 都没有**（键集闭集由 `tests/sim_face.rs` 断言，作用是防止退化回逐项访问）：`step(dt)`（`dt` = Q32.32 整数）→ `{tick, hash}`、`hash()`、`snapshot()`（全量字节一次给）、`restore(bytes)` → `{tick, bytes, hash}`（先校验后改）、`spawn_batch(frame)` → `{spawned, next_id}`、`despawn_batch(frame)` → `{despawned}`、`insert_batch(frame)` → `{entities, values}`、`status()` → `{mounted, tick, entities, archetypes, hash}`（单个状态包）、`trace(on)` → `{tracing}`、`replay(snap, log)` → `{ticks, matched, first_mismatch, final_hash}`（在临时 Sim 上重放）。

- **字节帧过面不要求 UTF-8**：`restore` / 三个批量面 / `replay` 参数按 Lua string 的**原始字节**取（Rust 侧 `mlua::String`）。按 `String` 收会把「帧格式校验」降级成与帧内容无关的类型错误。
- **数值过面只有一个闸门**：只接受「值上等于整数的 Lua number」（`n.fract() == 0.0` 且 `|n| ≤ 2^53`），其余一律 `YGG-SIM-FIXED`——静默取整会把「调用方算错了」变成「内核算错了」；定点除零同码（不给 `i64::MAX`，那会把除零藏成数值异常）。
- **字节帧形状**（列式；与快照列编码**共用同一个读写器**）：`spawn_batch: u32 n | u16 comp_n | [u16 len | name]×comp_n | 每组件一列载荷`；`insert_batch` 在 `n` 后多 `u64 ids[n]`；`despawn_batch: u32 n | u64 ids[n]`；每帧尾部 `u64 digest = FNV-1a 64(前面全部字节)`。
- **未挂载降级**：表**始终**在场，宿主没注入实例时**每个**函数抛 `YGG-SIM-UNMOUNTED`——`nil` 与「状态为空」不可分辨，静默降级会让 Mod 把「宿主不支持」读成「世界是空的」。
- **错误码六枚**：`YGG-SIM-UNMOUNTED` / `YGG-SIM-FIXED` / `YGG-SIM-FRAME` / `YGG-SIM-SCHEMA` / `YGG-SIM-ENTITY` / `YGG-SIM-STATE`。六者的共同性质：**状态未变**（先校验后改）。
- **确定性是内核性质，不是本面的纪律**：随机只来自 seed 流派生（与 `pkg.procgen.rng` 位级同源、同一份向量表双向断言）；结构操作 tick 内只入队、tick 边界按 FIFO 应用；`hash` = FNV-1a 64(快照字节去尾 digest)，与 `snapshot` 同源。

## 5. `ygg.codec.*`（Tier 2 确定性 JSON 加速面，宿主注入）

三个键（键集闭集 = 3）：`encode(value)` → 确定性 JSON 原始字节、`decode(text)` → Lua 值（与 `pkg.codec.decode` 同构）、`sha256(bytes)` → 64 位小写 hex（与 `pkg.codec.sha256` 逐字对齐）。注入点在 `host_api::install`（调用 `codec_face::install`），**没有实例、没有 CLI 激活**——本面无状态。

- 包的 Lua 实现仍是**规范执行体**，本面是它的**符合实现**：任一合法输入的产出字节必须与 Lua 路径**逐字节相等**。
- **缺席即回退**（与 `ygg.sim.*` 唯一有意不一致处）：本面没有 `UNMOUNTED` 码，也不注册抛码空表——调用方直接走 Lua 路径，产出字节相同（包契约要求它在无宿主裸沙箱里可用）。`ygg.sim.*` 必须显式抛码，是因为那里 `nil` 与「世界是空的」不可分辨。
- **数值按 Lua 子类型分派**：`Value::Integer` / `Value::Number` 是「`1` 与 `1.0` 是两个字节串」的唯一来源，面侧不做数值合流。
- **带元方法的表整值回退**：Lua 侧 `pairs` 会走 `__pairs`、`value[key]` 会穿 `__index`，复刻它们等于「编码过程中执行 Lua 代码」（纯函数变可重入、可报错、可侧效）。整值回退是语义正确且契约允许的唯一低成本选择。
- **错误码两枚**：`YGG-CODEC-UNSUPPORTED`（值域拒绍或带元方法的表，调用方回退后文案与原实现逐字相同）、`YGG-CODEC-INTERNAL`（内部失败，**不回退**——缺陷必须响，静默降级会把 bug 藏成「性能偶尔不达标」）。**不复制包的六码文案**：那是第二套字符串契约。

## 6. 安全边界与设计依据

**五项硬上限，不是建议**：零盘上 I/O、零网络、零时钟、内存上限（`64 MiB`，`SANDBOX_MEMORY_LIMIT`）、API 白名单 + 确定性种子 PRNG。**「零盘上 I/O」有两个且只有两个出口**：`artifacts/` 的内存 VFS（写不落盘、dispatch 失败整面回滚）与 `mounts/` 的挂载桥（写穿磁盘、不回滚）。

**指令燃料已整体退役（S34，2026-11-22 用户裁定）**：燃料机制（VM 指令计数 hook + 每转换预算 + env 逃生门 + 燃料耗尽稳定码）删除，**替代 = 无**——不再有任何指令级防护（无 watchdog、无时间片）。合法负载不再有预算上限，代价是 Mod 内死循环 / 无限递归无内核级拦截；在「mod 全自研可信、无不可信代码面」的前提下用户裁定接受，开发期症状 = 跑 rt / 测试即挂起（bug 一次即现，非静默）。配套收缩：燃料耗尽码从稳定错误码清单删除、`rt --fuel` 旗删除（传入即 usage error exit 2）、`YGG_FUEL_BUDGET` 成为 no-op。**判据**：① 阻断——以任何形式重新引入指令级计量 / watchdog / 时间片，除非先在「设计依据」立新裁定并给出开销与误杀数据；② 症状——Mod 死循环导致 rt / 测试挂起（预期内，非缺陷）；③ 确认——`grep -rn "fuel\|FUEL" crates/ --include="*.rs"` 为 0 命中（考古白名单 = 本页叙事与 git 历史）。

**`require` 面（受控装载器，无 `package` 全局）**：`require(name)` 只查宿主注入的**已登记模块表**（`ModuleSet`），不做路径拼接；名字是点分标识符（各段 `[A-Za-z0-9_]`），未登记的名字在装载前即可读拒绝。两个命名空间：`src.*`（本 Mod 源码，`pack.json` 的 `code` 段）与 `pkg.<id>.*`（能力包 `packages/libs/<id>/`，`packages` 段）。沙箱里**没有** `package` 全局——造一个具体表会把「解析顺序」交给 mod 可变状态，同一 seed 两次回放可能装到不同模块，破确定性判据。两个载体是两件实事：沙箱 = **模块表注入**（等价物就是 `ModuleSet`）；普通 Lua（`examples/selftest` 与 `ygg-lint` 静态面）= **`package.searchers` 追加项**。两者读**同一份**包索引，结构上不可能漂移。模块 chunk 顶层一律是纯导出面（`ShadowBanned`），chunk 名带真实路径（包模块是相对包根的真实路径，如 `@fixture/init.lua`）。

**为什么权限面必须在加载期封死**：内容审查被简化为「JSON Schema 校验 + `ygg-lint` + `playtest`」三件可离线跑的事，运行期不能再靠权限收束。反方案（运行期按需给 `os` / `io` 开口）会让「同一份内容在任何宿主上结果相同」失效、回放不再可重现。

**判据（本页整体）**：① 阻断——向白名单添加未在 `safe_stdlib` 里的面、引入非确定性来源（时钟 / 随机 / 环境读）、把 AST 分析搬进 `ygg-sandbox`；② 症状——同一 seed 两次回放产出不同回执，或 `YGG-MOD-EFFECT` 在加载期未拒绝顶层副作用；③ 确认——`cargo test --workspace`（含 `ygg-lint` 与沙箱语义用例）、`cargo run -q -p ygg-lint -- --path <mod>`、`cargo test -p ygg-sandbox --test sim_face` / `--test codec_face`。

## 7. 单文件上限与拆分纪律（S-25）

宿主装配体按面拆分，**纯搬移不是重写**：`host_api` 由单文件 2880 行拆为 `host_api/` 11 文件（`mod.rs` 装配 + `value` / `narrate` / `fs` / `tool` / `kv` / `entity` / `entity_decode` / `selector` / `sim` / `meta` 十面），`install()` 拆为逐面薄函数后注册序不变（on / emit / narrate / choice → meta → fs → tool → kv → entity → sim → codec）。同法：`ygg-store` 的 `mounts/` 4 文件、`codec_face/` 4 文件、`lib.rs` 门面 + `artifact.rs` + `resolve.rs`（新模块为私有 `mod`，符号经 crate 根 `pub use` 转口，`ygg_store::` 下的路径逐字节不变）。

**拆分的行为不变量由门禁计数守恒背书**（workspace / egg / pkg selftest / agent-seed / superagent 各门禁 before-after 逐项相等 + clippy 警告集合相等），零可见性放宽。拆分动机只有一个：单文件超限使面边界不可读——拆完的形状必须与本章的分面叙述一一对应。**判据**：① 阻断——以「顺手改行为」的方式拆分，或为拆分放宽可见性；② 症状——门禁计数不再守恒、外部路径变化；③ 确认——拆分前后各跑一遍全部门禁并逐个 binary 比对计数。
