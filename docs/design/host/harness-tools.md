# Yggdrasill · Harness Tools 检测机制（`.pi/tools`）
> **id** host/harness-tools · **模块** ② egg CLI 与 host · **触发** 改或调用 .pi/tools/* 与结算机证参数之前
> **读不到代价** silent-drift（机证参数错 → 结算结论静默漂移）

> **定位**：`.pi/tools/` 只读检测脚本的契约、注册面、三层调用方参数来源与「无 egg 环境的参数约定」。
> **契约属性**：只读检测脚本 + 三层参数来源；机证与人证同级不越权（§3.2）。
> **上游**：[docs/README.md](../../README.md) · **相关**：[tool-bridge.md](../mod/tool-bridge.md) · [lua-api.md](../kernel/lua-api.md)

## 1. 契约：纯命令行接口（argv/env）

`.pi/tools/<name>.sh` 是自足 POSIX sh 命令行程序。三禁：

- **不读 config**：不知道 `settings.tools.*` 的存在；
- **不读局 KV**：不碰 `run.sqlite` / `autosave.json` / `events.jsonl`；
  **唯一例外（`docs-consistency.sh --attest`）**：验签必须读事件日志，故该模式要求调用方显式给
  `--run <run-dir>`（缺 = exit 2），脚本**不猜「最新局」、不读 config、不依赖 egg**，并以
  `sqlite3 'file:…?immutable=1'` 只读打开（不建 WAL、不写盘）。即例外只是「允许读调用方点名的那份事件日志」，
  不引入任何隐式局状态耦合；默认模式（索引闭包）行为一字不变。
- **不依赖 egg**：`sh <script> <args>` 即可运行，无 egg 环境的 agent 直接调用。

**输入面** = argv（具名旗标，主）+ 进程环境（位置性缺省，次）。

**输出面存在两个亚家族（r8 纠偏：此前本文写「全脚本统一 k=v」，与事实不符）**：

| 亚家族 | 成员 | 缺省输出 | 切换旗标 |
| --- | --- | --- | --- |
| harness 四件 (§2) + `scripts/` | `todo-check` / `sessions-count` / `manifest-verify` / `docs-consistency` / `docs-ver` | 单行 `k=v;k=v` | `--json` 出同信息集单行 JSON |
| **git 亚家族 (§2.2)** | `git-alignment-check` / `git-conflict-check` / `git-integrate` / `git-release` / `git-delivery-check` | 单行 JSON **三段式** `ok` / `data` / `error` | `--text` 出人读单行 |

为什么 git 亚家族不并到 k=v：`data` 是嵌套结构（`checks.<name>.<field>`、`failed[]`、`waived[]`），k=v 表达不了层级；且这五件在 k=v 口径成文**之前**就已经是 JSON 缺省。制造第三种方言的代价大于统一个已既成的方言。`--json` 在 git 亚家族里是**兼容性空操作**（JSON 已是缺省）。

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
| `sessions-count.sh` | `--parent <id>`\* `[--sessions-root <dir>]` `[--trash-root <dir>]` `[--json]` | `total=…;matching=…;trash=…;ok=…`（matching=0 → ok=1；结算不变式；matching 不含父会话自身） |
| `manifest-verify.sh` | `[--repo <dir>]`（缺省 git toplevel）`[--mod <dir>]`（缺省 `<repo>/.ygg/mods/agent-seed`）`[--json]` | `files=…;drift=…;first=…;ok=…`（drift=0 → ok=1） |
| `docs-consistency.sh` | `[--repo <dir>]` `[--index <file>]`（缺省 `<repo>/docs/README.md`）`[--json]`；`--attest --run <dir>`\* `[--max-history <n>]` | `links=…;broken=…;first=…;ok=…`（broken=0 → ok=1）；`--attest` → `claims=…;forged=…;stale=…;first=…;ok=…`（claims>0 且 forged=stale=0 → ok=1） |

### 2.1 仓库级文档工具（`scripts/`，不在 mod 工具面内）

`scripts/docs-ver.sh` 是 docs/context 召回树的身体检查，**不注册为 mod 工具**（pack.json 不声明，避免改 mod）：

| 脚本 | argv（必填 `*`） | 输出（exit 0，k=v 缺省） |
| --- | --- | --- |
| `scripts/docs-ver.sh` | `[--check]` `[--links]` `[--json]` `[--root <dir>]` | `files=…;changed=…;missing_header=…;broken_links=…;first=…;ok=…`（全 0 → ok=1） |

- **ver 定义**：固定头 `> **ver** <hex6>` 的 token 换成定宽占位 `000000`（字节数不变）、其余字节原样后 `git hash-object` 取前 6 位——直接哈希文件本体是自引用循环。规则与退场条件见 [../../wiki-governance.md](../../wiki-governance.md)「ver 定义与旧路径兼容层两分法」。
- 不带 `--check` = 就地刷新（幂等）；`--check` = 只报告不写，并顺带跑树内链接闭包与 `scenes.json` 一致性（表内 id 必须解析到文件且与文件头 `id` 逐字相等）。
- 软链一律跳过（兼容层不是权威内容，不参与 ver 账）。

`todo-check` 是**状态重放机**：按转录内 `create`（建清单；`tasks[]` 项缺 id 时由递增计数器分配——复刻宿主口径：会话内单调递增、`clear` 重置、`delete` 不回收号段）→ `update`（按 id 改状态）→ `delete` → `clear` 全序列重放，算最终 `total`/`remaining`（`pending`/`in_progress`/`open`/`blocked` 计未完成）。`sessions-count` 用 `find -print0 | xargs -0 grep -lF` 做布尔 contains(parent) 匹配，并排除父会话自身的归档（`<parent>.jsonl` / `*_<parent>.jsonl`）——主档必然含自己的 id，不排除则 `matching=0` 不可达；`trash` 递归计入每个 run 子目录，`total`/`matching` 只看平铺的库（macOS BSD grep 兼容，内容零回显）。

### 2.2 git 亚家族五脚本 argv 规格（缺省 JSON 三段式，`--text` 人读）

风格约定：`--repo <dir>` 缺省 = 调用者 cwd 的 git toplevel；`--text` / `--json`（兼容空操作）；`--help`。
全部 **只读**（不改 refs / index / worktree）；`git-integrate` / `git-release` 的例外是缺省 dry-run、只有显式 `--push` 才写（写入面，不是检查器）。

| 脚本 | argv（必填 `*`） | `data` 关键字段 / 结论键 |
| --- | --- | --- |
| `git-alignment-check.sh` | `[--repo] [--develop] [--master] [--origin]`；三者 argv > `$YGG_*` > `.env.local` > `.env` | `branches.{develop,master}.{name,local,remote,ahead,behind,status}`；`ok` = 两者皆 `aligned` |
| `git-conflict-check.sh` | `[--repo] [--develop] [--origin]` | rebase/merge 冲突预检结果（只读） |
| `git-integrate.sh` | `--feature <branch>`\* `[--develop] [--origin] [--push\|--dry-run]` | `steps[]`（`name`/`status`/`command`）、`merge_sha`、`pushed`、`reset_develop`、`partial`、`failed_step` |
| `git-release.sh` | `[--develop] [--master] [--origin] [--push\|--dry-run]` | 同上（develop → master `--no-ff` + reset develop） |
| `git-delivery-check.sh` | `[--repo] [--upstream <ref>] [--max-paths <n>] [--no-fmt] [--no-stash] [--fmt-cmd <cmd>]` | `checks.{dirty,unpushed,stash,conflict_debris,fmt}`（每项都有 `count`）；`failed[]`、`waived[]`、`truncated`；**`ok` = 五项 count 全 0** |

**`git-delivery-check.sh` 的五项口径（r8 / E2）**：

| 检查 | 命令 | 边界（为什么这样定） |
| --- | --- | --- |
| `dirty` | `git status --porcelain=v1 -uall` | `-uall` 是必须的：缺省 `-unormal` 把整个未跟踪目录折叠成一行，回执数字会骗人。拆 `tracked` / `untracked` / `unmerged` |
| `unpushed` | `git rev-list --count <upstream>..HEAD` | **无 upstream 记 `-1` = 失败**：`0` = 比过了且一致，`-1` = 没法比，两者必须可分辨（「本地干净、远端没有」的半交付只能靠这个区分）；detached HEAD 同记 `-1` 但给不同 hint |
| `stash` | `git stash list --format='%gd %s'` | stash 栈**仓库级共享**（泳道 worktree 池与主检出共用），所以 `--no-stash` 豁免必须进 `waived` 并写明理由 |
| `conflict_debris` | `git ls-files -z --cached --others --exclude-standard` 后按 basename 匹配 `*~*` / `*.orig` / `*.rej` | 用 `*~*`（**含**波浪号）而非 `*~`：真实残留形如 `copy-contract-core.md~develop`，波浪号在中间；`--exclude-standard` 让 `target/` 下的 `.orig` 不成噪音 |
| `fmt` | 有 `Cargo.toml` 时 `cargo fmt --check`（`--fmt-cmd` / `$YGG_FMT_CMD` 可覆盖） | 无 `Cargo.toml` → `n/a`（count 0，**不计**）；`--no-fmt` → `skipped`；命令不可执行 → `error` + count 1（fail-closed）。**语言无关是刻意的**：git 亚家族其余四件只认 git，把 cargo 焊死会让脚本离开本仓就废；但「不存在即静默通过」会造假绿，所以 `n/a` / `skipped` 在输出里显式可见 |

exit 码与 §1 同表；**「检测发现问题」永远 exit 0 + `ok=false`**（与 `git-alignment-check.sh` 同口径）。挂载点：交付自检 = [rituals/dispatch.md](../../context/rituals/dispatch.md) §7；验收证据 = [req-acts/integration.md](../../context/req-acts/integration.md) §7 表格首行。

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

**机证与人证同级不越权**（[state-machine.md](../kernel/state-machine.md) §2.9.2）：`settlement_problem()` 保持纯 KV 断言、零改动；机证只是「脚本跑过 + 输出与人证一致」的证据，不高于人证。

### 3.3 未注册提醒落点

1. `coordinate.submit` 进 settle 相位：未注册 → `[TOOLS UNREGISTERED]` 一次性提示；
2. 调用点回退：`[TOOLS ROOT MISSING]` / `[TOOLS SCRIPT MISSING]`（含解析细节）；
3. 参数缺：`[SESSIONS PARENT MISSING]`（sessions parent 未注册）、`[TODO SESSION MISSING]`（session 未注册）。

全部非门禁、不阻塞、纯 KV 判据。

## 4. 无 egg 环境的参数约定

无 egg 环境的 agent / 人按本节直接拼命令行。**注册值双写纪律**：`tools.root` 等值既写局 KV（config.set，机读）也写本节约定（人/agent 读），变更时两边同步（结算 docs 维护需求核对，见 [../../wiki-governance.md](../../wiki-governance.md)「批结算自动维护需求」）。

### 4.1 参数总表

| 值 | 写在脚本旗标 | 缺省推导 |
| --- | --- | --- |
| 脚本根 | （位置参数，非旗标） | checkout 根的 `.pi/tools/` |
| 主会话 id | `--session` / `--parent` | `$YGG_SESSION_ID` → `$PI_SESSION_ID` |
| 会话库根 | `--sessions-root` | `$YGG_SESSIONS_ROOT` → `$HOME/.pi/agent/sessions/--<cwd-slug>--` |
| trash 根 | `--trash-root` | `$YGG_SESSIONS_TRASH` → `$HOME/.pi/agent/sessions-trash` |
| 仓库根 | `--repo` | 脚本所在 git toplevel |

`cwd-slug` = cwd 去前导 `/` 后把 `/` 换成 `-`（与宿主 `cwd_slug` 同口径，见 `crates/ygg-sandbox/examples/egg/`）。**注意**：会话库是**主工作区**的库，泳道 worktree 的 cwd-slug 不同——跨泳道调用必须显式 `--sessions-root` 指向主工作区库。

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

# docs/context ver 验真 + 链接闭包（changed=missing_header=broken_links=0 → ok=1）
bash scripts/docs-ver.sh --check

# 推进签名验真（claims>0 且无 forged/stale → ok=1；--run 必填，脚本不猜最新局）
bash .pi/tools/docs-consistency.sh --repo "$(git rev-parse --show-toplevel)" \
  --attest --run "$(git rev-parse --show-toplevel)/.ygg/runs/<run-id>"

# 任一脚本加 --json 得单行 JSON（程序化消费）
sh .pi/tools/sessions-count.sh --parent "$PI_SESSION_ID" --sessions-root <dir> --json
```

### 4.3 `.env` 可选缺省（非机密）

`YGG_TOOLS_ROOT` / `YGG_SESSION_ID` / `YGG_SESSIONS_ROOT` / `YGG_SESSIONS_TRASH` 为**非机密**环境变量，可作脚本缺省覆盖（脚本读进程环境，与 egg 的 `$ENV{}` 无关）。凭证类值永不进此面。

## 5. 跨 harness 契约稳定性

冻结面 = 脚本名 + argv 旗标表 + 输出方言（k=v / `--json`）+ exit 码表 + 注册键名 + 本文参数约定。pi 特有口径（会话库路径、jsonl 结构）全封在脚本内；其它 harness 以同名脚本 + 同旗标表满足契约（如 `.claude/tools/sessions-count.sh` 改缺省推导、旗标不变）。演进：旗标只增不改名、输出键只增不改名。
