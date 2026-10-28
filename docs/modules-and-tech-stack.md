# 模块划分与技术栈（Modules & Tech Stack）

> 状态：v1 终态（2026-10-29 终态整理）
> 上游文档：`docs/vision.md`、`docs/architecture-principles.md`

## 1. Monorepo 布局

```text
yggdrasill/
├── crates/                      # Rust workspace
│   ├── ygg-core/                # 规则内核：实体表/事件/转换驱动（纯逻辑，零 IO）
│   ├── ygg-sandbox/             # 确定性 Lua 沙箱：mlua + host API 白名单 + ygg stdlib 嵌入
│   ├── ygg-schema/              # 资源数据模型 + 校验（manifest + sha256 + jsonschema）
│   ├── ygg-lint/                # Lua 静态检查（full_moon AST，R1/R2/R3/PARSE；lib + CLI）
│   └── ygg-wasm/                # wasm-bindgen 绑定层（薄壳，内核的对外契约）
├── packages/                    # pnpm workspace
│   ├── schema/                  # JSON Schema 文件 —— 资源格式契约（单一事实来源，D15）
│   ├── client-web/              # 表现层（React + TS）：渲染内核状态、收集输入、音频
│   └── pipeline/                # M1 内容管线（TS）：LLM 编排 → Schema 校验 → 重生成回路
├── services/                    # M2+ 平台服务（Rust + Axum）：分享/导入导出/后续社区化
└── tools/                       # CLI（Rust + clap）：Mod 校验、打包、示例游戏构建
```

### 模块间依赖约束（硬性）

- `ygg-core` 不依赖任何内部模块；`ygg-schema` 单向依赖 `ygg-core`；`ygg-sandbox` 单向依赖 `ygg-core` 与 `ygg-schema`（资源校验）；`ygg-wasm` 依赖 `ygg-core` 与 `ygg-sandbox`（driver 接线，D12）
- **JSON Schema 文件（`packages/schema/`）是资源格式契约的单一事实来源**（D15）：TS 侧用 ajv 校验，Rust 侧用 jsonschema crate 校验，两边读同一份文件，杜绝双实现漂移；Mod 的可执行接口为 Lua（D11），Schema 不约束 Mod 逻辑
- 表现层只通过 `ygg-wasm` 接口与内核对话——该接口是未来 Godot / CS 服务端适配的同构改动面
- `pipeline` 与内核之间只通过 Schema 契约耦合，不直接调用内核代码（验证工具走子进程：lint CLI / playtest harness）

## 2. 内核语言：Rust 的核心论证（D7）

**Rust 在本项目的最大价值不是性能，而是「单一内核实现，多端复用」：**

| 运行环境 | 内核形态 | 状态 |
| --- | --- | --- |
| 浏览器（Web-first） | `ygg-wasm`（wasm-bindgen 两步构建链，含 Lua VM 实机） | ✅ 已交付（GitHub Pages 线上） |
| Godot（M2.5） | gdext 原生链接（Rust 动态库，不走 wasm） | 📌 spike GO |
| 权威服务器（多人/微信 CS） | native 编译（直接或进 Axum 服务） | 📌 待立项 |

纯函数式内核 `(state, event) → state'` 用 Rust 实现的收益：

- 确定性保障（无 GC 抖动、严格类型、所有权模型天然约束不可变语义）
- M4 多人时客户端/服务端**跑同一份确定性逻辑**（lockstep 回放的前提）
- 性能是副产品，不是选型主因

### 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| 内核语义调整期 Rust 迭代摩擦 | 内核语义先在纸面定稿（Schema 设计先行），再动手写实现 |
| wasm 调试体验弱于原生 JS | `ygg-core` 保持纯逻辑 + 完整单测，native 环境测试（Lua VM 面由 `ygg-sandbox` 承载确定性探针与 golden-vector）；wasm 只是薄壳 |
| 微信端兼容风险 | 已裁决 CS 架构绕开（服务端 native，见 vision M0.5-CS） |

## 3. 各模块技术栈总表（D7–D10）

| 模块 | 技术栈 | 决策 | 理由 |
| --- | --- | --- | --- |
| 规则内核 `ygg-core` | Rust（serde、thiserror、serde_json——零依赖铁律白名单） | D7 | 多端复用 + 确定性，见 §2 |
| Lua 沙箱 `ygg-sandbox` | mlua（git master pin，Lua 5.4 + vendored） | D12/D16 | 确定性沙箱 + host API 白名单 + ygg stdlib 嵌入 + 燃料/种子化 RNG |
| 资源校验 `ygg-schema` | Rust + serde + jsonschema（default-features=false）+ sha2 | D12/D15 | 资源 manifest + sha256 内容寻址 + Schema 校验 |
| wasm 绑定 `ygg-wasm` | wasm-bindgen（`scripts/build-wasm.sh` 两步构建链：cargo + wasm-bindgen CLI） | D7 | 内核对外契约薄壳；wasm 实机 2.9MB，wasm-opt 兼容已实证 |
| 表现层 `client-web` | React + TypeScript | D8 | 生态最成熟、AI 辅助语料最丰富；文本密集 UI 用组件库 + CSS 动效即可，M3 若需高性能渲染再评估 Pixi |
| 内容管线 `pipeline` | TypeScript | D10 | LLM 编排为 IO 密集非性能热点；TS 生态（结构化输出、流式、重试回路）最顺手 |
| 平台服务 `services` | Rust + Axum | D9 | 与内核同语言，复用 ygg-schema 校验零重复，服务端栈统一；M2 前不动工 |
| 工具链 `tools` | Rust + clap | D7 延伸 | 与内核同语言，直接复用校验逻辑 |
| 共享契约 `schema` | JSON Schema | — | 资源格式契约（D15），语言无关，双端读取 |
| 包管理与构建 | cargo workspace + pnpm workspace | — | 双 workspace 并行；仓库级任务编排后续选定（just / make 均可） |

### 显式否决项（记录避免反复）

- ❌ 纯 Python 实现：用户明确排除（快速但效率低）；个别 LLM 实验脚本如需 Python，仅限一次性工具不入主干
- ❌ 全 TS 内核 / TS 先行后 Rust 重写：M4 双实现漂移风险 / 注定重写，均否决
- ❌ Phaser / 重型游戏引擎（表现层）：图文冒险阶段过度设计；M3 RPG 时重新评估
- ❌ XState（内核状态机）：语义过重，自研薄 FSM（见 architecture-principles.md §3.3）
- ❌ inkjs / json-rules-engine 引入（结论保留，理由更新）：inkjs 为 JS 运行时，叙事语义面已归 ygg Lua stdlib（D11/D16）；json-rules-engine 对应的声明式谓词引擎已从内核范围移除（条件归 Lua，D11）。两者仅作设计参考保留（见 architecture-principles.md §3.4/§3.5）

## 4. 对既有里程碑的影响

| 阶段 | 技术栈注入 |
| --- | --- |
| M0 ✅ | `crates/ygg-core` + `ygg-sandbox` + `ygg-schema` + `ygg-wasm` + `packages/client-web` + `packages/schema` + `packages/mods/test-adventure`（全部已交付） |
| M1 ✅ | `packages/pipeline`（TS）+ `crates/ygg-lint` + playtest harness + client-web 动态载入（全部已交付） |
| M0.5-CS | 服务端 native（Axum 预留）+ 小程序客户端（待立项） |
| M2 | 新增 `services/`（Axum）+ `tools/` CLI |
| M2.5 | gdext 原生链接（spike/godot/ 已 GO） |
| M3 | 表现层评估 Pixi/Canvas 渲染层；内核扩展（战斗/地图）仍在 `ygg-core` |
| M4 | `ygg-core` native 编译进 `services/` 权威服务器 |
