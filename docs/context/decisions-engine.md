# Yggdrasill · 架构决策台账 · 核心引擎与平台（D1–D21）

> **定位**：引擎与平台层架构决策的权威台账（含台账维护规则，与 decisions-agent 共用同一段引言）。
> **replaces**：CONTEXT §5 引言 + §5.1（L134–161）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[decisions-agent.md](decisions-agent.md) · [principles.md](principles.md)
> **搬运**：以下正文自 CONTEXT.md L134–L137 + CONTEXT.md L138–L161 逐字迁移，不重写语义。

## 5. 关键架构决策台账（`D1–D21` 与 `M-Agent` 演进）

> 任何新的项目级架构决策，或对既有决策的修订，都必须在实现它的同一次提交中记录到本表（**原地修订对应行**，编号顺延但决策行只改内容不追加变更叙事——流水账不进本文档，历史归 git 与结算产物）。

### 5.1 核心引擎与平台决策（`D1–D21`）

| ID | 决策领域 | 裁决与理由 | 状态 |
| --- | --- | --- | --- |
| **D1** | 初始运行时目标 | **Web 优先**：通过 `ygg-wasm` 实现纯浏览器可玩，表现与逻辑严格分离；微信小游戏与 Godot 作为后续目标。 | 已确认 |
| **D3** | 内核规格 | **从第 1 天起即为完整 RPG 级规则内核**（实体、属性、标签、事件总线、FSM、Lua 条件），使文字冒险通过增加内容/子系统演进为 RPG，而非重写内核。 | 已确认 |
| **D4** | 首个里程碑策略 | **先引擎 + 手写参考 Mod（`test-adventure`）**，待内核语义得到证明后再接 AI 生成流水线（`M1`）。 | 已确认 |
| **D6** | 游戏引擎集成 | **Godot 原生链接（`gdext`）** 排期于 M2.5（原生链接 Rust 库，无 Wasm 开销）；保持表现/逻辑分离。 | 已排期（Spike GO） |
| **D7** | 内核语言 | **单一 Rust 实现**：编译为浏览器 Wasm（`ygg-wasm`）、原生链接进 Godot（`gdext`），并在权威服务器上原生运行（`egg` / Axum；原名 `ygg-agent`，已更名）。保证无 GC 抖动的确定性回放。 | 已确认 |
| **D8** | Web 表现技术栈 | **React + TypeScript**（`packages/client-web`），用于以文本为中心的叙事 UI 与成熟的 AI 工具生态。 | 已确认 |
| **D9** | 平台服务（M2+） | **Rust + Axum**，与内核直接共享 `ygg-schema` 校验逻辑。 | 已计划 |
| **D10** | 内容流水线技术栈 | **TypeScript**（`packages/pipeline`），用于 I/O 密集的 LLM 编排、结构化输出解析与重试循环。 | 已确认 |
| **D11** | Mod 契约 | **Lua 脚本作为唯一可执行接口**：叙事流、效果与条件以 Lua 编写；静态内容存放于 JSON 资源文件，经 `ygg.data(...)` 加载。取代更早的声明式 JSON 条件/对话树草案。 | 已确认 |
| **D12** | Crate 边界 | **`crates/ygg-sandbox`** 封装 `mlua`、宿主 API 白名单与内嵌的 `ygg` Lua 标准库；`crates/ygg-lint` 提供独立的 AST 静态分析。 | 已确认 |
| **D13** | 沙箱安全边界 | **零 I/O、零网络、零时钟、指令燃料上限、内存上限（`16 MiB`）、API 白名单（`safe_stdlib`）与确定性种子 PRNG**。内容审查简化为 JSON Schema 校验 + `ygg-lint` + `playtest`。 | 已确认 |
| **D14** | 持久化状态权威 | **持久化状态仅存在于 Rust（`ygg.kv` / `ygg.entity`）**。Lua 局部/全局变量在单次 dispatch 内是临时的；`ygg-lint`（`YGG-R2`）标记未持久化的实体句柄。 | 已确认 |
| **D15** | 外部数据与 Schema 单一来源 | **资源 JSON 文件 + `packages/schema/schemas/` 中的单一来源 JSON Schema**，由 Rust（`jsonschema`）与 TypeScript（`ajv`）做完全一致的校验。 | 已确认 |
| **D16** | Lua 方言 | **标准 Lua 5.4**（经内置 `mlua`），带指令计数钩子燃料、加固的标准库（移除 `os`/`io`/`debug`/`package`/`dofile`/`loadfile`，`require` 替换为纯沙箱加载器），以及确定性的 `luai_makeseed`。 | 已确认 |
| **D17** | 并发与分片模型 | **单实例同步 FIFO 确定性内核 + 宿主侧分片**：每个 `KernelFacade` 严格单线程 FIFO；水平扩展使用相互隔离的 `KernelFacade` 分片，仅通过规范的 tick 边界消息通道通信。已在 1 万实体上验证亚毫秒 dispatch（`probe_determinism`）。 | 已确认 / 保留 |
| **D18** | LLM 提供方抽象 | **双协议适配器（`openai-chat` 默认 + `anthropic-messages` 回退）**，严格通过 git-ignored 的 `.env` 键（`YGG_LLM_*`）配置。 | 已确认 |
| **D19** | 试玩验证契约 | `playtest` 产出的**机器可读 8 键 JSON 报告**（贪心 option-1 遍历、自定义 `--choices`、`--max-steps`），通过 Mod 声明的 `agent.endingPrefix` 将结局归类为 `ending` / `choice` / `limit` / `deadend`。 | 已确认 |
| **D20** | 资源模板单一来源 | **`scene-texts.json` 与 `agent-config.json` 的 Schema 规范地存放于 `packages/schema/schemas/`**；LLM 填充数据实例，绝不自创 Schema 定义。 | 已确认 |
| **D21** | egg 全局化与全局唯一 serve | **链式软链**：`~/.local/bin/egg` → `<repo>/.ygg/bin/egg`（安装副本为唯一事实源，重建后重跑 `scripts/install-egg.sh` 即同步；cargo clean 无感、免 sudo、零墓碑）；**单实例按端口成立**（r1 修订）：启动前只读探针 `GET /api/roots`（零锁文件）——自家实例幂等复用退出 0 / 旧代无端点实例拒绝并提示重启 / 外姓占用由 bind 失败兑住；**根集 = 进程内注册表**（r1 修订）：空根启动，`POST/DELETE /api/roots` 运行时注册/注销（runs 父目录或单个 run-root；重启为空；`--runs-root` 与 serve.json 退役），**拒绝文件系统爬扫**（面板不得出现未注册过的根；注册 = 观察配置非局状态写）。 | 已确认 |

