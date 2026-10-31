# Yggdrasill · Agent Harness 强制约束与冷启动

> 本文件只写两件事：不能违反的约束，以及怎么开始 / 继续 / 暂停 / 结束。玩法细节（两层状态机、
> 动作协议、异步 inbox、进度查看、runs 产物、Mod 维护、host 契约）现场查 `guide.show harness`；
> 项目规则同法现场查（`guide.show <chapter>`）。`CLAUDE.md` 是本文件的软链。

## 一、强制约束（不可协商）

1. **必须用 CLI 代入角色**：所有推进经 `.ygg/bin/ygg-agent` 派发动作，不绕过状态机直接改仓库文件。
2. **禁止用脚本跳过剧情与节点**：不批量刷 `--event`、不手改 `autosave.json` / `events.jsonl`、
   不用 `--drive cmd` 的脚本伪装决策——决策必须来自对当前处境的真实判断。
3. **必须遵循节点提示词**：`pendingChoice` 与叙事里的约束就是当前动作面，不按记忆臆断；
   发现不合理，在局末 `review.note` 里反馈，不得中途无视。
4. **凭据只在 `.env` / `.env.local`**：Mod 资源、配置、报告只出现键名，不出现值。

## 二、冷启动

### 构建与安装

```bash
cargo build --release --example agent
mkdir -p .ygg/bin && cp target/release/examples/agent .ygg/bin/ygg-agent
```

`.ygg/bin/`（可执行）与 `.ygg/runs/`（本局产物）在 `.gitignore` 内；`.ygg/mods/` 入库。

### 开始

```bash
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id <局名> --runs-root .ygg/runs --event event.start
```

### 继续

```bash
A=".ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id <局名> --runs-root .ygg/runs"
$A --resume --drive stdin                          # 交互：人来选（缺省）
$A --resume --drive cmd:scripts/decide-choice.sh   # 挂机：外部决策命令；返回 HUMAN 即安全暂停
$A --resume --drive none                           # 只看当前处境（含 pendingChoice），不推进
.ygg/bin/ygg-agent --run-id <局名> --runs-root .ygg/runs --requirement '<标题>|<描述>'
# ↑ 异步补需求 / 提案 / 修订：只追加 inbox.jsonl，不读写存档，可与运行中的局并发
```

### 暂停

- 交互模式输入 `q`，或 stdin 结束。
- 挂机模式决策命令返回 `HUMAN[:理由]`、超时、非零退出、非法输出、越界序号——一律安全暂停，状态不推进，不会回退选 1。
- 暂停后重新执行「继续」里的命令即可，不会丢状态。

### 结束

```bash
.ygg/bin/ygg-agent --run-id <局名> --runs-root .ygg/runs --settle
```

固定结算顺序（不可跳步）：

1. 确认所有需求完成、inbox 无未消费行 → `--settle` 并核验 `settlement.json` 的 `ok`。
2. 完成必要的合流与结果汇报，确认本局没有遗留实现或验收工作。
3. **清空主会话 Todo 列表**（`todo clear`，含已完成与历史任务）后结束本局；不得把已结束局的任务带入下一局。
