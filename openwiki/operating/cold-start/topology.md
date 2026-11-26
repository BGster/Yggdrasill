---
type: "Reference"
title: "Monorepo 拓扑与依赖白名单 — 目录形状与五条依赖不变量"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-8339725b1afcf7133cd86688
    resource: repo://.ygg/mods/agent-seed/pack.json
  - id: openwiki-source-00c14b6c16d892b3a7043859
    resource: repo://crates/ygg-core/Cargo.toml
  - id: openwiki-source-ff21c93b1a4de37f7ebb38e4
    resource: repo://crates/ygg-lint/Cargo.toml
  - id: openwiki-source-4c2ab33dcfd357289041ac6f
    resource: repo://crates/ygg-sandbox/Cargo.toml
  - id: openwiki-source-a4a7569ab993da998f5988fd
    resource: repo://crates/ygg-sandbox/examples/egg/main.rs
  - id: openwiki-source-68ffb3bb5cf7f0b8193a901f
    resource: repo://crates/ygg-sim/Cargo.toml
  - id: openwiki-source-23147ebf2408129337e69a1d
    resource: repo://packages/schema/schemas/manifest.json
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# Monorepo 拓扑与依赖白名单 — 目录形状与五条依赖不变量

> **id** cold-start/topology · **幕** cold-start · **触发** 冷启动 / 改仓库结构前
> **读不到代价** irreversible（依赖白名单误改）
> **replaces**：`docs/context/cold-start/topology.md`（搬运基线 `3a2536f`，原 CONTEXT §4 L97–133）
> **上游**：[overview.md](../../overview.md) · **相关**：[principles.md](principles.md) · [../../contracts/mod/mod-package.md](../../contracts/mod/mod-package.md)

## 1. 目录树（当前真实形状）

```text
Yggdrasill/
├── crates/
│   ├── ygg-core/       # 纯确定性内核：实体表、事件总线、迁移驱动（零内部依赖；仅 serde/serde_json/thiserror）
│   ├── ygg-modules/    # 纯多文件 Lua 模块路径解析与副作用禁用清单（零依赖叶 crate）
│   ├── ygg-schema/     # Manifest SHA-256 内容寻址与 JSON Schema 校验（仅依赖 ygg-core）
│   ├── ygg-store/      # 运行持久化端口：SQLite 默认（run.sqlite + VFS）与遗留 JSON 后端；observe/ = serve 只读面
│   ├── ygg-sandbox/    # 确定性 Lua 5.4 VM（mlua）、宿主 API 白名单、ygg stdlib、ModBundle 加载器、宿主 CLI（examples/egg/）
│   ├── ygg-lint/       # 静态 AST 检查器（full_moon）：YGG-R1/R2/R3、YGG-MOD-EFFECT、YGG-PARSE
│   ├── ygg-sim/        # Tier 1 原生模拟内核：tick 循环、列式 ECS、确定性快照/回放/state hash
│   └── ygg-wasm/       # 浏览器 WebAssembly 绑定（wasm-bindgen 薄壳）
├── packages/
│   ├── schema/         # JSON Schema 单一事实来源（manifest / agent-config / scene-texts / scene-texts-fragment）
│   ├── libs/           # 通用能力层：assetgen codec dialogue entity fixture procgen quest role-skill textframe
│   ├── mods/
│   │   ├── agent-seed -> ../../.ygg/mods/agent-seed   # 规范工作流 Mod 的符号链接
│   │   ├── pkg-selftest/   # 八包通用能力检查夹具（199 例）
│   │   ├── shapez/ superagent/ test-adventure/ tla-models/   # 游戏与工作流 Mod
│   ├── client-web/     # React + TypeScript 浏览器播放器
│   └── pipeline/       # TypeScript AI Mod 生成与校验循环
├── clients/godot/      # Godot 4 原生 gdext 客户端
├── .ygg/
│   └── mods/agent-seed/  # 规范多文件治理 Mod：mod.lua + src/** 五层 + data/{texts,config.json} + tests/
├── .pi/
│   ├── agents/         # 执行者定义（kernel-dev / pkg-author）
│   └── tools/          # 只读核验脚本（git 四助手、wiki-consistency.sh、wiki-ver.sh、manifest-verify.sh…）
├── scripts/            # install-egg.sh、sync-seed-mod.sh、gen-scene-readfirst.py、wiki-ver.sh
└── openwiki/           # 唯一知识库（overview / contracts / operating / governance）
```

## 2. 硬性依赖不变量（五条，一条都不能松）

- `ygg-core` 只能依赖 `serde`、`serde_json`、`thiserror`。任何 VM、Schema 校验器、I/O 或平台 API 不得进入 `ygg-core`。
- `ygg-modules` 是被 `ygg-sandbox` 与 `ygg-lint` 共享的零依赖叶 crate。
- `ygg-lint` 绝不依赖 `ygg-core`、`ygg-schema` 或 `ygg-sandbox`（静态 AST 检查免 `mlua` 编译）。
- `ygg-store`（及 `rusqlite`）只被原生宿主 CLI（`crates/ygg-sandbox/examples/egg/`，经 dev-dependencies）链接，确保 `ygg-wasm` 与 `ygg-lint` 的依赖树 100% 不含 SQLite 与文件系统依赖。
- `ygg-sim` 零内部依赖（不依赖任何 `ygg-*` crate），运行时依赖仅 `thiserror`；方向单一 `ygg-sandbox → ygg-sim`（宿主是注入者）。它必须能编到 `wasm32-unknown-unknown`（经 `ygg-wasm → ygg-sandbox → ygg-sim`），故 `src/**` 不得出现 `std::time` / `std::fs` / 线程 / `std::process`；`ygg-sim` 也不得出现在 `ygg-lint` 的依赖树。

## 3. 仓库纪律（原 playbook `repository` / `branches` 章）

- **`repository.vision-single-source`**（must）：决策表只活在真正支配该决策的契约页「设计依据」段；设计讨论产生的新决策必须回写那张表，而不是只活在会话里。
- **`repository.decision-conflict-is-defect`**（must-not）：实现与决策表冲突即缺陷；「已经做完了」不是让决策悄悄漂移的理由。
- **`repository.schema-single-source`**（must）：资源格式契约只在 `packages/schema/schemas/`；TS 侧（ajv）与 Rust 侧（jsonschema）读同一文件，任何一侧都不得留内联副本。
- **`repository.schema-does-not-constrain-lua`**（must-not）：Schema 约束的是资源文件格式，不是 Mod 逻辑——Mod 的唯一可执行接口是 Lua。
- **`repository.core-purity`**（must-not）：`ygg-core` 零内部依赖；允许清单只有 serde / serde_json / thiserror。
- **`repository.vm-dependency-location`**（must）：`mlua` 之类 VM 依赖只能出现在 `crates/ygg-sandbox`。
- **`repository.dependency-direction`**（must）：单向——`ygg-sandbox` → `ygg-core` + `ygg-schema`；`ygg-wasm` → `ygg-sandbox`；`ygg-lint` 无任何 `ygg-*` 内核依赖；`ygg-schema` 与 `ygg-core` 之间无代码依赖边。
- **`repository.living-docs-closed-loop`**（must）：`openwiki/` 的契约页与操作页是 `plan` 与 `development` 阶段的强制参考——每份 brief 都含它们，执行者提方案或改代码前必须查阅（正文见 [../../governance.md](../../governance.md)）。
- **`repository.mod-text-hygiene`**（must-not）：绝不在 `data/texts/**` 重新引入已退役的 mod/CLI 路径字面量；只有同行 `[deprecated-ref]` 标注才算真实历史引用。
- **`branches.no-direct-master-work`**（must-not）：绝不在 master 上直接开发；master 只接受合并。
- **`branches.feature-naming`**（must）：一切工作（含文档）都在 `feature/<topic>` 或 `docs/<topic>` 分支上。
- **`branches.no-ff-merge`**（must）：每个合并（feature → develop → master）都用 `git merge --no-ff`；每一跳都禁止 fast-forward。
- **`branches.merge-node-is-rollback-anchor`**（must）：保留合并节点，让一个迭代批次（如 `v<date>.<n>`）在历史图里可追溯、可整体回滚——这是 no-FF 的目的，不是风格偏好。
- **`branches.integration-branch-standing`**（must）：`develop` 是常驻集成分支（物理分支名，`origin/develop` 存在）：每个需求收尾把功能分支 rebase 到 `develop` 之上，再以 `--no-ff` 并入；`develop` 只在轮次收尾并入 `master`；功能分支永不直连 `master`——进入 `master` 的唯一边是轮级的 `develop` → `master`。
- **`branches.push-after-verify`**（must）：功能分支核验通过后即 push origin（协同与备份）。
- **`branches.merge-follows-play-mode`**（must-not）：轮次合并前先读 [../acts/release.md](../acts/release.md)；guide/goal 触发条件、绿灯前提与合并节奏由该幕独占，本节不是第二套合并政策。

## 4. 判据

① 阻断——把 VM / I/O 依赖塞进 `ygg-core`、让 `ygg-lint` 依赖 `ygg-sandbox`、在 `packages/mods/` 里造第二份 agent-seed 副本（而不是符号链接）、在 master 上直接提交。② 症状——`cargo tree` 里 `ygg-wasm` 出现 `rusqlite`、`ygg-lint` 出现 `mlua`、`agent-seed` 两份不同内容。③ 确认——`cargo tree -p ygg-lint` 无 `ygg-*`、`cargo tree -p ygg-wasm` 无 `rusqlite`、`readlink packages/mods/agent-seed` 指向 `../../.ygg/mods/agent-seed`。

维护责任 = 改 crate 依赖边、目录形状或分支政策的执行者；触发 = 新增 crate / package、改 `Cargo.toml` 依赖、改分支政策；检查 = 上面三条 `cargo tree` / `readlink` 探针 + 本页五条不变量与各 `Cargo.toml` 一致。
