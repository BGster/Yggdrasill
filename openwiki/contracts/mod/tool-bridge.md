---
type: "Reference"
title: "同步工具桥契约"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-c1b4b807feda7647ba638e19
    resource: repo://.ygg/mods/agent-seed/data/config.json
  - id: openwiki-source-cfe8ec9e3ed7fc4dc881025e
    resource: repo://.ygg/mods/agent-seed/mod.lua
  - id: openwiki-source-8339725b1afcf7133cd86688
    resource: repo://.ygg/mods/agent-seed/pack.json
  - id: openwiki-source-9f00464f869376ffc138f1bd
    resource: repo://.ygg/mods/agent-seed/src/runtime/tools.lua
  - id: openwiki-source-6c8df78132e639681d1122c3
    resource: repo://crates/ygg-sandbox/src/host_contract/tools.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# 同步工具桥契约

> **id** mod/tool-bridge · **模块** ③ agent-seed mod · **触发** 改工具桥或凭据解析之前
> **读不到代价** irreversible（凭据泄露不可撤销）
> **replaces**：DESIGN §2.11（L352–355）（搬运基线 `3a2536f`）
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[mod-package.md](mod-package.md) · [action-faces.md](action-faces.md) · [../kernel/state-machine.md](../kernel/state-machine.md) · [../host/harness-tools.md](../host/harness-tools.md)

## 1. 它是宿主契约，不是 Mod 内部细节

同步工具桥（`ygg.tool.call`）的 `tool.call` / `tool.result` payload 形状、`meta.tool_calls` 的稠密数组编码 `{id, tool, args?}`（**空 `args` 必须省略**，因为 Lua 空表只能序列化为 `[]`）由宿主消费。修改任一形状必须在同批更新 `pack.json` 的 `inboxRoutes.tool_result.fields` 与宿主解析器——**单侧变更属静默集成缺陷，不是重构**。核验 = 工具桥 payload / allowlist 语义用例 + 双 pack 声明奇偶用例，失败即装载或语义测试拒绝。

## 2. 双层白名单（Mod 只能执行其中一层）

| 层 | 位置 | 强制时点 | 裁决 |
| --- | --- | --- | --- |
| **Layer 1** | `pack.json` → `tools` | 装载时，宿主 | 形状与能力：允许启动哪个程序、固定参数、超时与输出上限 |
| **Layer 2** | `data/config.json` → `allowedTools` | 请求时，Mod | 本局发布清单：Mod 可请求哪些已声明工具 |

- `allowedTools` 必须是 `tools` 键的**超集**，否则宿主拒绝装载；段缺失 = 全部禁止（fail-closed）——复制来的 Mod 不能突然执行 `git`。
- **Mod 读不到 `pack.json`**：`ygg.data` 只解析 `entries` 声明的资源，所以 Mod 只执行 Layer 2。
- 跨层一致性由 `selftest` 的 declaration-parity 用例钉死（双 pack 的 `tools` 深等、`allowedTools` 集合相等、每个 `tool_*` beat 存在），不靠两个文件碰巧一致。
- **Layer 2 是漂移门而非安全边界**：能改 `data/config.json` 的人也能改 `pack.json`。真正边界是宿主 Layer 1 校验（程序白名单、不经 shell、禁绝对路径与 `..`）加沙箱无 IO。

## 3. 凭据

`$ENV{VAR}` 由宿主从进程环境、`<mod_dir>/.env.local`、`<mod_dir>/.env` 依次解析；**解析值绝不进入 payload、回执或通知**，错误只点名变量（键名，不含值）。解析实现见 [../host/harness-tools.md](../host/harness-tools.md) 与 `crates/ygg-sandbox/src/host_contract/tools.rs`。

## 4. Mod 不自己做 IO，也不重复应用结果

- **无进程、无网络、无直接文件系统调用**：Mod 只发布请求（`meta.tool_calls`）、应用结果（`tool.result`）。唯一可碰的文件面是宿主中介的 `ygg.fs` VFS（run 相对、路径夹紧）；`vfs.write` 与 artifact 探针共用同一份有界台账。宿主无 `ygg.fs` 时 `vfs.write` 记同一行台账并叙述 `[VFS LEDGER ONLY]`，**不得声称已写盘**——两种宿主形态下 KV 与叙述必须一致。
- **挂载桥激活语义（S-1，2026-11-21）**：`ygg.fs.*` 的 `mounts/` 面由宿主在 CLI 两内核构造点（开局 / 热载）装配（`activate_host_faces`：读 `pack.json` 的 `mounts` 段 → `MountStore::attach` → `set_mount_fs`）。真 CLI 里声明了 mounts 的 mod 写盘即写穿 `<run>/mounts/`（审计落 `fs.jsonl`），不再 `YGG-FS-UNMOUNTED`；热载重 attach 走三态幂等，失败沿用旧内核。未接桥的宿主形态（库内裸 `KernelFacade`）仍统一报 `YGG-FS-UNMOUNTED`：码在首段，`nil` 与「文件不存在」不可分辨的静默降级被禁止。
- **结果幂等**：携带已应用过 `external_id` 的 `tool.result` 是**重放**——叙述 `[REPLAY:TOOL]`，不改任何状态，`pending` 计数绝不重复扣减；同一 call 收到**不同的终态**是冲突、不是覆盖，必须显式报冲突，而不是静默采用后到的那个。

其余不变量各归其权威，本页不复制：receipt / reactive 职责边界、探针失败是警告、机证与人证同级、截断必须诚实见 [../kernel/state-machine.md](../kernel/state-machine.md) §效应折叠边界；`k=v;` 输出形状与结算 cross-check 接入点见 [../host/harness-tools.md](../host/harness-tools.md) §2、§3.2。

## 5. 设计依据：为什么是一回合内联 + 双层白名单

宿主挂载了工具运行器时，白名单工具在**同一次 dispatch 回合内内联执行**，结果立即折叠进 KV；未挂载的宿主保留异步 `tool.call` / `tool.result` 回退。**LLM 提供方只能是双协议适配器**（`openai-chat` 缺省 + `anthropic-messages` 回退），且严格只从 git-ignored 的 `.env` 键（`YGG_LLM_*`）配置。

**依据**：每次工具调用走一次异步往返意味着「探测 → 决策」跨回合，而回合间状态可能已变（另一泳道推进了同一个需求）——内联把「读到的就是判断依据」变回恒真。双协议适配器是因为两种 messages 形状的差异集中在适配层而非业务层，反过来（业务层按提供商分岔）会把提供商细节泄进状态机。凭据只走 `.env`：环境的凭据一旦进入 payload / 回执 / 通知就无法回收。

**判据**：① 阻断——把凭据**值**写进 payload / 回执 / 通知 / 文档，或在业务层按提供商分岔；② 症状——回执里出现密钥形态字符串，或同一工具结果在不同宿主形态下产生不同 KV；③ 确认——Mod 套件（`cargo run -q --example selftest -- --mod .ygg/mods/agent-seed`）的工具桥与 declaration-parity 用例，加 `grep -rn 'YGG_LLM' <检出>`（应只在 `.env` 样例与文档键名处）。

维护责任 = 修改工具声明、配置、线协议或反应器的执行者；触发 = 任一相关 commit；检查 = declaration-parity、payload/allowlist、receipt/replay 与 subsystem 语义用例。**缺一层同步，或把 Layer 2 当安全边界，均不得验收。**
