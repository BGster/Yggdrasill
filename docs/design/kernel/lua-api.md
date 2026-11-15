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
| `ygg.fs.{write,read,stat,list}` | 可选宿主注入的 VFS 接口，严格限定于 `artifacts/*.md`。`write(path, content)` 返回 `{ bytes, sha256 }`。 | 变更调用 **模块顶层禁用** |
| `ygg.tool.call(tool_id, args_table)` | 可选同步宿主工具桥。挂载时内联执行白名单工具并返回 `{ ok = true, data = { ... } }` 或 `{ ok = false, error = "..." }`。未挂载时返回 `{ ok = false, unmounted = true }`，使 Mod 回退到异步 `meta.tool_calls`。 | **模块顶层禁用** |

#### 3.1.1 设计依据（沙箱安全边界 / 语言方言 / 边界划分）

**沙箱的安全边界是六项硬上限，不是建议**：零 I/O、零网络、零时钟、指令燃料上限、内存上限（`16 MiB`）、API 白名单（`safe_stdlib`）与确定性种子 PRNG。**语言标准是标准 Lua 5.4**（内置 `mlua`），带指令计数钩子燃料；标准库被加固——移除 `os` / `io` / `debug` / `package` / `dofile` / `loadfile`，`require` 换成纯沙箱加载器，种子走确定性 `luai_makeseed`。**`crates/ygg-sandbox` 封装 `mlua`、宿主 API 白名单与内嵌的 `ygg` 标准库；`crates/ygg-lint` 提供独立的 AST 静态分析。**

**依据**：内容审查被简化为「JSON Schema 校验 + `ygg-lint` + `playtest`」三件可离线跑的事，因此运行期不能再靠权限收束——权限面必须在加载期就封死。反方案（在运行期给 `os`/`io` 按需开口）会让「同一份内容在任何宿主上结果相同」失效，且回放不再可重现。确定性种子与燃料上限是回放可重现的前提；把两者拆到两个 crate 是因为静态分析与执行沙箱的失败模式完全不同（前者发诊断，后者 fail-closed）。

**判据**：① 阻断——向白名单添加未在 `safe_stdlib` 里的标准库面、引入非确定性来源（时钟/随机/环境读）、或把 AST 分析搬进 `ygg-sandbox`；② 症状——同一 seed 两次回放产出不同回执，或 `YGG-MOD-EFFECT` 在加载期未拒绝顶层副作用；③ 确认——`cargo test --workspace`（含 `ygg-lint` 与沙箱语义用例）与 `cargo run -q -p ygg-lint -- --path <mod>`。

