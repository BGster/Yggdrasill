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

| 字段 | 类型 | 必填 | 缺省 | 持久化位置 |
| --- | --- | --- | --- | --- |
| `event` | table `{ type, source, target, payload }`（source/target 是实体 ref） | 是 | — | 不持久化（运行时入参） |
| `handlers.<type>` | array（`{ kind = "builtin"\|"agent", name, opts }`，按注册序） | 是 | — | `ygg.kv` 键 `dlg.handlers` |
| `threads.<key>` | table `{ turns = { … }, depth = n }` | 否 | `{}` | `ygg.kv` 键 `dlg.thread.<key>`；`memory = "none"` 时不落 |
| `dlg.seq` | number（轮次计数） | 是 | `0` | `ygg.kv` 键 `dlg.seq` |

线程键 `key`：`memory = "pair"`（缺省）时 = `"<source>:<target>"`；`memory = "entity"` 时 = `"<source>"`。

`opts` 里 `agent` handler 的凭据只写 `.env` 的**键名**（如 `credential_key = "NPC_SHOPKEEPER_TOKEN"`），值不进 pack、不进事件日志。

## 3. 不变量

- **I1 通道封闭**：handler 的执行只有两条路——`builtin:<name>`（mod 注册的 Lua 函数）与 `agent`（经 `ygg.tool.call` 的既有工具桥）。包内出现第二个 egress 即为违规，无论它多「方便」。
- **I2 回退链必存在**：`agent` handler 超时或返回 `ok = false` 时，回退到同 `type` 的 `builtin` handler；未注册 `builtin` 则 emit `dialogue.failed` 并把错误挂进回执 meta，不静默吞掉。
- **I3 环深度帽**：一次对话链的嵌套 emit 深度 `≤ max_depth`（缺省 4），超限 emit `dialogue.depth_capped` 并停止当前链（与宿主 `max_cascade_depth` 同向，防止两个 NPC 无限对聊）。
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

## 5. 性能预算

| 指标 | 阈值（Goal 原则二） | 本包取值与测量方法 |
| --- | --- | --- |
| 包加载耗时 | `< 10ms` | 目标 `≤ 1ms`（纯 Lua，`install` 不连 adapter、不读 KV）。测法同 [entity.md](entity.md) §5。 |
| 实体按 id 查找 / 更新 | `O(1)` | **不适用**：本包不做实体索引；线程键是一次 KV 直取（`dlg.thread.<key>`），无遍历。 |
| 序列化吞吐（实体 / 秒，双向） | 附基准集与耗时 | **不适用**：线程随 `dlg` 前缀由 [codec.md](codec.md) §5 基准集覆盖。 |
| 10k 实体常驻内存 | 给出数字 | 目标 1k 线程 × 12 轮 `≤ 2 MiB`；硬帽 `16 MiB`。口径同 [entity.md](entity.md) §5。 |
| 泄漏检测 | 导入导出 100 轮内存不增长 | 100 轮 `emit → history`（`memory = "none"`）后 `dlg.thread.*` 键数不变（0），内存差 `≤ 1%`。 |

## 6. 用点计划

| 复现项目 | 用点 | 该包提供什么 | 阶段 |
| --- | --- | --- | --- |
| PokeRogue | 对话与事件文本 | 事件总线 + 文本键；对白内容全在数据里 | P3 |
| CDDA | NPC 对话 | `builtin` handler + `pair` 记忆窗口；NPC 记得上一次见面 | P3 |
| Saltline | 终端日志 / 旁白 | `memory = "none"` + `dialogue.say` 单轮广播（同一个包承载两种完全不同的用法） | P3 |

## 7. 自测与验收

- **用例名前缀**：`pkg.dialogue.`（例 `pkg.dialogue.fallback-chain`、`pkg.dialogue.depth-capped`）。
- **断言面**：I1 用「包目录内不出现 `http` / `socket` / `os.execute` / `io.` 字面量」的静态用例 + adapter 只经 `ygg.tool.call` 的可调用面断言；I2 / I3 / I4 各一条拒绝路径用例（断事件名与次数）；I6 用「凭据键名的值不出现在任何 KV 值里」。
- **计数**：计入 `cargo run -q --example selftest -- --mod .ygg/mods/agent-seed` 的 `cases passed=`（发现机制见 [README.md](README.md) §5 未决项）。
- **禁止宽松断言**：只断「调用成功」不断回退链走过几跳，视同没测 I2。

## 8. 设计依据

**为什么对话是事件总线而不是对话树。** 对话树把「说什么」与「什么时候能说」绑死在一份静态结构里，而四个复现项目里真正需要的是「某件事发生了，谁来响应」：任务推进要有人说话、NPC 见到玩家要打招呼、终端要播日志。事件 + handler 注册表能同时承载这三件事，且新增一种响应 = 注册一个 handler，不改包。

**反方案一：每个 mod 自己接 agent。** 同一段 HTTP 调用、同一套凭据处理、同一份超时兜底会在四个复现里各写一遍，凭据暴露面乘以四。v0.1 设计裁定「agent handler 属游戏基建」，正是为了把这些收进一个可审计的位置。

**反方案二：对话节点图（narrative.node / goto）。** 已由宿主的 `ygg.narrative.*` 提供，它的语义是「故事节点跳转」，适合线性剧情，不承担 handler 回退、记忆窗口、环深度帽这些运行时职责。两层的分工是：`narrative` 管剧情图，`dialogue` 管实体间说话这件事。

**判据（①②③）。** ① 阻断——包内新增网络出口；agent 失败静默；记忆无限增长；两个 NPC 互相回话不设深度帽。② 症状——NPC 对话实现每个复现一份、超时之后再没人说话、一次对话跑成死循环、凭据值出现在 bundle 或日志里。③ 确认——本文件被索引；`selftest` 含 `pkg.dialogue.*`；§5 五个指标有数字。
