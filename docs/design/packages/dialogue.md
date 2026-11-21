# Yggdrasill · packages/dialogue — 实体间对话事件总线与 handler 回退链

> **id** packages/dialogue · **模块** ⑤ packages · 通用能力层 · **触发** 改 handler 注册面、回退链、记忆窗口或 agent 通道之前
> **读不到代价** irreversible（agent 通道一旦长出第二个 egress，凭据与网络出口在包内扩散，事后审计查不出来）

> **上游**：[../README.md](../README.md) · **相关**：[README.md](README.md)（模板与命名规则）· [quest.md](quest.md)（任务事件驱动对白）· [../kernel/lua-api.md](../kernel/lua-api.md)（`ygg.tool.call` 是唯一出口）· [../kernel/events.md](../kernel/events.md)（级联深度帽）

## 1. 边界（这个包不做什么）

- **不写对话内容**：文案是数据（`ygg.data` 文本资源 / mod 的 `data/texts`），包只搬文本键。
- **不实现 handler 语义**：`builtin:<name>` 由 mod 用 `dialogue.on` 注册；包不内置任何角色或剧情。
- **不新增网络出口**：`agent` handler 只经既有 `ygg.tool.call` 工具桥；包内不得出现 HTTP、socket、子进程。
- **不做渲染、不发 choice**：呈现归 mod 的 view 面。
- **不做记忆压缩**：窗口按条数截断，不做摘要（摘要是 AI 侧的策略，属 mod）。
- **不持有时钟**：超时由工具桥的调用超时决定，包只读其结果。

## 2. 数据形状

逻辑形状（代码看见的）：`event` = 运行时消息帧、`handlers.<type>` = 注册序的 handler 数组、`threads.<key>` = 线程轮次、`dlg.seq` = 轮次计数。

**物理落点（KV 只收标量——D10）**：宿主 `ygg.kv.set` 经 `value_from_lua` 只接受四个标量型（`crates/ygg-sandbox/src/host_api/value.rs:35`），表写入即 `unsupported payload type: table`。所以「表值」的字段全部落成**扁平标量键 + 换行名表**：

| 键 | 值 | 说明 |
| --- | --- | --- |
| `dlg.seq` | number | 轮次计数；**唯一写口 = 认领后的 `remember` / `say`** |
| `dlg.types` | string | 有 handler 的 type 名表（升序） |
| `dlg.h.<type>.names` | string | handler 名表——**注册序，不排序**（I7：名表序 = 执行序） |
| `dlg.h.<type>.n.<name>.kind` | string | `builtin` / `agent` |
| `dlg.h.<type>.n.<name>.opts` | string | `opts` 的键名表（升序） |
| `dlg.h.<type>.n.<name>.opt.<key>` | 标量 | `memory` / `window` / `tool` / `credential_key` …（只放数据） |
| `dlg.keys` | string | 线程键名表（升序）——**D2 的索引**：`ygg.kv` 没有遍历，没有它则导出与 I4 的泄漏检测都不可达 |
| `dlg.thread.<key>.turns` | number | 该线程已存轮数（窗口内） |
| `dlg.thread.<key>.t.<i>.who` / `.text` / `.seq` | number / string / number | 第 `i` 轮（1 基）；`who = 0` 表示系统；`text` 是**文本键**不造文案 |

线程键 `key`：`memory = "pair"`（缺省）时 = `"<source>:<target>"`；`memory = "entity"` 时 = `"<source>"`。`source` / `target` 用 `0` 表示「系统」，不用 `nil`（表里 `nil` 值等于缺键，且 [codec.md](codec.md) I7 拒绝数组洞）。

**type 名允许 `.`、handler 名禁止 `.`**：quest 的派生事件名（`quest.advanced` 等）本身带点，桥把它**直接当 dialogue 的 type 用**（不做字符串改写）；若 handler 名也允许点，`dlg.h.a.n.n.x.kind` 就能由两组 `(type, name)` 拼出，两套注册会写到同一批键上。

**函数不进 KV（D3）**：`builtin` handler 需要 Lua 函数，而函数不可序列化（[codec.md](codec.md) I7 亦拒 `function`）。于是 `on(type, spec)` 拆两处——KV 只存 `{ kind, name, opts }`，函数进模块局部 `REGISTRY`（键 = `type .. ":" .. name`）。重载后 **KV 有 spec 而无函数 = 未绑定**：`emit` 走 I2 的回退链，未处理时发 `dialogue.failed`（`reason = "unbound"`），不是静默不响应。

**`off` 之后的墓碑**：KV 没有删除面，`off` 只从名表里摘掉一行；被删 handler 的 `n.<name>.*` 键留在原处（同名重注册会覆盖它们，所以墓碑不会无限长）。这是宿主无删除面的直接代价，与 [entity.md](entity.md) I5 的墓秃同源。

**`agent` 的工具实参是扁平标量（D11）**：宿主 `tool_args_from_lua` 只收「字符串键 → 标量值」，嵌套表直接拒。所以帧不整体下传，agent 侧能看到的固定五键是 `type` / `source` / `target` / `text_key` / `credential_key`（全部转成字符串，缺省空串）。

`opts` 里 `agent` handler 的凭据只写 `.env` 的**键名**（如 `credential_key = "NPC_SHOPKEEPER_TOKEN"`，形态 `^[A-Z][A-Z0-9_]*$`），值不进 pack、不进 KV、不进事件、不进工具实参。

## 3. 不变量

- **I1 通道封闭**：handler 的执行只有两条路——`builtin:<name>`（mod 注册的 Lua 函数）与 `agent`（经 `ygg.tool.call` 的既有工具桥）。包内出现第二个 egress 即为违规，无论它多「方便」。
- **I2 回退链必存在**：`agent` handler 超时或返回 `ok = false` 时，回退到同 `type` 的 `builtin` handler；未注册 `builtin` 则 emit `dialogue.failed` 并把错误挂进回执 meta，不静默吞掉。
- **I3 环深度帽**：一次对话链的**同步嵌套** emit 深度 `≤ max_depth`（缺省 4），超限 emit `dialogue.depth_capped` 并停止当前链（防止两个 NPC 无限对聊）。**宿主没有级联深度帽**（`max_cascade_depth` 只活在两句文档里，`grep -rni cascade --include=*.rs crates/` 命中 0——D8），所以本包的帽是这条链的**唯一**守卫；`ygg.emit` 入队的派生事件由 driver 排空，不共享本链，深度从 1 重算。
- **I4 记忆窗口有界**：`memory = "pair"` 时线程历史 `≤ window` 条（缺省 12），超出丢最旧；`memory = "none"` 完全不写 KV。
- **I5 无时钟**：超时判定不得依赖 `ygg.ticks()`（有 API 无推进者，恒 0，见 [../kernel/state-surface.md](../kernel/state-surface.md) §2.8.1）。
- **I6 凭据只留键名**：包内任何路径都不得把凭据值写进 KV、事件、bundle 或回执 meta。
- **I7 注册序即执行序**：同一 `type` 的多个 handler 按注册序执行，第一个返回「已处理」的终止链；顺序不因 `require` 先后而变（注册只发生在 `install` 里）。
- **I8 顶层纯度**：包模块顶层只有纯导出，`ygg.on` / 注册一律在 `M.install(deps)`。

## 4. API 面

| 函数 | 签名 | 返回 | 副作用类别 |
| --- | --- | --- | --- |
| `dialogue.on` | `(type, spec)` | `true` | 写（`dlg.handlers`） |
| `dialogue.off` | `(type, name)` | `boolean` | 写 |
| `dialogue.emit` | `(type, source, target, payload?)` | `{ handled, by, depth }` | 写 + 事件（派生） |
| `dialogue.say` | `(thread_key, text)` | `boolean` | 写（追加一轮，受 I4 截断） |
| `dialogue.history` | `(thread_key)` | array（副本） | 纯读 |
| `dialogue.register_adapter` | `(name, adapter)` | `true` | 写；`adapter = { send(frame) → reply, health() → ok }` |
| `dialogue.adapters` | `()` | array（名列表） | 纯读 |
| `dialogue.frame` | `(type, source, target, payload?)` | table（消息帧） | 纯读；供 adapter 与自测断言帧形状 |
| `dialogue.handlers` | `(type)` | array（注册序副本） | 纯读 |
| `dialogue.threads` | `()` | `{key, …}`（升序） | 纯读 |
| `dialogue.keys` | `()` | `{键, …}`（升序） | 纯读；`codec.export({ keys = dialogue.keys() })` 的点名面（D6） |

**错误码（逐条可断）**：码面 `YGG-PKG-DIALOGUE-<COND>`。

| 码 | 触发 |
| --- | --- |
| `-INSTALL` | `install(deps)` 收到非表；`deps.quest` 无 `EVENTS` 表 |
| `-TYPE` | type 名非法；`spec` 形状坏（`kind` 不在词表、`name` 非法、`opts` 值非标量、`memory` 不在词表、`tool` 非串、`credential_key` 不合形态）；`source`/`target` 不是 ref；`payload` 非表；`text` 非串；线程键非法；adapter 形状坏 |
| `-DUP` | 同 type 下同名 handler 重复注册；adapter 重名 |
| `-UNBOUND` | 已导出但**未抛**：未绑定的 handler 走回退链，未处理时 `dialogue.failed` 的 `reason` 是 `"unbound"`（可断言，不静默） |
| `-WINDOW` | `opts.window` / `config.window` 不是正整数 |
| `-DEPTHCFG` | `config.max_depth` 不是正整数 |
| `-HOST` | `ygg` 不可达且未注入 `deps.host`（环境面，与 `entity.md` 的 `-HOST` 同义） |

## 5. 性能预算

**口径声明（先读再读数字——D12）**：实测值产在 **selftest 面**（普通 Lua + mock KV），沙箱里既无 `os.clock` 也无 `collectgarbage`（`sandbox.rs` 的白名单测试钉死）。内存那行是**包侧 Lua 堆上界**（含 mock KV 表开销）。复算：`cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-dialogue`（三次取中位）。

| 指标 | 阈值（Goal 原则二） | 实测与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`；**实测 0.7423 ms**（三次取中位；单次 0.71–0.97 ms。测法同 [entity.md](entity.md) §5） |
| 实体按 id 查找 / 更新 | `O(1)` | **不适用**（本包不做实体索引）。附测：`history(thread)` 每次只读一条线程，线程数 1 / 100 / 1k 三档 **21.6 / 22.3 / 23.0 µs**，档间比 **1.069×**（三次取中位；不设契约阈值，仅作回归参考） |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：线程随 `dlg` 前缀由 [codec.md](codec.md) §5 基准集覆盖 |
| 10k 实体常驻内存 | 给出数字 | 1k 线程 × 12 轮 **实测 3653.5 KiB**（硬帽 `16 MiB`，实测数字已入档）。**与 2 MiB 目标值不可比**：36k 枚键的每枚键开销（唯一键名字符串 + 表槽 ≈ 100 B）由 mock KV 主导，包侧无省余量；真实宿主后端（SQLite / JSON）的开销结构不同，此数只能当上界读 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 100 轮 `emit → history`（`memory = "none"`）后 `dlg.thread.*` 键数**恒 0**、`dlg.keys` 不落，GC 差 ≤ 1% |

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 对话与事件文本 | 事件总线 + 文本键；对白内容全在数据里 | P3 |
| CDDA | NPC 对话 | `builtin` handler + `pair` 记忆窗口；NPC 记得上一次见面 | P3 |
| Saltline | 终端日志 / 旁白 | `memory = "none"` + `dialogue.say` 单轮广播（同一个包承载两种完全不同的用法） | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.dialogue.`（例 `pkg.dialogue.fallback-chain`、`pkg.dialogue.depth-capped`）。
- **断言面**：I1 用「包目录内不出现 `http` / `socket` / `os.execute` / `io.` 字面量」的静态用例 + adapter 只经 `ygg.tool.call` 的可调用面断言；I2 / I3 / I4 各一条拒绝路径用例（断事件名与次数）；I6 用「凭据键名的值不出现在任何 KV 值里」。
  - 静态扫描的字面量清单就是上句这四个（外加 `ygg.data(` 与 `ticks(`），扫描对象是 **manifest `packages` 段里在册的源文件**。源码注释里**不写**这些字面量，否则扫描会命中自己（开发期实测两次假红）。
- **计数（D4）**：26 条（22 行为 + 4 性能）计入 `cargo run -q --example selftest -p ygg-sandbox -- --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-dialogue` 的 `cases passed=`。载体是 `packages/mods/pkg-selftest/`；生产 Mod（`agent-seed`）不声明包，它的 `cases passed=` 永远是 79/0，这是回归钉。
- **禁止宽松断言**：只断「调用成功」不断回退链走过几跳，视同没测 I2。

## 8. 设计依据

**为什么对话是事件总线而不是对话树。** 对话树把「说什么」与「什么时候能说」绑死在一份静态结构里，而四个复现项目里真正需要的是「某件事发生了，谁来响应」：任务推进要有人说话、NPC 见到玩家要打招呼、终端要播日志。事件 + handler 注册表能同时承载这三件事，且新增一种响应 = 注册一个 handler，不改包。

**反方案一：每个 mod 自己接 agent。** 同一段 HTTP 调用、同一套凭据处理、同一份超时兜底会在四个复现里各写一遍，凭据暴露面乘以四。v0.1 设计裁定「agent handler 属游戏基建」，正是为了把这些收进一个可审计的位置。

**反方案二：对话节点图（narrative.node / goto）。** 已由宿主的 `ygg.narrative.*` 提供，它的语义是「故事节点跳转」，适合线性剧情，不承担 handler 回退、记忆窗口、环深度帽这些运行时职责。两层的分工是：`narrative` 管剧情图，`dialogue` 管实体间说话这件事。

**判据（①②③）。** ① 阻断——包内新增网络出口；agent 失败静默；记忆无限增长；两个 NPC 互相回话不设深度帽。② 症状——NPC 对话实现每个复现一份、超时之后再没人说话、一次对话跑成死循环、凭据值出现在 bundle 或日志里。③ 确认——本文件被索引；`selftest` 含 `pkg.dialogue.*`；§5 五个指标有数字。
