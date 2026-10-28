# Yggdrasill · 开发主 Agent 约定

> **这是什么**：本仓库开发侧的主 Agent 定义——定义"帮人开发 Yggdrasill"的 Agent 在项目管理、开发、集成、合并、部署全流程中应当遵守什么。
> **项目定位**：游戏 + 平台（数据驱动规则内核 + AI 生成 Mod：Lua 模板 + 数据填充，D11），Web-first，内核 Rust→WASM。规划与决策的完整背景见 `docs/vision.md`、`docs/architecture-principles.md`、`docs/modules-and-tech-stack.md`。
> 落点：pi / Codex 原生读本文件；`CLAUDE.md` 是本文件的软链（Claude Code 用）。

---

## 一、凭据与数据红线（最高优先级，不可协商）

**禁止把任何凭据、密钥、令牌写入仓库或对话产物。** 包括但不限于：

- LLM API key（M1 内容管线的模型调用凭据）；
- 平台服务（M2+）的数据库连接串、对象存储 AK/SK；
- 任何第三方服务的 token。

**纪律**：

1. 本地开发凭据一律走 `.env`（已 gitignore，权限 600）；`.env.example` 只放键名不放值。
2. 汇报、文档、commit message、Mod 示例数据中只出现**键名**，不出现值。
3. 本仓库无涉密业务数据，但**示例/测试数据一律自造合成数据**，不引入任何真实个人数据。
4. 违规后的处理：如实列出暴露了什么（具体文件 + 内容位置），不淡化、不辩解；随后停止相关操作，等待用户指示。

---

## 二、仓库形态

```text
yggdrasill/
├── AGENTS.md                 开发侧主 Agent 定义（本文件）
├── CLAUDE.md                 -> AGENTS.md（软链）
├── docs/                     人读文档（愿景 / 架构原则 / 模块与技术栈）
│   ├── vision.md             ★ 项目决策唯一事实源（D1-D16 决策表）
│   ├── architecture-principles.md
│   └── modules-and-tech-stack.md
├── crates/                   Rust workspace（M0 落地中）
│   ├── ygg-core/             规则内核：纯逻辑，零 IO，零渲染依赖
│   ├── ygg-sandbox/          确定性 Lua 沙箱：mlua + host API 白名单 + ygg stdlib 嵌入
│   ├── ygg-schema/           资源数据模型 + 校验
│   └── ygg-wasm/             wasm-bindgen 绑定薄壳
├── packages/                 pnpm workspace（规划中）
│   ├── schema/               JSON Schema 文件——资源格式契约单一事实源（D15）
│   ├── client-web/           表现层（React + TS）
│   └── pipeline/             M1 内容管线（TS）
├── services/                 M2+ 平台服务（Rust + Axum）
└── tools/                    CLI（Rust + clap）
```

**三条必须记住的形态约束：**

1. **`docs/vision.md` 的决策表是项目决策唯一事实源**：任何方案讨论收敛出的新决策（D11+）必须回填该表；与决策冲突的实现一律视为缺陷。
2. **JSON Schema 文件（`packages/schema/`）是资源格式契约的单一事实源**（D15）：TS 侧（ajv）与 Rust 侧（jsonschema crate）读同一份文件，禁止任何一侧另写内联副本；Mod 的可执行接口为 Lua 脚本（D11，2026-10-27），Schema 不约束 Mod 逻辑。
3. **`crates/ygg-core` 零依赖铁律**：不依赖任何内部模块、DOM/平台 API、渲染库（详见 `docs/architecture-principles.md` §2 三条铁律）；代码审查与门禁按此检查；mlua 等 VM 依赖仅允许出现在 `crates/ygg-sandbox`（D12）。

---

## 三、分支管理与泳道

### 分支模型

| 分支类型 | 命名 | 用途 |
| --- | --- | --- |
| 主干 | `master` | 唯一主干；部署只认 master |
| 集成分支 | `develop/yggdrasill`（预留） | 迭代轮次多、泳道并行起来后启用；启用前功能分支直接合 master |
| 功能分支 | `feature/<主题>` 或 `docs/<主题>` | 一切开发（含文档）都在功能分支进行 |

- **no-FF 铁律**：一切分支合并（功能 → 集成 → 主干）一律 `git merge --no-ff`，禁止 fast-forward——保留合并节点，让迭代批次（如 `v<日期>.<序号>`）在历史图中可追溯、可整体回滚。
- **禁止直接在 master 上开发**；master 只接受合并。
- **push 纪律**：功能分支核验通过后 push origin（供协同与备份）；建 PR、合并入 master 由用户显式触发。

### 泳道 worktree（并行开发时启用）

- 约定路径：`~/tmp/worktrees/yggdrasill-<n>`（空闲时 detach 在 `origin/master` 头）。
- **挂牌机制（硬约束）**：每条泳道 worktree 根维护 `.lane` 文件（不进 git）：`LANE_OCCUPIED=true|false` + `LANE_TASK` / `LANE_BRANCH` / `LANE_SINCE`。
  1. **派发前必检**：`LANE_OCCUPIED=true` 则换泳道或等释放；无文件视为空闲但须补挂牌。
  2. **占用即挂牌**；**收口即摘牌**（任务合并/废弃后改回 false、清空任务字段）。
  3. **共享禁止**：两个任务不得共用同一 worktree；派发前要求 `git status` 干净（`.lane` 除外）。

### Subagent 派发纪律（硬约束）

1. **后台派发，不阻塞主会话**：创建 subagent 执行任务一律后台运行，主会话不原地等待——派发期间继续推进其他泳道、核验或规划工作，收到完成通知后再核验合流。仅当后续动作严格依赖其结果且无任何可并行推进的工作时，才允许同步等待（并优先将该时间用于其他泳道）。
2. **模型与 thinking 从 `.env.local` 读取**：派发前读取仓库根 `.env.local`（已 gitignore，绝不入库）：
   - `YGG_SUBAGENT_MODEL`：subagent 模型（如 `sonnet` / `provider/model-id`）；
   - `YGG_SUBAGENT_THINKING`：thinking level（`off|minimal|low|medium|high|xhigh|max`）。
   未设置则不显式传参（用 harness 默认）。值仅用于派发参数——不打印、不写文档、不进 commit。

---

## 四、开发-集成-合并-部署流程

### ① 迭代点级（一个功能分支的收口）

1. `git fetch origin`
2. 检查主干是否前移：分叉则先 `git pull --ff-only` 对齐，再 rebase 功能分支
3. **门禁全绿**（见 §五）——未过门禁不得请求合并
4. 合并（由用户触发或用户批准后由主 Agent 执行）：主检出 `git merge --no-ff <feature>`
5. push master；删除远端功能分支（可选）

**串行收口**：集成/合并/部署操作由主 Agent（主会话）串行执行；泳道 subagent 只负责开发 + 门禁 + push 功能分支，**不做**集成/合并/部署。

### ② 轮次级（一组迭代点构成一个可发布批次）

1. 端到端验收通过（游戏可玩性走查：示例内容完整通关 + 存档/读档验证；M1 后加 AI 生成→可玩闭环）
2. 打批次 tag：`v<YYYY.MM.DD>.<序号>`（no-FF 合并节点是回滚锚点）
3. CHANGELOG / 汇报（如有）

### ③ 部署级

- **只许部署 master（硬约束）**：任何通道部署的内容必须已合入 master，非 master ref 直接拒绝，无例外开关。
- **部署通道**（随阶段定稿，未定稿前本节为占位）：
  - M0-M2：Web 客户端静态托管（通道候选：GitHub Pages / Cloudflare Pages，**待用户定稿后回填此处**）
  - M2+：`services/` 平台服务部署方案待定
- 部署属写操作，**每次部署前向用户报备并获批准**。

---

## 五、验证命令门禁

```bash
# Rust 侧（crates/）
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# wasm 绑定（ygg-wasm 实装后，B1）
# 注意：不用 wasm-pack——EH 模块需 wasm-bindgen --reference-types，wasm-pack 不透传该旗标。
# 需 WASI SDK（约定 /opt/wasi-sdk 或 WASI_SDK_PATH 导出，见 .cargo/config.toml 头部注释）
# 可选后处理：PATH 上有 wasm-opt（binaryen ≥118，brew install binaryen）时自动 -Oz；
#   YGG_WASM_OPT=off 可关、=on 令失败致命（binaryen 117 对 EH 模块优化必崩，实证在脚本头注）。
./scripts/build-wasm.sh && node crates/ygg-wasm/smoke.mjs

# FR-9 wasm 侧双跑哈希等价（M1 C2 起入门禁）：golden-vector 同序列经 wasm
# 内核在 node 上双跑，状态哈希必须等于 fixture 钉死的 native 基线（不等 =
# 平台间确定性破口，不得静默改基线）。
node crates/ygg-wasm/verify-determinism.mjs

# FR-9 残余缺口注记（不得静默吸收）：node 泳道已覆盖 native/wasm 双跑哈希
# 等价（上）；wasm-bindgen-test 浏览器泳道（真实浏览器 wasm 实例重放同一
# 断言）仍未实现，声称 FR9 全量满足前需补该泳道或显式决策豁免。

# TS 侧（packages/，进对应包目录）
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest
pnpm build         # vite build

# 契约一致性（有双端校验后）
# 同一份 Mod 样例分别过 ajv 与 jsonschema crate，结果必须一致（CI 脚本承载，待落地）
```

- 门禁基线是**快照**（随迭代增长），验收标准是「不引入新失败」。
- **内核纯度检查**：`ygg-core` 的 `Cargo.toml` 依赖清单纳入审查——出现渲染/平台/IO 类依赖即违规。
- 门禁未接通的模块（尚未建立测试的）须在汇报中显式声明「门禁未覆盖」，不得默认通过。

---

## 六、分层验证纪律

门禁单测只证明「没破坏既有断言」，不证明游戏真的能玩。两轮验证缺一不可：

1. **迭代点级**：功能分支合并前，除门禁外必须有过活链验证——内核改动用 Rust 单测 + 一个最小可玩脚本驱动（加载样例 Mod → 推进若干事件 → 断言状态）；表现层改动浏览器实走并截图。
2. **轮次级**：一批迭代点合并后，完整通关一次示例游戏（M1 后含 AI 生成内容闭环），验证点逐项实证，**证据先行**：界面结论配截图、接口/内核结论配命令输出与退出码。

---

## 七、证据优先

任何结论必须来自以下之一，不得凭印象断言：

- 命令输出与**退出码**；
- 产物文件的存在性与内容；
- 读到的源码原文（引用时给 `文件:行号`）。

不确定就说不确定，并给出"下一步怎么验证"。不写"我已完成"这类无证据的完成语。

---

## 八、语言与输出

- 与用户对话：中文；代码、标识符、路径、命令：原样英文。
- 报错/结论先给结论，再给依据。
- 改文件前先读；改多个文件时逐个说明改了什么、为什么。
- 大段机械改写（尤其 Markdown 树形结构、表格）先打印结果再写盘，写盘后立即回读校验。

---

## 九、去哪找（只放指针，不复制内容）

| 想做什么 | 去哪 |
| --- | --- |
| 项目定位 / 演进路线 / 决策表 | `docs/vision.md` |
| 架构铁律 / 内核组件边界 / 开源评估 | `docs/architecture-principles.md` |
| 模块划分 / 技术栈 / 否决项 | `docs/modules-and-tech-stack.md` |
| 内核设计约束（实体/事件/状态机/叙事；条件归 Lua，D11） | `docs/architecture-principles.md` §3 |
| 改 `.gitignore` | 参考根目录现有一份；改动说明写进 commit message |

---

## 十、本仓库硬约束（不要做的事）

1. **不把凭据写进仓库**（§一）——这是唯一一条不可协商的。
2. **不在 master 上直接开发**；master 只接受 no-FF 合并。
3. **不部署非 master 内容**；每次部署前报备并获用户批准。
4. **不给 `ygg-core` 加渲染/平台/IO 依赖**；发现"内核需要碰 DOM/网络"时，说明架构分层出了问题，回设计层讨论。
5. **不双写 Schema**：资源格式契约只改 `packages/schema/`，Rust/TS 两侧都是从属方（Schema 约束的是资源文件格式而非 Mod 逻辑——Mod 可执行接口为 Lua，见 D11/D15）。
6. **不让 subagent 做集成/合并/部署**——收口永远由主 Agent 串行执行。
7. **决策变更必须回填 `docs/vision.md` 决策表**；代码与决策冲突时，要么改代码、要么走决策变更流程，不允许静默漂移。
