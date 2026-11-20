# Yggdrasill · Lua 沙箱宿主 API 契约（ygg.*）
> **id** kernel/lua-api · **模块** ① ygg 内核 · **触发** 新增或修改 ygg.* 宿主 API 之前
> **读不到代价** hard-refusal（未白名单 API 在加载或调用时被拒）

> **定位**：ygg.* 全 API 面签名、返回契约与副作用类别（含接口契约章题与章内导航）。
> **replaces**：DESIGN §3 章题 + §3.1（L383–402）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[mod-package.md](../mod/mod-package.md) · [action-faces.md](../mod/action-faces.md)
> **搬运**：以下正文自 DESIGN.md L383–L384 + DESIGN.md L385–L402 逐字迁移，不重写语义。

## 3. 接口契约（规范线协议与 API 面）

### 3.1 Lua 沙箱宿主 API 契约（`ygg.*`）

Lua 5.4 沙箱只暴露安全标准库（`base`、`table`、`string`、`math`、`utf8`、`coroutine`）外加冻结的 `ygg` 全局表：

| API 面 | 签名与返回契约 | 副作用类别（`YGG-MOD-EFFECT`） |
| --- | --- | --- |
| `ygg.on(event_name, handler_fn)` | 注册事件处理器 `function(payload)`。`mod.lua` 加载完成后即封存；在运行时或子模块顶层调用即为错误。 | **模块顶层禁用**（仅入口 `mod.lua` 或 `M.install`） |
| `ygg.emit(event_name, payload)` | 入队一个内部派生事件（FIFO，受 `max_cascade_depth` 上限约束）。若在同一处理器中 `ygg.choice` 之后调用则被丢弃（`YGG-R1`）。 | **模块顶层禁用** |
| `ygg.narrate(text)` | 向当前迁移的临时叙述回执缓冲区追加一行。 | **模块顶层禁用** |
| `ygg.choice(prompt, options_array)` | 将 `pending_choice` 写入状态并**立即**通过哨兵错误终止当前处理器。玩家选择以新外部事件 `ygg.choice_selected`（1 基整数索引）到达。 | **模块顶层禁用** |
| `ygg.meta(key, json_value)` | 将结构化元数据（`summary`、`tool_calls`、`harness_actions`）写入当前 dispatch 回执的临时 `meta` 映射。 | **模块顶层禁用** |
| `ygg.data(resource_id)` | 纯读取，返回 `pack.json` `entries` 中声明的、经 Schema 校验的 JSON 资源的深度 Lua 表副本。 | 模块顶层允许（纯读取） |
| `ygg.kv.get(key)` / `ygg.kv.set(key, val)` | 读取或写入持久化键值状态（`nil`、`boolean`、`number`、`string` 或可 JSON 序列化的表）。处理器失败时回滚。 | `get` 允许；`set` **模块顶层禁用** |
| `ygg.entity.*` | `spawn()`、`exists(id)`、`destroy(id)`、`get_attribute(id, k)`、`set_attribute(id, k, v)`、`add_tag(id, t)`、`remove_tag(id, t)`、`has_tag(id, t)`。返回数字实体 ID（`YGG-R2` 要求将 ID 持久化到 `ygg.kv`）。 | 查询允许；变更 **模块顶层禁用** |
| `ygg.fsm.*` / `ygg.narrative.*` | 内嵌 Lua 标准库辅助，用于扁平有限状态机（`fsm.define`、`fsm.current`、`fsm.on_event`）与故事节点（`narrative.node`、`narrative["goto"]`；点形式 `narrative.goto` 是 Lua 5.4 语法错误，由 `YGG-R3` 捕获）。 | `define` / `node` / `goto` **模块顶层禁用** |
| `ygg.fs.{write,read,stat,list}` | 可选宿主注入的两根文件面：`artifacts/<file>`（内存 VFS，dispatch 级回滚）与 `mounts/<name>/<rel>`（挂载桥，写穿磁盘）。`write(path, content)` 返回 `{ path, bytes, sha256 }`。两根的文法、尺寸帽、返回形状与错误码见 [§3.1.2](#312-mounts-挂载面宿主注入的盘上文件桥)。 | 变更调用 **模块顶层禁用** |
| `ygg.tool.call(tool_id, args_table)` | 可选同步宿主工具桥。挂载时内联执行白名单工具并返回 `{ ok = true, data = { ... } }` 或 `{ ok = false, error = "..." }`。未挂载时返回 `{ ok = false, unmounted = true }`，使 Mod 回退到异步 `meta.tool_calls`。 | **模块顶层禁用** |

#### 3.1.1 设计依据（沙箱安全边界 / 语言方言 / 边界划分）

**沙箱的安全边界是六项硬上限，不是建议**：零盘上 I/O、零网络、零时钟、指令燃料上限、内存上限（`16 MiB`）、API 白名单（`safe_stdlib`）与确定性种子 PRNG。

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

