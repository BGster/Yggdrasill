---
type: architecture-index
title: 触碰面 → 契约
description: 按「我要碰哪个面」查契约页：内核、host、mod、packages 四组共 30 份契约，索引完备性由测试断言。
tags: [contracts, touch-surface, index, architecture]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-cfe8ec9e3ed7fc4dc881025e
    resource: repo://.ygg/mods/agent-seed/mod.lua
  - id: openwiki-source-61bded3ee6e0191591000c88
    resource: repo://.ygg/mods/agent-seed/tests/modpkg.lua
  - id: openwiki-source-3066cfe018ac65c8d63a3a17
    resource: repo://.ygg/mods/agent-seed/tests/surface-tree.lua
  - id: openwiki-source-8037e2358a2c4f9b2c722a11
    resource: repo://AGENTS.md
  - id: openwiki-source-ef4b88be1d64012bd99b7527
    resource: repo://crates/ygg-core/src/lib.rs
  - id: openwiki-source-d2b71bd24583ba41d15160e6
    resource: repo://crates/ygg-sandbox/src/lib.rs
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

> **id** architecture · **触发** 要改一个子系统、先找它的契约页时
> **读不到代价** silent-drift（改错层、或去改一个已被契约页收口的实现细节）

# 触碰面 → 契约

> **定位**：本页是**「改之前必须遵守的契约」**的总入口——判据是**有代码消费者**（mod 的 `data/**` 引 `basis`/`source`、`src/**` 注释、crates 测试与注释按路径引用它）。行为叙事与产品定位不在此组，见 [../operating/story/run-story.md](../operating/story/run-story.md)。
> **replaces**：`docs/design/README.md`（旧 18 文件平铺形态在 r6 已退役，不留软链）。
> **判据**：[../governance.md](../governance.md)「契约 / 操作二分判据」——有代码消费者进 `contracts/`，无消费者的行为叙事进 `operating/`。

## 目录树

```text
openwiki/contracts/
├── kernel/              # ① ygg 内核（crates/ygg-core、crates/ygg-sim、crates/ygg-sandbox 本体）
│   ├── state-machine.md  state-surface.md  events.md  sim-kernel.md
│   └── wire-protocol.md  lua-api.md        compaction.md
├── host/                # ② egg CLI 与 host
│   ├── cli-daemon.md  serve.md  rt-face.md
│   └── harness-tools.md  lanes.md
├── mod/                 # ③ agent-seed mod
│   ├── mod-package.md  prompt-surface.md  tool-bridge.md
│   ├── action-faces.md  identity.md  testing-strategy.md
│   └── superagent-game.md  shapez-game.md  tla-models.md   # ④ 三个包内 mod 的示范面
└── packages/            # ⑤ packages 通用能力层（README + 8 个包）
```

本页（`architecture/README.md`）是这棵树的索引：**不在树内**，只维护「触碰面 → 页」的映射。

## 我碰哪个面 → 读哪一页

| 触碰面 | 契约页 | 触发（召回时点） | 读不到代价 |
| --- | --- | --- | --- |
<!-- openwiki: broken internal link [kernel/state-machine.md] file "kernel/state-machine.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ① 内核 · 状态机语义 / 微状态判定 | [kernel/state-machine.md](../contracts/kernel/state-machine.md) | 改状态机语义 / 增删事件 / 改微状态判定之前 | silent-drift |
<!-- openwiki: broken internal link [kernel/events.md] file "kernel/events.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ① 内核 · 事件全集口径 | [kernel/events.md](../contracts/kernel/events.md) | 增删事件 / 改事件分类口径之前 | hard-refusal |
<!-- openwiki: broken internal link [kernel/state-surface.md] file "kernel/state-surface.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ① 内核 · KV 键契约与拒绝面 | [kernel/state-surface.md](../contracts/kernel/state-surface.md) | 写新 KV 键 / 改拒绝面之前 | silent-drift |
<!-- openwiki: broken internal link [kernel/wire-protocol.md] file "kernel/wire-protocol.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ① 内核 · payload 文法与尾段链 | [kernel/wire-protocol.md](../contracts/kernel/wire-protocol.md) | 改 payload 形状 / 改 exec-actor 尾段链之前 | hard-refusal |
<!-- openwiki: broken internal link [kernel/lua-api.md] file "kernel/lua-api.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ① 内核 · `ygg.*` 宿主 API | [kernel/lua-api.md](../contracts/kernel/lua-api.md) | 新增或修改 `ygg.*` API 之前 | hard-refusal |
<!-- openwiki: broken internal link [kernel/compaction.md] file "kernel/compaction.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ① 内核 · 压缩与检查点交接 | [kernel/compaction.md](../contracts/kernel/compaction.md) | 改压缩水位 / 检查点判定 / 交接产物之前 | silent-drift |
<!-- openwiki: broken internal link [kernel/sim-kernel.md] file "kernel/sim-kernel.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ① 内核 · 原生模拟内核（Tier 1） | [kernel/sim-kernel.md](../contracts/kernel/sim-kernel.md) | 改 tick / ECS 状态 / 快照回放 / 确定性策略 / 热核面下沉之前 | hard-refusal |
<!-- openwiki: broken internal link [host/cli-daemon.md] file "host/cli-daemon.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ② host · CLI 与 daemon 线协议 | [host/cli-daemon.md](../contracts/host/cli-daemon.md) | 改命令行 / daemon 协议 / 存储与版本兼容面之前 | hard-refusal |
<!-- openwiki: broken internal link [host/serve.md] file "host/serve.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ② host · serve 观察面 | [host/serve.md](../contracts/host/serve.md) | 触碰 serve 任何一行之前 | irreversible |
<!-- openwiki: broken internal link [host/rt-face.md] file "host/rt-face.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ② host · rt 实时宿主面 | [host/rt-face.md](../contracts/host/rt-face.md) | 改 `ygg.rt.*` / rt 引擎 / `examples/rt` / 帧循环或录制回放语义之前 | silent-drift |
<!-- openwiki: broken internal link [host/harness-tools.md] file "host/harness-tools.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ② host · `.pi/tools` 检测脚本 | [host/harness-tools.md](../contracts/host/harness-tools.md) | 改或调用 `.pi/tools/*` 与结算机证参数之前 | silent-drift |
<!-- openwiki: broken internal link [host/lanes.md] file "host/lanes.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ② host · 泳道池与进度投影 | [host/lanes.md](../contracts/host/lanes.md) | 改泳道池 / worktree 停放 / 进度投影之前 | silent-drift |
<!-- openwiki: broken internal link [mod/mod-package.md] file "mod/mod-package.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · 包契约与多文件纯度 | [mod/mod-package.md](../contracts/mod/mod-package.md) | 改 `pack.json` / `manifest` / 多文件纯度契约之前 | hard-refusal |
<!-- openwiki: broken internal link [mod/prompt-surface.md] file "mod/prompt-surface.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · 提示词面（beats） | [mod/prompt-surface.md](../contracts/mod/prompt-surface.md) | 改文案键面或键账之前 | silent-drift |
<!-- openwiki: broken internal link [mod/tool-bridge.md] file "mod/tool-bridge.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · 同步工具桥 | [mod/tool-bridge.md](../contracts/mod/tool-bridge.md) | 改工具桥或凭据解析之前 | irreversible |
<!-- openwiki: broken internal link [mod/testing-strategy.md] file "mod/testing-strategy.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ④ mod · 测试策略（纯 Lua 自检） | [mod/testing-strategy.md](../contracts/mod/testing-strategy.md) | 改 `tests/**` 或 `selftest.rs` 之前 | silent-gap |
<!-- openwiki: broken internal link [mod/action-faces.md] file "mod/action-faces.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · 三层动作面 | [mod/action-faces.md](../contracts/mod/action-faces.md) | 新增或移动一个动作面之前 | silent-drift |
<!-- openwiki: broken internal link [mod/superagent-game.md] file "mod/superagent-game.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · superagent 游戏（P4） | [mod/superagent-game.md](../contracts/mod/superagent-game.md) | 改 `packages/mods/superagent/**` 任一行之前 | silent-drift |
<!-- openwiki: broken internal link [mod/shapez-game.md] file "mod/shapez-game.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · shapez 游戏（S26-1） | [mod/shapez-game.md](../contracts/mod/shapez-game.md) | 改 `packages/mods/shapez/**` 任一行之前 | silent-drift |
<!-- openwiki: broken internal link [mod/tla-models.md] file "mod/tla-models.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · TLA+ 模型移植（S26-4） | [mod/tla-models.md](../contracts/mod/tla-models.md) | 改 `packages/mods/tla-models/**` 任一行之前 | silent-drift |
<!-- openwiki: broken internal link [mod/identity.md] file "mod/identity.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ③ mod · 身份与权限 | [mod/identity.md](../contracts/mod/identity.md) | 改执行者台账 / 身份判定 / profile 之前 | silent-drift |

## ⑤ packages · 通用能力层（可复用的引擎能力，契约先于实现）

| 触碰面 | 契约页 | 触发（召回时点） | 读不到代价 |
| --- | --- | --- | --- |
<!-- openwiki: broken internal link [packages/README.md] file "packages/README.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 索引与命名规则 | [packages/README.md](../contracts/packages/README.md) | 新增或修改任何 package 或其契约之前 | hard-refusal |
<!-- openwiki: broken internal link [packages/entity.md] file "packages/entity.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 实体与属性袋 | [packages/entity.md](../contracts/packages/entity.md) | 改实体 / 属性袋形状、索引或生命周期事件之前 | silent-drift |
<!-- openwiki: broken internal link [packages/codec.md] file "packages/codec.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 编解码与存档往返 | [packages/codec.md](../contracts/packages/codec.md) | 改 bundle 形状、键序规则或导入冲突策略之前 | irreversible |
<!-- openwiki: broken internal link [packages/quest.md] file "packages/quest.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 任务与委托 | [packages/quest.md](../contracts/packages/quest.md) | 改任务模板 / 实例形状或步骤推进语义之前 | silent-drift |
<!-- openwiki: broken internal link [packages/dialogue.md] file "packages/dialogue.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 对话事件总线 | [packages/dialogue.md](../contracts/packages/dialogue.md) | 改 handler 注册面、回退链或 agent 通道之前 | irreversible |
<!-- openwiki: broken internal link [packages/role-skill.md] file "packages/role-skill.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 角色与技能 | [packages/role-skill.md](../contracts/packages/role-skill.md) | 改角色授予 / 技能引用 / 属性合成之前 | silent-drift |
<!-- openwiki: broken internal link [packages/procgen.md] file "packages/procgen.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 程序化生成 | [packages/procgen.md](../contracts/packages/procgen.md) | 改 PRNG 算法、流派生规则或生成器声明形状之前 | irreversible |
<!-- openwiki: broken internal link [packages/assetgen.md] file "packages/assetgen.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 素材生成 | [packages/assetgen.md](../contracts/packages/assetgen.md) | 改素材字节形状、噪声与波形参数、容器头之前 | irreversible |
<!-- openwiki: broken internal link [packages/textframe.md] file "packages/textframe.md" does not exist. Fix the href or restore the target, then delete this comment. -->
| ⑤ packages · 帧文本 | [packages/textframe.md](../contracts/packages/textframe.md) | 改帧缓冲形状、分层语义或行装配口径之前 | irreversible |

## 不在本组的（行为面）

`run-story` / `requirement-story`（运行级与需求级剧情，即「一整局/一条需求怎么跑」）与 `system-boundaries`（三层架构的产品定位）**没有代码消费者**，按判据归 `operating/`：[../operating/story/run-story.md](../operating/story/run-story.md)、[../operating/story/requirement-story.md](../operating/story/requirement-story.md)、[../operating/cold-start/system-boundaries.md](../operating/cold-start/system-boundaries.md)。旧 `§N` 引用经 [../overview.md](../overview.md) 的映射表解析，跨组也持续有效。

## 维护闭环

- **索引即契约**：本页链接集合与 `contracts/**/*.md` 实文件集合**双向相等**由 `.ygg/mods/agent-seed/tests/surface-tree.lua` 的树索引用例断言；新增/删除契约页必须同批改本页，否则用例转红。
- **一级目录固定四组**：`contracts/` 下只允许 `kernel` / `host` / `mod` / `packages` 四个子目录；出现任何平铺文件或软链即判红（结构性禁止留壳）。
- **指针**：`.ygg/mods/**` 与 `crates/**` 里的 `openwiki/contracts/<module>/<topic>.md` 由同一用例逐条核存在性（支持子目录、拒绝 `..`）；`AGENTS.md` 的路由表由 `tests/modpkg.lua` 核。
- **每页头部是机读契约**：`id`（= 相对 `contracts/` 的路径去 `.md`）与 `读不到代价`（四值域 `irreversible` / `hard-refusal` / `silent-drift` / `none`）；版本号已从页面移到 `openwiki/ver.json`，见 [../governance.md](../governance.md)。
