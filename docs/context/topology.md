# Yggdrasill · Monorepo 模块拓扑与依赖白名单

> **定位**：仓库目录拓扑全景与 4 条硬性依赖不变量的唯一清单。
> **replaces**：CONTEXT §4（L97–133）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[principles.md](principles.md) · [../design/mod-package.md](../design/mod-package.md)
> **搬运**：以下正文自 CONTEXT.md L97–L133 逐字迁移，不重写语义。

## 4. Monorepo 模块拓扑与依赖白名单

```text
Yggdrasill/
├── crates/
│   ├── ygg-core/        # 纯确定性内核：实体表、事件总线、迁移驱动器（零内部依赖；仅 serde/serde_json/thiserror）
│   ├── ygg-modules/     # 纯多文件 Lua 模块路径解析与副作用禁用清单（零依赖叶 crate）
│   ├── ygg-schema/      # Manifest SHA-256 内容寻址与 JSON Schema 校验（仅依赖 ygg-core）
│   ├── ygg-store/       # 运行持久化端口：SQLite 默认（run.sqlite + VFS）与遗留 JSON 后端（仅作为 ygg-sandbox 示例的 dev 依赖）；observe.rs = serve 只读观察面
│   ├── ygg-sandbox/     # 确定性 Lua 5.4 VM（mlua）、宿主 API 白名单、ygg stdlib、ModBundle 加载器、宿主 CLI（examples/agent/，含 serve.rs 只读观察台与 assets/panel 内置两级列表页）
│   ├── ygg-lint/        # 静态 AST 检查器（full_moon）：YGG-R1/R2/R3、YGG-MOD-EFFECT、YGG-PARSE（仅依赖 ygg-modules，零内核/VM 依赖）
│   └── ygg-wasm/        # 浏览器 WebAssembly 绑定（wasm-bindgen 薄壳，位于 ygg-core + ygg-sandbox 之上）
├── packages/
│   ├── schema/          # JSON Schema 单一事实来源（manifest、pack、story、agent-config）
│   ├── mods/
│   │   ├── agent-seed -> ../../.ygg/mods/agent-seed         # 指向规范工作流 Mod 的符号链接
│   │   └── test-adventure/                                  # 参考可玩文字冒险 Mod
│   ├── client-web/      # React + TypeScript 浏览器播放器
│   └── pipeline/        # TypeScript AI Mod 生成与校验循环
├── clients/
│   └── godot/           # Godot 4 原生 gdext 客户端（M2.5）
├── .ygg/
│   └── mods/agent-seed/      # 规范多文件 Agent 治理 Mod（mod.lua、src/** 五层、data/texts|playbook|config、schemas/）
├── .pi/
└── docs/
    ├── README.md            # LLM Wiki 顶层索引（地图 + 导航表 + 阅读路径）
    ├── CONTEXT.md           # 薄壳索引：领域/决策树入口（context/）
    ├── DESIGN.md            # 薄壳索引：系统设计树入口（design/）
    ├── wiki-governance.md   # 活文档闭环三职责
    ├── context/             # 领域语境：愿景/词汇/原则/拓扑/决策台账
    └── design/              # 系统设计：边界/状态机/剧情/事件/接口契约
```

### 硬性依赖不变量
- `ygg-core` 只能依赖 `serde`、`serde_json` 与 `thiserror`。任何 VM、Schema 校验器、I/O 或平台 API 都不得进入 `ygg-core`。
- `ygg-modules` 是被 `ygg-sandbox` 与 `ygg-lint` 共享的零依赖叶 crate。
- `ygg-lint` 绝不依赖 `ygg-core`、`ygg-schema` 或 `ygg-sandbox`（保证静态 AST 检查快速且免去 `mlua` 编译）。
- `ygg-store`（及 `rusqlite`）只被原生宿主 CLI（`crates/ygg-sandbox/examples/agent/main.rs`，经 `[dev-dependencies]`）链接，确保 `ygg-wasm` 与 `ygg-lint` 的依赖树 100% 不含 SQLite 与文件系统依赖。

---

