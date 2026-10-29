# @ygg/pipeline — M1 AI 内容管线

「描述一个剧本 → 可玩」闭环（FRD：`.rpiv/artifacts/discover/2026-10-28_18-02-47_m1-ai-content-pipeline.md`）：
自然语言剧本 → LLM 生成（Lua 模板 + 资源数据，D11/D-5）→ 三道门（Schema 校验 / ygg-lint 静态检查 / 无头装载走查）→ 有界重生成 → `packages/mods/generated/<slug>/` 五件套。

## 准备

1. 仓库根创建 `.env`（复制 `packages/pipeline/.env.example`；已被 gitignore）：
   `YGG_LLM_BASE_URL` / `YGG_LLM_API_KEY` / `YGG_LLM_MODEL` / `YGG_LLM_PROTOCOL`。
   汇报与日志只见键名，不见值（凭据红线）。
2. Rust 工具链可用（lint 与 playtest 两道门以子进程调 `cargo run`）。

## 使用

```bash
pnpm -C packages/pipeline run generate -- "一个关于守灯塔人在暴风雨夜救下搁浅信天翁的短故事"
# 可选：--max-retries N（默认 3）/--out <目录>/--story-file <路径>
```

成功后：`packages/mods/generated/<slug>/` 生成五件套（mod.lua / pack.json /
manifest.json / data/scenes.json / schemas/scene-texts.json，与 test-adventure 同构）。

- 无头验收复跑：`cargo run -p ygg-sandbox --example playtest -- packages/mods/generated/<slug>`
- 静态检查复跑：`cargo run -p ygg-lint --bin ygg-lint -- packages/mods/generated/<slug>/mod.lua`
- 浏览器实玩（本地 dev）：`pnpm -C packages/client-web run dev` → `http://localhost:5173/?mod=generated/<slug>`
  （生成 Mod 仅本地 dev 可用；部署站只载静态 test-adventure，D-6）
  **dev server 已运行时新生成的 mod**：无需重启——vite 的 import.meta.glob 收集
  发生在模块转换期且**不重扫新增文件**（spike 实证），新生成的 slug 由 client-web
  的 dev middleware（`/__generated-mod/<slug>`，按需读盘）兑接：直接访问
  `?mod=generated/<slug>` 即载（C1 plan B）。缺失/未知 slug（middleware 404）
  仍落「不可用……已回退默认冒险」横幅（设计的优雅降级面不变）。
- 人工收编（满意后入 git）：`mv packages/mods/generated/<slug> packages/mods/<新名> && git add packages/mods/<新名>`

### 定向修改（revise，A4）

对已生成的 Mod 提一条意见做定向改写，改写产同样过三道门后覆盖同 slug：

```bash
pnpm -C packages/pipeline run revise -- <slug> "把风暴夜的光柱写得更冷硬一些"
# 可选：--allow-lua（授权大改）/--out <目录>/--feedback-file <路径>
```

**diff 白名单**（A4 核心；越权整轮作废，磁盘一字不动，`--allow-lua` 是唯一的解锁开关）：

| 变更 | 放行 | 拒绝 |
| --- | --- | --- |
| 六节已有键的文案 / tuning 数值 | 是（意见驱动的正常修改） | — |
| `mod.lua`（任一字节） | 仅 `--allow-lua` | `REVISE_LUA_FROZEN` |
| 六节键集增删（场景 / 结局 id 等） | 仅 `--allow-lua` | `REVISE_STRUCTURE`（冻结的 mod.lua 引用会悬空或成死数据） |
| `slug` / `title`（作品身份） | 否 | `REVISE_IDENTITY`（换 id/换名请重新 generate） |
| 空 diff（模型认为无需改） | 警告放行 `REVISE_NOOP` | — |

- 变更摘要（`scenes.scenes.storm` 这样的点分路径）逐条打印，也是 `ReviseResult.changed`；
- **单轮不重试**：revise 的失败面是「越权」或「意见没落地」，自动重试容易把越权改得更隐蔽；
- **title 锚点**：五件套不含 slug/title（pack.json 是 D-9 固定模板），故 generate/revise 成功后
  在 `<outDir>/.meta/<slug>.json` 记 `{slug, title}`，供下一轮冻结校验用（目录外，不进分发产物）；
  没有锚点的 A1-A3 期产物首轮降级为「以 slug 占位、跳过 title 校验」，成功后补写锚点。

## 失败行为

**generate**：重试额度耗尽（默认 3 次重试 + 首轮）：退出码 1；最后一轮已过 story 契约校验时，
最后产物保留在 `packages/mods/generated/<slug>/`（现场保留，FR-7），契约未过则
不落盘（outDir 完整五件套或不存在，D-15）。stderr 打印聚合错误
（哪道门 / 哪条规则 / 文件:行号 / 修复 hint）。每轮打印累计 token 计数（软警告，无硬闸）。

**revise**：退出码 0 = 改写已过白名单 + 三道门并覆盖落盘；1 = 被白名单/门拒绝（磁盘保持原样）；
2 = 配置面错误（bundle 不存在 / pack.json 声明面不符 / LLM 环境变量缺失 / 意见为空）。

## 预置 schema 模板集

当前仅 `scene-texts` 单模式（键集故事自定、六节结构固定；模板单源落
`packages/schema/schemas/scene-texts.json`，D-10）。**扩模板触发条件**：当生成需求出现
「结构化实体集合」（角色卡 / 物品表）且 scene-texts 单模式开始表达扭曲
（把实体塞进 tuning/beats 凑数）时，启动扩模板小迭代（新增模板 = 新 schema
文件 + pack 模板扩展，机制不变）。

## 测试

- `pnpm -C packages/pipeline run test`——单测（mock provider / fake runner，秒级）
- `pnpm -C packages/pipeline run test:integration`——闭环集成（mock provider + 真实三道门子进程，需 Rust 工具链）
- `pnpm -C packages/pipeline run typecheck`
