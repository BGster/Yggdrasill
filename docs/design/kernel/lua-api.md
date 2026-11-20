# Yggdrasill · Lua 沙箱宿主 API 契约（ygg.*）
> **id** kernel/lua-api · **模块** ① ygg 内核 · **触发** 新增或修改 ygg.* 宿主 API 之前
> **读不到代价** hard-refusal（未白名单 API 在加载或调用时被拒）

> **定位**：ygg.* 全 API 面签名、返回契约与副作用类别（含接口契约章题与章内导航）。
> **replaces**：DESIGN §3 章题 + §3.1（L383–402）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[mod-package.md](../mod/mod-package.md) · [action-faces.md](../mod/action-faces.md)
> **搬运**：以下正文自 DESIGN.md L383–L384 + DESIGN.md L385–L402 逐字迁移，不重写语义。**唯一例外**：§3.1 的首句按实测更正（见该节末尾的「这一句是更正，不是搬运」）。

## 3. 接口契约（规范线协议与 API 面）

### 3.1 Lua 沙箱宿主 API 契约（`ygg.*`）

Lua 5.4 沙箱**不暴露任何标准库表**：globals 是一张 deny-by-default 的全新表，只有 `BASE_WHITELIST` 的 16 个 base 纯计算函数（`assert` / `error` / `getmetatable` / `ipairs` / `next` / `pairs` / `pcall` / `rawequal` / `rawget` / `rawlen` / `select` / `setmetatable` / `tonumber` / `tostring` / `type` / `xpcall`）、字符串方法（经 `getmetatable("").__index`：`("ab"):upper()` 可达而 `string` 全局为 `nil`）、受控 `require` 与冻结的 `ygg` 全局表。`string` / `table` / `utf8` / `coroutine` / `math` / `os` / `io` / `package` 与 `dofile` / `loadfile` / `load` / `print` / `rawset` / `collectgarbage` 全为 `nil`（`crates/ygg-sandbox/src/sandbox.rs` 的 `whitelisted_environment_exposes_only_the_allow_list` 逐名断言）；`debug` 同理——不在白名单里就取不到。

**两级可用面（包与 mod 的分界）**：第一级 = 上面这张裸沙箱面，`Sandbox::new()` 就有；第二级 = **只有 driver 路径**额外注入的 `inject::install`（`crates/ygg-sandbox/src/inject.rs`）——15 个 `math` 成员（`abs` / `floor` / `ceil` / `sqrt` / `fmod` / `modf` / `tointeger` / `ult` / `min` / `max` / `random` / `huge` / `maxinteger` / `mininteger` / `type`）与 `ygg.ticks`。**包只许用第一级**（越级即裸沙箱装载期炸），**mod 两级都可用**。判据：① 阻断——按旧文献写 `table.insert` / `string.format`，或认为「包可以用 `math`」；② 症状——裸沙箱装载期 `attempt to index a nil value (global 'math')`，或 `table` 为 nil 的同类崩；③ 确认——`cargo test -p ygg-sandbox --test packages_sandbox` 三条用例（①driver / ②裸沙箱 / ③源码禁名列）。

> **这一句是更正，不是搬运**：本节其余正文自 DESIGN 逐字迁移，但原文「只暴露安全标准库（`base`、`table`、`string`、`math`、`utf8`、`coroutine`）」与实测**三个方向都不符**：`table` / `utf8` / `coroutine` / `string` 全局缺席（只余字符串方法），`math` 只在 driver 注入之后在场，`base` 也只是 16 个纯计算子集。按旧句写会在装载期或调用期 nil-index 崩，而 selftest（普通 Lua）看不出来——所以包侧另有一道静态门禁（`packages_sandbox.rs` 用例 ③），边界表归 [../packages/README.md](../packages/README.md) §5。

| API 面 | 签名与返回契约 | 副作用类别（`YGG-MOD-EFFECT`） |
| --- | --- | --- |
| `ygg.on(event_name, handler_fn)` | 注册事件处理器 `function(payload)`。`mod.lua` 加载完成后即封存；在运行时或子模块顶层调用即为错误。 | **模块顶层禁用**（仅入口 `mod.lua` 或 `M.install`） |
| `ygg.emit(event_name, payload)` | 入队一个内部派生事件（FIFO，受 `max_cascade_depth` 上限约束）。若在同一处理器中 `ygg.choice` 之后调用则被丢弃（`YGG-R1`）。 | **模块顶层禁用** |
| `ygg.narrate(text)` | 向当前迁移的临时叙述回执缓冲区追加一行。 | **模块顶层禁用** |
| `ygg.choice(prompt, options_array)` | 将 `pending_choice` 写入状态并**立即**通过哨兵错误终止当前处理器。玩家选择以新外部事件 `ygg.choice_selected`（1 基整数索引）到达。 | **模块顶层禁用** |
| `ygg.meta(key, json_value)` | 将结构化元数据（`summary`、`tool_calls`、`harness_actions`）写入当前 dispatch 回执的临时 `meta` 映射。 | **模块顶层禁用** |
| `ygg.data(resource_id)` | 纯读取，返回 `pack.json` `entries` 中声明的、经 Schema 校验的 JSON 资源的深度 Lua 表副本。 | 模块顶层允许（纯读取） |
| `ygg.kv.get(key)` / `ygg.kv.set(key, val)` | 读取或写入持久化键值状态。可写值只有**四个标量**：`nil` / `boolean` / `number` / `string`——表一律被拒（`value_from_lua` 报 `unsupported payload type`，core 的 `Value` 只有四个标量变体）。复合结构 = **一个键 + 一条确定性编码字符串**（口径归 `pkg.codec`，见 [packages/README.md](../packages/README.md) §5）。处理器失败时回滚。 | `get` 允许；`set` **模块顶层禁用** |
| `ygg.entity.*` | `spawn()`、`get_attribute(id, k)`、`set_attribute(id, k, v)`、`add_tag(id, t)`、`remove_tag(id, t)`、`has_tag(id, t)`。返回数字实体 ID（`YGG-R2` 要求将 ID 持久化到 `ygg.kv`）。**无 `exists` / `destroy`**（与 [packages/entity.md](../packages/entity.md) §1 一致：id 永不复用，死亡=墓碑降级；回收面演进见 Goal 排期 S9）。 | 查询允许；变更 **模块顶层禁用** |
| `ygg.fsm.*` / `ygg.narrative.*` | 内嵌 Lua 标准库辅助，用于扁平有限状态机（`fsm.define`、`fsm.current`、`fsm.on_event`）与故事节点（`narrative.node`、`narrative["goto"]`；点形式 `narrative.goto` 是 Lua 5.4 语法错误，由 `YGG-R3` 捕获）。 | `define` / `node` / `goto` **模块顶层禁用** |
| `ygg.fs.{write,read,stat,list}` | 可选宿主注入的两根文件面：`artifacts/<file>`（内存 VFS，dispatch 级回滚）与 `mounts/<name>/<rel>`（挂载桥，写穿磁盘）。`write(path, content)` 返回 `{ path, bytes, sha256 }`。两根的文法、尺寸帽、返回形状与错误码见 [§3.1.2](#312-mounts-挂载面宿主注入的盘上文件桥)。 | 变更调用 **模块顶层禁用** |
| `ygg.sim.*` | 可选宿主注入的**原生模拟内核面**（Tier 1）：`step` / `hash` / `snapshot` / `restore` / `spawn_batch` / `despawn_batch` / `insert_batch` / `status` / `trace` / `replay`——恰好 10 个键，**一个逐实体 getter 都没有**（批式铁律：bytes/数组进出、单帧单调用）。全部字节帧带尾 digest，坏帧先拒后改。未挂载时**每个函数**抛 `YGG-SIM-UNMOUNTED`（不是 `nil` / `false`）。形状、`16 位小写 hex` 与错误码见 [§3.1.3](#313-sim-模拟内核面宿主注入)。 | 查询允许；变更调用 **模块顶层禁用**（静态 `YGG-MOD-EFFECT` 规则与宿主激活同批接，排期 S1；本行运行期面已注册，未挂载即拒） |
| `ygg.tool.call(tool_id, args_table)` | 可选同步宿主工具桥。挂载时内联执行白名单工具并返回 `{ ok = true, data = { ... } }` 或 `{ ok = false, error = "..." }`。未挂载时返回 `{ ok = false, unmounted = true }`，使 Mod 回退到异步 `meta.tool_calls`。 | **模块顶层禁用** |

**`require` 面（受控装载器，无 `package` 全局）**：`require(name)` 只查宿主注入的**已登记模块表**（`ModuleSet`），不做任何路径拼接；名字是点分标识符（各段字符集 `[A-Za-z0-9_]`），未登记的名字在装载前即可读拒绝。表里有**两个命名空间**：

| 命名空间 | 来源 | 声明面 |
| --- | --- | --- |
| `src.*` | 本 Mod 的模块源码 | `pack.json` 的 `code` 段（§3.1 上方） |
| `pkg.<id>.*` | 能力包 `packages/libs/<id>/` | `pack.json` 的 `packages` 段（见 [mod-package.md §3.2.3](../mod/mod-package.md)） |

沙箱里**没有** `package` 全局（`package.path` / `package.searchers` 均不可见）——这是白名单测试钉死的红线：造一个具体表会把「解析顺序」交给 mod 可变状态，同一 seed 两次回放可能装到不同模块，破确定性判据。所以「挂入 `package.path`」在本仓的两个载体上是两件实事：沙箱 = **模块表注入**（无路径概念，等价物就是 `ModuleSet`）；普通 Lua（`examples/selftest` 与 `ygg-lint` 的静态面）= **`package.searchers` 追加项**（Lua 5.4 的正规扩展点）。两者读**同一份**包索引（`模块名 → 相对包根路径`），结构上不可能漂移。

模块 chunk 顶层一律是纯导出面（`ShadowBanned`），包模块与本 Mod 模块**同受此约束**；chunk 名带真实文件路径，包模块是**相对包根**的真实路径（如 `@fixture/init.lua`），不是 `path_of` 会推出的 `pkg/fixture.lua`。

#### 3.1.1 设计依据（沙箱安全边界 / 语言方言 / 边界划分）

**沙箱的安全边界是六项硬上限，不是建议**：零盘上 I/O、零网络、零时钟、指令燃料上限、内存上限（`64 MiB`，`sandbox.rs` 的 `SANDBOX_MEMORY_LIMIT`）、API 白名单（`safe_stdlib`）与确定性种子 PRNG。

**「零盘上 I/O」有两个且只有两个出口**：`artifacts/` 的内存 VFS（写不落盘，dispatch 失败整面回滚）与 `mounts/` 的挂载桥（写穿磁盘、不回滚）。挂载桥不是「第三根」的入口，而是把宿主声明的挂载点变成沙箱可见目录；它由三道边界收束，缺一即不得上线：**路径狱**（`mounts/<name>/<rel>` 之外一律 `YGG-FS-JAIL`，含末段软链与路径分量包含检查）、**尺寸帽**（单件 4 MiB / 每次 attach 的写预算 64 MiB / 单次 list 16384 条目，超限 `YGG-FS-CAP`）、**审计行**（每次写——成功与被拒——追加 `<run-dir>/fs.jsonl` 一行，写成功而审计失败报 `YGG-FS-AUDIT`）。① 阻断：向 Lua 白名单添加「有盘上副作用且不经挂载桥」的 I/O 面（放出 `io`/`os`，或让 `ygg.fs.*` 接受 `artifacts/`、`mounts/` 之外的第三根）；② 症状：同一 seed 两次回放产出不同的外部文件集合，或 `mounts/…` 之外的路径能读到宿主任意文件，或某次成功写没有 `fs.jsonl` 行；③ 确认——`cargo test -p ygg-store --test mounts`（狱向量 + 审计行）、`cargo test -p ygg-sandbox --test mounts`（面 + 未挂载降级）、`grep -c '"event":"fs.write"' <run-dir>/fs.jsonl`。**语言标准是标准 Lua 5.4**（内置 `mlua`），带指令计数钩子燃料；标准库被加固——移除 `os` / `io` / `debug` / `package` / `dofile` / `loadfile`，`require` 换成纯沙箱加载器，种子走确定性 `luai_makeseed`。**`crates/ygg-sandbox` 封装 `mlua`、宿主 API 白名单与内嵌的 `ygg` 标准库；`crates/ygg-lint` 提供独立的 AST 静态分析。**

**依据**：内容审查被简化为「JSON Schema 校验 + `ygg-lint` + `playtest`」三件可离线跑的事，因此运行期不能再靠权限收束——权限面必须在加载期就封死。反方案（在运行期给 `os`/`io` 按需开口）会让「同一份内容在任何宿主上结果相同」失效，且回放不再可重现。确定性种子与燃料上限是回放可重现的前提；把两者拆到两个 crate 是因为静态分析与执行沙箱的失败模式完全不同（前者发诊断，后者 fail-closed）。

**判据**：① 阻断——向白名单添加未在 `safe_stdlib` 里的标准库面、引入非确定性来源（时钟/随机/环境读）、或把 AST 分析搬进 `ygg-sandbox`；② 症状——同一 seed 两次回放产出不同回执，或 `YGG-MOD-EFFECT` 在加载期未拒绝顶层副作用；③ 确认——`cargo test --workspace`（含 `ygg-lint` 与沙箱语义用例）与 `cargo run -q -p ygg-lint -- --path <mod>`。

#### 3.1.2 mounts 挂载面（宿主注入的盘上文件桥）

`ygg.fs.*` 是一个面、两个根，**按首段分派**：`artifacts/` 走上一节的既有内存 VFS（逐字节不变），`mounts` / `mounts/` 走本节（本节的所有拒绍一律 `YGG-FS-JAIL`）。**其余首段仍归既有 VFS 门**（`artifacts/` 前缀门的文案与行为一字不改）——旧面的逐字节兼容优先于错误码统一，`YGG-FS-JAIL` 只覆盖挂载面内部的路径。

**路径文法**：沙箱可见的挂载路径只有两种形状——`mounts/<name>`（挂载根本身）与 `mounts/<name>/<rel>`（`<rel>` = 1..=24 个 `/` 分隔段）。`<name>` 取自 `pack.json` 的 `mounts[].name`（声明面见 [mod-package.md §3.2.2](../mod/mod-package.md)），不在声明集内即拒。段**不**套用制品面的 `[A-Za-z0-9._/-]`：狱是**包含性**属性而不是字符集属性，过度收紧会让中文名 / 空格名的业务目录不可用，且不增加安全性。

**拒绝条件**（任一命中即 `YGG-FS-JAIL`，Lua 侧可读报错、磁盘零写入）：

1. 既不是 `mounts/<name>` 也不以 `mounts/<name>/` 起头（`mountsfoo`、裸 `mounts` 均拒）；
2. `<name>` 不在声明集内；
3. `<rel>` 任一段为空（`//`、结尾 `/`）、为 `.` 或 `..`、含 NUL、含 ASCII 控制字符（`<0x20` 或 `0x7f`）；
4. 全路径 > 512 字节，或 `<rel>` 段数 > 24；
5. **包含检查**：`canonicalize` 之后的目标必须落在冻结的挂载根之内，比较按**路径分量**（`Path::starts_with`），绝不做字符串前缀比较（`/a/bc` 与 `/a/b` 的经典漏洞）；
6. **末段永不跟随**：目标已存在且 `symlink_metadata` 报 `is_symlink()` → 拒；
7. 需要父目录的调用（`read` / `write` / `stat`）父目录不存在 → 拒。

`<rel>` 为空（即 `mounts/<name>`）只被 `stat` 与 `list` 接受——它就是一个目录；`read` / `write` 对空 `rel` 报 `YGG-FS-JAIL`。

**符号链接策略（三句）**：挂载根**本身**可以是宿主建的软链（`mounts[].dir` 的 bind 语义，见 [cli-daemon.md](../host/cli-daemon.md) 的「run 目录文件契约」），它在 attach 期 `canonicalize` **一次并冻结**，之后不再解析；**中间段**允许是软链，但其 canonical 结果必须仍在冻结根内（由 5 保证）；**末段永不允许是软链**（由 6 保证）——于是「读穿到别处」与「写穿到别处」两条逃逸同时关闭。

**尺寸帽**（超限 `YGG-FS-CAP`，**拒绍而不截断**——截断会把半个文件交给 Mod，是静默错答）：

| 常量 | 值 | 口径 |
| --- | --- | --- |
| `MOUNT_FILE_MAX_BYTES` | 4 MiB（4 194 304） | 单次 `read` / `write` 的字节上限（读写对称） |
| `MOUNT_RUN_WRITE_BUDGET` | 64 MiB（67 108 864） | 每次 attach 的累计写预算。**进程级、不落盘**：跑飞闸不需要跨重启账本，续局拿到新预算 |
| `MOUNT_PATH_MAX_BYTES` | 512 | 沙箱路径字节上限（含 `mounts/` 前缀） |
| `MOUNT_PATH_MAX_SEGMENTS` | 24 | `<rel>` 段数上限（词法上界，不是安全边界） |
| `MOUNT_LIST_MAX_ENTRIES` | 16384 | 单次 `list` 的条目上限（无分页：超大目录不可 list，逐路径 `read` / `stat` 不受影响） |

**四个调用的返回形状**（新面；`artifacts/` 面不变）：

| 调用 | `mounts/<name>[/<rel>]` |
| --- | --- |
| `write(path, content)` | 同步写穿磁盘，返回 `{ path, bytes, sha256 }`，并追加一行审计（见下）。`mode = ro` 的挂载点拒（`YGG-FS-MOUNT`） |
| `read(path)` | 命中 → string；miss → `nil`（与 `artifacts/` 面同口径：miss 不 throw） |
| `stat(path)` | `{ exists, bytes, sha256, kind }`；`kind` 取 `file` / `dir`；目录 `bytes = 0`、`sha256 = ""`；miss 时 `exists = false`、`kind = ""` |
| `list(path)` | `{ name, path, bytes, sha256, kind }` 的**稠密数组**（1..=n），按 `name` 字节序升序，**不递归**。**不算摘要**——对每行算 sha256 是 O(总字节)，会把「看一眼目录」变成「读一遍全盘」，要摘要走 `fs.stat`，故 `sha256` 恒为 `""`（键在场、值留白）。**跳过软链条目**（「末段永不是软链」的同一条纪律：列表不得把链接目标冒充成 file/dir）；非 UTF-8 名同理跳过——沙箱路径是 Rust `String`，它们本来就不可寻址 |

`fs.list()` 无参时仍是 `artifacts/` 的旧面（字节级不变）；有参且以 `mounts` 起头走本面；有参而**不**以 `mounts` 起头 → `YGG-FS-JAIL`（不提供「按前缀列 artifacts」的第二套遍历语义）。

**未挂载桥**：宿主没挂 `MountFs` 时，访问 `mounts/…` 抛 `YGG-FS-UNMOUNTED`（不是 `nil` / `false`）——`nil` 与「文件不存在」不可分辨，静默降级会把「宿主没挂」读成「文件没了」。

**审计行**：落 `<run-dir>/fs.jsonl`（append-only、懒创建），`seq` 在文件内 1 起单调、**无墙钟**（行序即执行序）：

```json
{"bytes":7,"event":"fs.write","mount":"work","path":"mounts/work/a.txt","seq":1,"sha256":"…"}
{"bytes":0,"event":"fs.write.rejected","mount":"work","path":"mounts/work/../x","reason":"YGG-FS-JAIL","seq":2,"sha256":""}
```

**被拒也登记**：审计的价值一半在「谁试图做了什么」，只记成功等于对探测行为留白。写成功而审计 append 失败 → `YGG-FS-AUDIT`（诚实报「已写未审」，不假装成功）。行形状刻意与事件行同键（`seq` / `event` / 事实字段），后续若要并入 `events.jsonl` 只改写入点、不改读者；r1 **不并轨**——`events.jsonl` 一行 = 一次 dispatch，其 `seq` 由提交临界区唯一分配，带外追加会打乱唯一来源。

**错误码**（一律经 `Error::runtime` 浮出，message 首段即码；信封码仍为 `code::VALIDATION`——新增信封码要动映射面与下游按码分支的消费者，收益只是省一次 message 前缀解析）：

| 码 | 语义 |
| --- | --- |
| `YGG-FS-JAIL` | 路径狱拒绍（首段非两根本 / 段非法 / 越界 / 父缺失 / 末段软链 / 目标是目录——`read` / `write` 不能把目录当文件） |
| `YGG-FS-CAP` | 尺寸帽拒绍（单件 / 写预算 / list 条目数） |
| `YGG-FS-MOUNT` | 声明非法（attach 期）或挂载点状态冲突；写只读挂载 |
| `YGG-FS-UNMOUNTED` | 宿主未挂载文件桥 |
| `YGG-FS-AUDIT` | 已写未审 |

**确定性与回滚**：`artifacts/` 面保持 dispatch 级事务（失败 dispatch 回滚内存 VFS）；`mounts/` 面**写穿、不回滚**——挂载点是沙箱唯一的对外面，外部已发生的写不可能被「撒销」，而撒销本身是一次未审计的变更。canonical state 不含挂载内容，`state_hash` 与回放都不读它。代价诚实声明：失败 dispatch 内的 `fs.write` 会留下痕迹，这正是审计行「成功与被拒都记」的分量所在。

**明确不做**：目录递归、通配、rename / delete、mtime / 权限面、流式大文件、并行 I/O、挂载点内的 mkdir（父目录不存在即拒）。

**性能（绝对数字；本需求不做优化，因此不主张任何提速）**——recipe：`cargo test -p ygg-store --release --test mounts -- --nocapture mount_io_budget_report`（macOS / APFS，各取三次中位数）：

| 操作 | 中位数 | 口径 |
| --- | --- | --- |
| attach（1 目录挂载 + 1 私有） | 0.067 ms | 「已存在且一致 → 静默通过」路径；每条声明 ≤3 次 syscall |
| write 4 MiB | 20.190 ms | 含一次真实写 + 一行审计 append |
| read 4 MiB | 0.327 ms | canonicalize 父 + symlink_metadata + 一次 read |
| stat 4 MiB | 15.991 ms | 摘要要读全文件（与 `artifacts/` 面同口径） |
| list 1 000 条目 | 1.047 ms | 不递归、不算摘要 |
| list 4 000 条目 | 4.745 ms | 同上；>16384 条目 → `YGG-FS-CAP` |

用例只设**故障探测阈值**（任一单操作 > 500 ms 即判定「有隐藏的树扫描」并回炉），**不设性能目标**。


#### 3.1.3 sim 模拟内核面（宿主注入）

`ygg.sim.*` 是 Tier 1（`crates/ygg-sim`，契约 [sim-kernel.md](sim-kernel.md)）在 Lua 侧的**唯一**进出口。它和 `ygg.fs` 的 `mounts/` 面同款纪律：库侧 `set_sim(...)` 注入（`driver.rs`），CLI 激活触点在 `egg.rs`（宿主面改动走用户授权账，**排期 S1**）；本需求交付「表始终注册 + 未挂载降级」，**不**在宿主里注入实例。

**十个键，一个逐键 getter 都没有**（批式铁律 ②：mlua 边界穿越 ~µs，细粒度调用是负收益）。键集是**闭集**——多一个键即红（`tests/sim_face.rs` 的键集断言），这条断言的作用是防止未来退化回逐项访问：

| 键 | 签名 | 返回 | 副作用 |
| --- | --- | --- | --- |
| `step` | `ygg.sim.step(dt)`，`dt` = Q32.32 整数 | `{ tick = int, hash = "16 hex" }` | 变更模拟态（一次 tick 一次穿越） |
| `hash` | `ygg.sim.hash()` | `"16 hex"` | 纯 |
| `snapshot` | `ygg.sim.snapshot()` | string（快照帧字节） | 纯（全量字节一次给） |
| `restore` | `ygg.sim.restore(bytes)` | `{ tick = int, bytes = int, hash = "16 hex" }` | 变更模拟态（先校验后改） |
| `spawn_batch` | `ygg.sim.spawn_batch(frame)` | `{ spawned = int, next_id = int }` | 变更模拟态（列式批量） |
| `despawn_batch` | `ygg.sim.despawn_batch(frame)` | `{ despawned = int }` | 变更模拟态（列式批量） |
| `insert_batch` | `ygg.sim.insert_batch(frame)` | `{ entities = int, values = int }` | 变更模拟态（列式批量） |
| `status` | `ygg.sim.status()` | `{ mounted = true, tick, entities, archetypes, hash }` | 纯（单个状态包，避免 N 次 getter） |
| `trace` | `ygg.sim.trace(on)` | `{ tracing = bool }` | 变更（逐 tick 证据缓冲开关） |
| `replay` | `ygg.sim.replay(snap, log)` | `{ ticks, matched, first_mismatch = int\|nil, final_hash = "16 hex" }` | 纯（在临时 Sim 上重放） |

**数值过面只有一个闸门**：Lua 的双精度与内核的定点之间，`ygg.sim.*` 只接受「**值上等于整数的 Lua number**」（`n.fract() == 0.0` 且 `|n| ≤ 2^53`），其余一律 `YGG-SIM-FIXED`——静默取整会把「调用方算错了」变成「内核算错了」。定点除零同码（不定义 0 的近似值：给 `i64::MAX` 会把除零藏成数值异常）。

**字节帧形状**（列式，可直拷；与快照列编码**共用同一个读写器**，所以「快照能解的列、批量也能解」是结构性质而不是两套代码的巧合）：

```text
spawn_batch:   u32 n | u16 comp_n | [u16 name_len | name]×comp_n | 每组件一列载荷
insert_batch:  u32 n | u64 ids[n] | u16 comp_n | [u16 name_len | name]×comp_n | 每组件一列载荷
despawn_batch: u32 n | u64 ids[n]
每帧尾部:      u64 digest = FNV-1a 64（前面的全部字节）
```

**未挂载降级**：`ygg.sim` 表**始终**在场，但宿主没注入实例时**每个**函数抛 `YGG-SIM-UNMOUNTED`。不返回 `nil` / `false` / 空表：`nil` 与「状态为空」不可分辨，静默降级会让 Mod 把「宿主不支持」读成「世界是空的」并据此写下错误状态。这一个码把「宿主没这个能力」变成装载期就能看见的硬停。

**错误码**（`YGG-SIM-*` 六码，全部经 `Error::runtime` 浮出，message 首段即码；信封码不新增——与 `YGG-FS-*` 同一处置）：`YGG-SIM-UNMOUNTED`（未注入）、`YGG-SIM-FIXED`（非整数值过面 / 定点除零）、`YGG-SIM-FRAME`（magic / 版本 / 长度 / 尾 digest / 列类型不符，含坏快照）、`YGG-SIM-SCHEMA`（未知组件名、同帧重复组件名、类型与列不符）、`YGG-SIM-ENTITY`（未知实体 id、`insert` 目标不存在、`next_id` 溢出）、`YGG-SIM-STATE`（内部不变量破坏）。**六个码的共同性质：状态未变**——先校验后改，拒绍不产生半改的模拟态。

**确定性不是本面的纪律而是内核的性质**：`step` 的随机数只来自 seed 流派生（`(seed, stream)` → 四字状态，与 `pkg.procgen.rng` 位级同源、同一份向量表双向断言）；结构操作在 tick 内只入队、tick 边界按 FIFO 应用；`hash` = `FNV-1a 64(快照字节去尾 digest)`，与 `snapshot` 同源。所以「同 seed + 同输入帧 → 同 `hash`」是可用一条等式验的，不需要 Mod 侧自证。

**判据**：① 阻断——给本面加逐实体 getter（`get_attribute(id, k)` 式的 N 次穿越），或让未挂载态返回 `nil` 而不是抛码；② 症状——同一 seed 两次 `step` 得到不同 `hash`（说明某处引了时钟 / 外部随机源 / 无序容器遍历），或 Mod 在未挂载宿主上静默算出空世界；③ 确认——`cargo test -p ygg-sim`（逐 tick hash + 跨进程 digest + 禁浮点/禁时钟/禁 `HashMap` 静态扫描）与 `cargo test -p ygg-sandbox --test sim_face`（键集闭集、未挂载降级、非整数拒绍）。
