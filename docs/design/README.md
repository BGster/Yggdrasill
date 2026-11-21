# Yggdrasill · Design 契约树（docs/design）
> **定位**：`docs/design/` 是**「改之前必须遵守的契约」**——判据是**有代码消费者**（playbook JSON 的 `basis`/`source`、`src/**` 注释、crates 测试与注释按路径引用它）。行为叙事与产品定位不在此树，见 [../context/README.md](../context/README.md)。
> **replaces**：r6 前的 18 文件平铺形态（`docs/design/<topic>.md`）。平铺路径已退役，**不留软链**；历史与理由在 git。
> **判据**：[../wiki-governance.md](../wiki-governance.md)「design / context 二分判据」——本文件只是它的导航投影，不复制正文。
> **上游**：[../README.md](../README.md)（顶层索引） · **机检**：`.ygg/mods/agent-seed/tests/surface.lua` 的 docs/design 树索引用例（目录/文件计数、索引双向相等、id 双射、指针锚存在性）与 `tests/modpkg.lua` 的 `AGENTS.md §3` 路由用例（用例名以测试文件为准，本文不维护叙述名）

## 目录树

```text
docs/design/
├── README.md          # 本文件：触碰面 → 文件（索引完备性由机检断言）
├── kernel/            # ① ygg 内核（crates/ygg-core、crates/ygg-sandbox 本体）
│   ├── state-machine.md   state-surface.md   events.md   sim-kernel.md
│   └── wire-protocol.md   lua-api.md         compaction.md
├── host/              # ② egg CLI 与 host
│   ├── cli-daemon.md  serve.md
│   └── harness-tools.md  lanes.md
├── mod/               # ③ agent-seed mod
│   ├── mod-package.md  prompt-surface.md  tool-bridge.md
│   ├── action-faces.md  identity.md  testing-strategy.md  superagent-game.md
│   └── shapez-game.md
└── packages/          # ⑤ packages 通用能力层（9）：README + entity / codec / quest / dialogue / role-skill / procgen / assetgen / textframe
```

## 我碰哪个面 → 读哪个文件

| 触碰面 | 文件 | 触发（召回时点） | 读不到代价 |
| --- | --- | --- | --- |
| ① 内核 · 状态机语义 / 微状态判定 | [kernel/state-machine.md](kernel/state-machine.md) | 改状态机语义 / 增删事件 / 改微状态判定之前 | silent-drift |
| ① 内核 · 事件全集口径 | [kernel/events.md](kernel/events.md) | 增删事件 / 改事件分类口径之前 | hard-refusal |
| ① 内核 · KV 键契约与拒绝面 | [kernel/state-surface.md](kernel/state-surface.md) | 写新 KV 键 / 改拒绝面之前 | silent-drift |
| ① 内核 · payload 文法与尾段链 | [kernel/wire-protocol.md](kernel/wire-protocol.md) | 改 payload 形状 / 改 exec-actor 尾段链之前 | hard-refusal |
| ① 内核 · `ygg.*` 宿主 API | [kernel/lua-api.md](kernel/lua-api.md) | 新增或修改 `ygg.*` API 之前 | hard-refusal |
| ① 内核 · 压缩与检查点交接 | [kernel/compaction.md](kernel/compaction.md) | 改压缩水位 / 检查点判定 / 交接产物之前 | silent-drift |
| ① 内核 · 原生模拟内核（Tier 1） | [kernel/sim-kernel.md](kernel/sim-kernel.md) | 改 tick / ECS 状态 / 快照回放 / 确定性策略 / 热核面下沉之前 | hard-refusal |
| ② host · CLI 与 daemon 线协议 | [host/cli-daemon.md](host/cli-daemon.md) | 改命令行 / daemon 协议 / 存储与版本兼容面之前 | hard-refusal |
| ② host · serve 观察面 | [host/serve.md](host/serve.md) | 触碰 serve 任何一行之前 | irreversible |
| ② host · `.pi/tools` 检测脚本 | [host/harness-tools.md](host/harness-tools.md) | 改或调用 `.pi/tools/*` 与结算机证参数之前 | silent-drift |
| ② host · 泳道池与进度投影 | [host/lanes.md](host/lanes.md) | 改泳道池 / worktree 停放 / 进度投影之前 | silent-drift |
| ③ mod · 包契约与多文件纯度 | [mod/mod-package.md](mod/mod-package.md) | 改 `pack.json` / `manifest` / 多文件纯度契约之前 | hard-refusal |
| ③ mod · 提示词面（beats） | [mod/prompt-surface.md](mod/prompt-surface.md) | 改文案键面或键账之前 | silent-drift |
| ③ mod · 同步工具桥 | [mod/tool-bridge.md](mod/tool-bridge.md) | 改工具桥或凭据解析之前 | irreversible |
| ④ mod · 测试策略（纯 Lua 自检） | [mod/testing-strategy.md](mod/testing-strategy.md) | 改 `tests/**` 或 `selftest.rs` 之前 | silent-gap |
| ③ mod · 三层动作面 | [mod/action-faces.md](mod/action-faces.md) | 新增或移动一个动作面之前 | silent-drift |
| ③ mod · superagent 游戏（P4） | [mod/superagent-game.md](mod/superagent-game.md) | 改 `packages/mods/superagent/**` 任一行之前 | silent-drift |
| ③ mod · shapez 游戏（S26-1） | [mod/shapez-game.md](mod/shapez-game.md) | 改 `packages/mods/shapez/**` 任一行之前 | silent-drift |
| ③ mod · 身份与权限 | [mod/identity.md](mod/identity.md) | 改执行者台账 / 身份判定 / profile 之前 | silent-drift |

### ⑤ packages · 通用能力层（可复用的引擎能力，契约先于实现）

| 触碰面 | 文件 | 触发（召回时点） | 读不到代价 |
| --- | --- | --- | --- |
| ⑤ packages · 索引与命名规则 | [packages/README.md](packages/README.md) | 新增或修改任何 package 或其契约之前 | hard-refusal |
| ⑤ packages · 实体与属性袋 | [packages/entity.md](packages/entity.md) | 改实体 / 属性袋形状、索引或生命周期事件之前 | silent-drift |
| ⑤ packages · 编解码与存档往返 | [packages/codec.md](packages/codec.md) | 改 bundle 形状、键序规则或导入冲突策略之前 | irreversible |
| ⑤ packages · 任务与委托 | [packages/quest.md](packages/quest.md) | 改任务模板 / 实例形状或步骤推进语义之前 | silent-drift |
| ⑤ packages · 对话事件总线 | [packages/dialogue.md](packages/dialogue.md) | 改 handler 注册面、回退链或 agent 通道之前 | irreversible |
| ⑤ packages · 角色与技能 | [packages/role-skill.md](packages/role-skill.md) | 改角色授予 / 技能引用 / 属性合成之前 | silent-drift |
| ⑤ packages · 程序化生成 | [packages/procgen.md](packages/procgen.md) | 改 PRNG 算法、流派生规则或生成器声明形状之前 | irreversible |
| ⑤ packages · 素材生成 | [packages/assetgen.md](packages/assetgen.md) | 改素材字节形状、噪声与波形参数、容器头之前 | irreversible |
| ⑤ packages · 帧文本 | [packages/textframe.md](packages/textframe.md) | 改帧缓冲形状、分层语义或行装配口径之前 | irreversible |

## 不在本树的（行为面）

`run-story` / `requirement-story`（运行级与需求级剧情，即「一整局/一条需求怎么跑」）与 `system-boundaries`（三层架构的产品定位）**没有代码消费者**，按 [../wiki-governance.md](../wiki-governance.md) 的判据归 `docs/context/`：[../context/story/run-story.md](../context/story/run-story.md)、[../context/story/requirement-story.md](../context/story/requirement-story.md)、[../context/cold-start/system-boundaries.md](../context/cold-start/system-boundaries.md)。旧 `§N` 引用经 [../DESIGN.md](../DESIGN.md) 的映射表解析，跨树也持续有效。

## 维护闭环

- **每个文件头两行是机读契约**：`id`（= 相对 `docs/design/` 的路径去 `.md`）与 `读不到代价`（四值域 `irreversible` / `hard-refusal` / `silent-drift` / `none`）。`id` 与真实路径的双射由 surface.lua 的树索引用例断言。
- **本文件是索引**：索引链接集合与 `docs/design/**/*.md` 实文件集合**双向相等**由机检断言；新增/删除文件必须同批改本文件，否则用例转红。
- **不留兼容层**：`docs/design/*.md` 下只允许本文件——出现任何平铺文件或软链即被 surface.lua 的树索引用例判红（结构性禁止留壳）。
- **指针**：`.ygg/mods/**` 与 `crates/**` 里的 `docs/design/<module>/<topic>.md` 由 surface.lua 的指针锚用例逐条核存在性（支持子目录、拒绝 `..`）；`AGENTS.md §3` 的路由由 modpkg.lua 的路由用例核。
