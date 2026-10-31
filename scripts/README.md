# scripts/

仓库自带的**开发/运维**辅助脚本（非内核、非 Mod、不入装载管线）。

| 脚本 | 作用 |
| --- | --- |
| `build-wasm.sh` | 构建 wasm 目标产物（WASI SDK 探测器；`crates/ygg-wasm/*.mjs` 冒烟与确定性校验的构建前置） |
| `decide-choice.sh` | `--drive cmd:` 的**最小可用决策命令示例/兜底**（见下） |
| `sync-seed-mod.sh` | 同步 seed 模板与项目实例的**骨架文件**（`mod.lua` + `schemas/`）并重算两侧 manifest（见下） |

---

## `decide-choice.sh` —— 决策命令示例，不是智能决策

宿主（`crates/ygg-sandbox/examples/agent.rs`）在 `--drive cmd:<路径>` 下，会把**完整当前回执**
写进 run 目录的 `decide-input.json`，再以该路径为唯一参数调用决策命令；命令必须在 stdout
输出**恰好一行**：

- `N`（`1..=pendingChoice.options` 的序号），或
- `HUMAN[:理由]`（安全暂停：宿主不推进状态，退出 `0`）。

`scripts/decide-choice.sh`（POSIX sh + python3，std-only，无第三方包、无绝对路径假设）实现的策略刻意保持简单：

1. 默认输出 `1`（菜单首项）；
2. 以下情形输出 `HUMAN:<理由>` 而不是替你做判断：
   - 回执 `meta.summary.conflicts` 非空（有未解决冲突——`keep_both` / `keep_first` / `revise`
     的取舍是架构决策）；
   - 候选里出现验收 / 门禁 / 冲突解决类动作（`proposal.accept`、`proposal.reject`、
     `gate.approve`、`gate.reject`、`requirement.resolve`）——让脚本代选等于伪造验收结论。

```bash
# 示例：把推进交给兜底脚本
.ygg/bin/ygg-agent --mod .ygg/mods/agent-seed --run-id my-task --runs-root .ygg/runs \
  --drive cmd:scripts/decide-choice.sh
```

**这是最小可用示例，真实决策应由主 Agent 或外部编排提供**（读同一份 `decide-input.json`，
按现场证据给出序号或 `HUMAN:<理由>`）。把本脚本当策略用会带来两个已知后果：它对
`verify.submit` 直接选首项（等于无条件上报全绿），且它对任何验收动作一律停等。

协议细节（超时 5 秒、非零退出 / 多行输出 / 越界序号一律 fail closed、`YGG_DECIDE_CMD` 覆盖路径）
见仓库根 `AGENTS.md` §3。

---

## `sync-seed-mod.sh` —— 骨架面同步（实例面不碰）

仓库里有两份同构的 Mod，**不是同一份东西**：

| 位置 | 角色 |
| --- | --- |
| `packages/mods/agent-seed/` | 对外分发的**种子模板**（`data/playbook.json` 只装通用纪律） |
| `.ygg/mods/agent-seed/` | **本项目实例**（`data/` 装本仓库自己的规则与配置） |

分工：**骨架面**（`mod.lua` + `schemas/`）逐字节一致；**实例面**（`data/`）各自维护。
本脚本只搬骨架，并重算**两侧** `manifest.json`（内容寻址：少算一侧就会以哈希不符拒绝装载）。

```bash
scripts/sync-seed-mod.sh                 # 默认 seed → project
scripts/sync-seed-mod.sh --from project  # 反向：项目侧改的机制回流到模板
scripts/sync-seed-mod.sh --no-pack       # 只同步文件，不重算 manifest
```

- **幂等**：内容已一致时不写文件，只打印「骨架已一致」，重复跑不产生新 diff。
- **镜像** `schemas/`：源侧删掉的 schema 文件会在目标侧删除（仍只限 `schemas/` 内）。
- **不碰** `data/` 与 `pack.json`：前者是各项目实例数据，后者是各项目自己的声明。
- CI 的骨架守卫（`.github/workflows/check.yml` 第一步）会用 `diff` 拦下只改一侧的提交，
  失败文案直接指向本脚本。
