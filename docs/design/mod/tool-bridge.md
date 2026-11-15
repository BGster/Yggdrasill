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

维护责任 = 修改工具声明、配置、线协议或反应器的执行者；触发 = 任一相关 commit；检查 = declaration-parity、payload/allowlist、receipt/replay 与 subsystem 语义用例。缺一层同步或把 Layer 2 当安全边界，均不得验收。

