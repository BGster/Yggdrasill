# Yggdrasill · Mod 包与多文件纯度契约

> **定位**：pack.json / data/config.json / manifest.json 三段包契约与工具白名单 Layer 1 面。
> **replaces**：DESIGN §3.2（L403–419）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[lua-api.md](lua-api.md) · [tool-bridge.md](tool-bridge.md) · [serve.md](serve.md)
> **搬运**：以下正文自 DESIGN.md L403–L419 逐字迁移，不重写语义。

### 3.2 Mod 包与多文件纯度契约（`pack.json`、`manifest.json`、`data/*`）

一个 Mod 包目录包含：

1. **`pack.json`（编写与宿主契约声明）**：
   - `id`、`version`、`engineVersion`（semver 范围）、`seed`、`limits`（`maxFuelPerDispatch`、`maxCascadeDepth`、`maxMemoryBytes`）。
   - `code`（可选）：`{ "entry": "mod.lua", "modules": ["src/core/util.lua", ...] }`。当存在时，`entry` 不得出现在 `modules` 中，且每个模块 chunk 顶层由 `ygg_modules::effects` 强制为纯。
   - `entries`：声明式资源清单（数组，每项 `{ id, kind, schema, path }`），逻辑资源按域拆片声明：一个 `agent-config`（`data/config.json`）+ 若干 `scene-texts-fragment`（`data/texts/*.json`，由 `mod.lua` 装配回 TEXTS）+ 若干 `agent-playbook-fragment`（`data/playbook/*.json`）；片段数随内容演进，以 `pack.json` 为准。
   - `agent`：8 个必填字段（`entryEvent`、`choiceEvent`、`summaryEvent`、`endingPrefix`、`metaKey`、`rejectionPrefixes`、`replayPrefix`、`defaultPayload`）。
   - `inboxRoutes`：声明 `idField`、`externalIdField`、`kindField`，以及每 kind 的 `{ event, fields[] }` 映射，用于异步 JSONL 收件箱摄入；需要宿主注入提交者身份的动作可声明 `actor` / `actor_session` 布尔（r3，缺省 false = 功能关闭）。
   - `tools`（可选，**Layer 1 能力白名单**）：将 `<namespace>.<name>` 映射到 `{ kind: "cli" | "http", program, args[], timeoutMs, maxOutputBytes, readOnly, resultEvent }`。占位符用 `{{arg}}` 表示调用参数，用 `$ENV{VAR}` 表示宿主侧密钥解析。
2. **`data/config.json`（运行时默认值与 Layer 2 发布白名单）**：
   - 依据 `packages/schema/schemas/agent-config.json` 校验。
   - 声明 `play.mode`（`"guide"` | `"goal"`）、`evolution`（`mode`、`allow_paths`、`max_per_run`）、`gates.require_all_green`、`drive`（`maxSteps`、`stagnationLimit`、`decideTimeoutMs`）、`settlement`（`requireNonEmptyEvents`、`requireZeroFailedSteps`、`requireEnding`、`requireInboxConsumed`）、`allowedTools`（**Layer 2 发布白名单**，必须是 `pack.json` `tools` 键的超集），以及可选的 `harness` 适配器元数据。
3. **`manifest.json`（内容寻址完整性锁）**：
   - 依据 `packages/schema/schemas/manifest.json` 校验。为每个代码文件与数据资源记录 `{ path, sha256, byteLength }`。经 `./scripts/sync-seed-mod.sh` 维护。

### Playbook 消费契约（phase-entry + 全局索引）

`src/view/guide.lua` 的 `STAGE_GUIDE` 必须覆盖装配后的全部章 id；每个相位渲染播出**全部候选章**，不是只播首章。`map.show` 保持单一操作视图且忽略 payload，但在 `render()` 产生 choice 之前额外播出 21 章全局索引；因此当前相位章在一次 `map.show` 回执中出现两次，其余章出现一次。退役幕同样播 `pointers`，不留只在全局索引可见的死角。

规范陈述的执行核验是 `agent_seed_semantics` 的静态章集合相等断言与逐相位回执探针；增删章、改章 id/title、改映射或改渲染序必须在同批更新该探针并重算 manifest。违反时测试红，未接线的章不得被宣称为 phase-entry 可达。负责人 = 修改 playbook/guide/render 的执行者；触发 = 任一上述变更；检查 = `cargo test -p ygg-sandbox --test agent_seed_semantics` 与 playtest。

### Mod 文案路径卫生（retired path literals）

`data/texts/**` 与 `data/playbook/**` 的散文不得含退休路径字面量 `ygg-agent` 或 `pi-agent-seed`；必须使用当前 `agent-seed` / `egg`，避免执行者被文案引到不存在的路径。宿主兼容层是旧名唯一合法消费者，且只能在 `crates/**`，不能在 Mod 文案。确实需要历史引用时，该行必须带字面标记 `[deprecated-ref]`，其余出现均违规。

① 阻止任何把旧 mod/CLI 路径重新引入 Mod texts、brief 或 playbook 的编辑；② 症状是执行者对不存在的 `…/mods/pi-agent-seed` 或 `ygg-agent` 执行门禁而 fail-closed，或 serve 面板解析陈旧别名；③ 仓库根运行以下命令，空输出为通过：

```bash
grep -rn 'ygg-agent\|pi-agent-seed' .ygg/mods/agent-seed/data/texts/ .ygg/mods/agent-seed/data/playbook/ | grep -v '\[deprecated-ref\]'
```

任何幸存行都必须加历史标记或改为当前名称。维护责任 = Mod 文案编辑者；触发 = 每个 texts/playbook 文案 commit；机器复核 = `agent_seed_semantics::mod_text_fragments_carry_no_retired_paths`。违规文案不得发布，因为它会把执行者导向 fail-closed 路径。

