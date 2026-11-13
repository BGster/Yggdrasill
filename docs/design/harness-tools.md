# Yggdrasill · Harness Tools 检测机制（`.pi/tools`）

> **定位**：`.pi/tools/` 只读检测脚本的契约、注册面、三层调用方参数来源与「无 egg 环境的参数约定」。
> **决策**：D-MA26（docs/context/decisions-agent.md）。
> **上游**：[docs/README.md](../README.md) · **相关**：[tool-bridge.md](tool-bridge.md) · [lua-api.md](lua-api.md)

## 1. 契约：纯命令行接口（argv/env）

`.pi/tools/<name>.sh` 是自足 POSIX sh 命令行程序。三禁：

- **不读 config**：不知道 `settings.tools.*` 的存在；
- **不读局 KV**：不碰 `run.sqlite` / `autosave.json` / `events.jsonl`；
- **不依赖 egg**：`sh <script> <args>` 即可运行，无 egg 环境的 agent 直接调用。

**输入面** = argv（具名旗标，主）+ 进程环境（位置性缺省，次）。**输出面** = 缺省单行 `k=v;k=v`（与现役工具同方言），`--json` 输出同信息集的单行 JSON（服务无 egg 环境的程序化消费，不进 tool.result wire）。

**exit 码表**（全脚本统一）：

| code | 含义 |
| --- | --- |
| 0 | 正常执行（结论在 `ok=1/0` 与计数键） |
| 2 | 用法错（缺必填旗标 / 旗标形状非法 / session 缺失） |
| 3 | 环境缺失（`$HOME` 缺席且未给根 / 无 python3 / 无 git） |
| 4 | 目标资源不可读（transcript / 索引 / manifest 缺失） |
| ≥5 | malfunction |

**「检测发现问题」永远 exit 0 + `ok=0`**，与 malfunction 严格区分。

**安全边界**：布尔/计数口径，文件内容零回显；不碰 `*.xlsx`/`*.csv` 等业务数据与涉密挂载；无凭据。

## 2. 首批四脚本 argv 规格

| 脚本 | argv（必填 `*`） | 输出（exit 0，k=v 缺省） |
| --- | --- | --- |
| `todo-check.sh` | `[--session <id>]`（缺省 `$YGG_SESSION_ID`→`$PI_SESSION_ID`）`[--sessions-root <dir>]` `[--json]` | `found=…;total=…;remaining=…;active=…;ok=…;approx=…`（remaining=0 → ok=1） |
| `sessions-count.sh` | `--parent <id>`\* `[--sessions-root <dir>]` `[--trash-root <dir>]` `[--json]` | `total=…;matching=…;trash=…;ok=…`（matching=0 → ok=1；结算不变式） |
| `manifest-verify.sh` | `[--repo <dir>]`（缺省 git toplevel）`[--mod <dir>]`（缺省 `<repo>/.ygg/mods/agent-seed`）`[--json]` | `files=…;drift=…;first=…;ok=…`（drift=0 → ok=1） |
| `docs-consistency.sh` | `[--repo <dir>]` `[--index <file>]`（缺省 `<repo>/docs/README.md`）`[--json]` | `links=…;broken=…;first=…;ok=…`（broken=0 → ok=1） |

`todo-check` 是**状态重放机**：按转录内 `create`（建清单）→ `update`（按 id 改状态）→ `delete` → `clear` 全序列重放，算最终 `total`/`remaining`（`pending`/`in_progress`/`open`/`blocked` 计未完成）。`sessions-count` 用 `find -print0 | xargs -0 grep -lF` 做布尔 contains(parent) 匹配（macOS BSD grep 兼容，内容零回显）。

## 3. 三层调用方参数来源

| 层 | 谁 | 参数从哪读 | 怎么拼 |
| --- | --- | --- | --- |
| **L1 egg 局内** | mod handler（review.lua） | **局 KV**（`settings.tools.*`，config.set 注册） | Lua 拼好 argv（`session=`/`parent=`/`sessions_root=`），经 `try_sync_call` 发出 |
| **L2 宿主** | egg tool 桥 | **只认 argv**（L1 拼好的），零改动 | pack.json `program:".pi/tools/<name>.sh"` 相对路径 + shebang 直接 exec；形状校验 + spawn + stdout 透传 |
| **L3 无 egg 环境** | 任何 agent / 人 | **本文 §4 参数约定**（+ `.env` 可选缺省） | 按文档直接拼 `sh .pi/tools/<name>.sh <args>` |

**argv 是唯一跨层事实**；KV / context / `.env` 只是各层的参数来源适配。

### 3.1 注册面（config.set → settings.tools.*）

| 键 | 值 | 消费点 |
| --- | --- | --- |
| `tools.root` | 脚本根目录（如 checkout 的 `.pi/tools`） | **机证启用开关**：未注册 = 结算人证回退；已注册 = 尝试机证 |
| `tools.session` | 主会话 id（= todo 的 `--session`、sessions 的 `--parent`） | 结算 cross-check 拼 argv（mod 沙箱读不到进程环境，只能来自 KV） |
| `tools.sessions_root` | 会话库路径（`--sessions-root`） | 同上 |
| `tools.<name>` | 脚本路径覆盖（`.sh` 结尾，缺省 `<name>.sh`） | 保留扩展面 |

`session` / `sessions_root` 是**保留参数键**（自由值），与脚本覆盖键（`.sh`）在 config.set 分支里分途。

### 3.2 结算 cross-check 接入点

`settlement.todo` / `settlement.sessions` 两 handler 在 apply 前同步调 harness 工具（复刻 `settlement.evolution` 的 artifact-probe 先例）：

- 机证与人证**矛盾** → `reject_payload`（零状态写入）；
- 未注册 / 参数缺 / 脚本不可执行 → **人证旧路径 + 可读提示**，绝不拒绝结算；
- 通过 → 写 `settlement.todo.source=machine:harness.todo-check` / `settlement.sessions.source=machine:harness.sessions-count`（人证写 `human`）。

**机证与人证同级不越权**（AGENTS.md §9.2c）：`settlement_problem()` 保持纯 KV 断言、零改动；机证只是「脚本跑过 + 输出与人证一致」的证据，不高于人证。

### 3.3 未注册提醒落点

1. `coordinate.submit` 进 settle 相位：未注册 → `[TOOLS UNREGISTERED]` 一次性提示；
2. 调用点回退：`[TOOLS ROOT MISSING]` / `[TOOLS SCRIPT MISSING]`（含解析细节）；
3. 参数缺：`[SESSIONS PARENT MISSING]`（sessions parent 未注册）、`[TODO SESSION MISSING]`（session 未注册）。

全部非门禁、不阻塞、纯 KV 判据。

## 4. 无 egg 环境的参数约定

无 egg 环境的 agent / 人按本节直接拼命令行。**注册值双写纪律**：`tools.root` 等值既写局 KV（config.set，机读）也写本节约定（人/agent 读），变更时两边同步（结算 docs 维护需求核对，D-MA25 通道）。

### 4.1 参数总表

| 值 | 写在脚本旗标 | 缺省推导 |
| --- | --- | --- |
| 脚本根 | （位置参数，非旗标） | checkout 根的 `.pi/tools/` |
| 主会话 id | `--session` / `--parent` | `$YGG_SESSION_ID` → `$PI_SESSION_ID` |
| 会话库根 | `--sessions-root` | `$YGG_SESSIONS_ROOT` → `$HOME/.pi/agent/sessions/--<cwd-slug>--` |
| trash 根 | `--trash-root` | `$YGG_SESSIONS_TRASH` → `$HOME/.pi/agent/sessions-trash` |
| 仓库根 | `--repo` | 脚本所在 git toplevel |

`cwd-slug` = cwd 去前导 `/` 后把 `/` 换成 `-`（与宿主 `cwd_slug` 同口径，见 `crates/ygg-sandbox/examples/egg.rs`）。**注意**：会话库是**主工作区**的库，泳道 worktree 的 cwd-slug 不同——跨泳道调用必须显式 `--sessions-root` 指向主工作区库。

### 4.2 可复制命令样例

```bash
# 子会话盘点（结算不变式 matching=0）：主工作区会话库 + 主会话 id 作 parent
sh .pi/tools/sessions-count.sh \
  --parent "$PI_SESSION_ID" \
  --sessions-root "$HOME/.pi/agent/sessions/--Users-zeki-leon-WorkHub-Yggdrasill--"

# 主会话 Todo 盘点（remaining=0 即清零）
sh .pi/tools/todo-check.sh \
  --session "$PI_SESSION_ID" \
  --sessions-root "$HOME/.pi/agent/sessions/--Users-zeki-leon-WorkHub-Yggdrasill--"

# 单源 Mod 完整性（drift=0 即一致）
sh .pi/tools/manifest-verify.sh --repo "$(git rev-parse --show-toplevel)"

# docs wiki 闭包（broken=0 即无断链）
sh .pi/tools/docs-consistency.sh --repo "$(git rev-parse --show-toplevel)"

# 任一脚本加 --json 得单行 JSON（程序化消费）
sh .pi/tools/sessions-count.sh --parent "$PI_SESSION_ID" --sessions-root <dir> --json
```

### 4.3 `.env` 可选缺省（非机密）

`YGG_TOOLS_ROOT` / `YGG_SESSION_ID` / `YGG_SESSIONS_ROOT` / `YGG_SESSIONS_TRASH` 为**非机密**环境变量，可作脚本缺省覆盖（脚本读进程环境，与 egg 的 `$ENV{}` 无关）。凭证类值永不进此面。

## 5. 跨 harness 契约稳定性

冻结面 = 脚本名 + argv 旗标表 + 输出方言（k=v / `--json`）+ exit 码表 + 注册键名 + 本文参数约定。pi 特有口径（会话库路径、jsonl 结构）全封在脚本内；其它 harness 以同名脚本 + 同旗标表满足契约（如 `.claude/tools/sessions-count.sh` 改缺省推导、旗标不变）。演进：旗标只增不改名、输出键只增不改名。
