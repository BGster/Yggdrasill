# Yggdrasill · 同步工具桥契约
> **id** mod/tool-bridge · **模块** ③ agent-seed mod · **触发** 改工具桥或凭据解析之前
> **读不到代价** irreversible（凭据泄露不可撤销）

> **定位**：ygg.tool.call 双层白名单（pack.json tools / allowedTools）的指针与反应器边界指引。
> **replaces**：DESIGN §2.11（L352–355）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[mod-package.md](mod-package.md) · [action-faces.md](action-faces.md)
> **搬运**：以下正文自 DESIGN.md L352–L355 逐字迁移，不重写语义。

### 2.11 工具桥契约

同步工具桥（`ygg.tool.call`）是**宿主契约**，不是 Mod 内部细节。`tool.call` / `tool.result` 的 payload 形状、`meta.tool_calls` 的稠密数组编码 `{id, tool, args?}`（空 `args` 必须省略，因为 Lua 空表只能序列化为 `[]`）由宿主消费；修改任一形状必须在同批更新 `pack.json` `inboxRoutes.tool_result.fields` 与宿主解析器。单侧变更属于静默集成缺陷，不是重构；核验 = 工具桥 payload/allowlist 语义用例与双 pack 声明奇偶用例，失败即装载或语义测试拒绝。

#### 双层白名单（Mod 只能执行其中一层）

| 层 | 位置 | 强制时点 | 裁决 |
| --- | --- | --- | --- |
| Layer 1 | `pack.json` → `tools` | 装载时，宿主 | 形状与能力：允许启动哪个程序、固定参数、超时与输出上限 |
| Layer 2 | `data/config.json` → `allowedTools` | 请求时，Mod | 本局发布清单：Mod 可请求哪些已声明工具 |

- `allowedTools` 必须是 `tools` 键的超集，否则宿主拒绝装载；段缺失 = 全部禁止（fail-closed），复制来的 Mod 不能突然执行 `git`。
- Mod 不能读 `pack.json`：`ygg.data` 只解析 `entries` 声明的资源，因此 Mod 只执行 Layer 2。跨层一致性由 `agent_seed_semantics` declaration-parity 用例钉死（双 pack 的 `tools` 深等、`allowedTools` 集合相等、每个 `tool_*` beat 存在），不能靠两个文件碰巧一致。
- Layer 2 是**漂移门而非安全边界**：能改 `data/config.json` 的人也能改 `pack.json`。真正边界是宿主 Layer 1 校验（程序白名单、不经 shell、禁绝对路径与 `..`）加沙箱无 IO。
- 凭据不进任一层文本： `$ENV{VAR}` 由宿主从进程环境、`<mod_dir>/.env.local`、`<mod_dir>/.env` 解析；解析值绝不进入 payload、回执或通知，错误只点名变量。

反应器边界（只写 KV + 叙事、探针失败是警告不是门禁、机证与人证同级）正文见 [state-machine.md](../kernel/state-machine.md) §2.9.2；本节不复制。

#### 2.11.2 Mod 不自己做 IO，也不重复应用结果

- **Mod 从不自己做 IO**：无进程、无网络、无直接文件系统调用。它只发布请求（`meta.tool_calls`）、应用结果（`tool.result`）；唯一可碰的文件面是宿主中介的 `ygg.fs` VFS（run 相对、路径夹紧），`vfs.write` 与 artifact 探针共用同一份有界台账。宿主无 `ygg.fs` 时 `vfs.write` 记同一行台账并叙述 `[VFS LEDGER ONLY]`，不得声称已写盘——两种宿主形态下 KV 与叙述必须一致。
- **结果幂等**：携带已应用过 `external_id` 的 `tool.result` 是**重放**——叙述 `[REPLAY:TOOL]`，不改任何状态，`pending` 计数绝不重复扣减；同一 call 收到**不同的终态**是冲突，不是覆盖，必须显式报冲突而非静默采用后到的那个。
- **其余不变量已各有权威，本节不复制**：双层白名单与凭据见本节上文；receipt / reactive 职责边界、探针失败是警告、机证与人证同级、截断必须诚实见 [state-machine.md](../kernel/state-machine.md) §2.9.2；`k=v;` 输出形状与结算 cross-check 接入点见 [harness-tools.md](../host/harness-tools.md) §2、§3.2。

#### 2.11.1 设计依据（为什么是一回合内联 + 双层白名单）

**当宿主挂载了工具运行器时，白名单工具在同一次 dispatch 回合内内联执行，结果立即折叠进 KV**；未挂载的宿主保留异步 `tool.call` / `tool.result` 回退。**LLM 提供方只能是双协议适配器**（`openai-chat` 缺省 + `anthropic-messages` 回退），且严格只从 git-ignored 的 `.env` 键（`YGG_LLM_*`）配置。

**依据**：每次工具调用走一次异步往返还意味着“探测 → 决策”跨回合，而回合间状态可能已变（另一泳道推进了同一个需求）——内联把“读到的就是判断依据”变回恒真。双协议适配器是因为两种 messages 形状的差异集中在适配层而不在业务层，反过来（业务层分支提供商）会把提供商细节泄到状态机。凭据只走 `.env`：环境的凭据一旦进入 payload / 回执 / 通知，就无法回收。

**判据**：① 阻断——把凭据值写进 payload / 回执 / 通知 / 文档，或在业务层按提供商分岔；② 症状——回执里出现密钥形态字符串，或同一工具结果在不同宿主形态下产生不同 KV；③ 确认——`cargo test -p ygg-sandbox --test agent_seed_semantics` 的工具桥与 declaration-parity 用例，加一次 `grep -rn 'YGG_LLM' <检出的交付面>`（应只在 `.env` 样例与文档键名处）。

维护责任 = 修改工具声明、配置、线协议或反应器的执行者；触发 = 任一相关 commit；检查 = declaration-parity、payload/allowlist、receipt/replay 与 subsystem 语义用例。缺一层同步或把 Layer 2 当安全边界，均不得验收。

