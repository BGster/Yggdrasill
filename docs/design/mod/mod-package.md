# Yggdrasill · Mod 包与多文件纯度契约
> **id** mod/mod-package · **模块** ③ agent-seed mod · **触发** 改 pack.json / manifest / 多文件纯度契约之前
> **读不到代价** hard-refusal（包契约不合法 → 加载被拒）

> **定位**：pack.json / data/config.json / manifest.json 三段包契约与工具白名单 Layer 1 面。
> **replaces**：DESIGN §3.2（L403–419）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../../README.md) · **相关**：[lua-api.md](../kernel/lua-api.md) · [tool-bridge.md](tool-bridge.md) · [serve.md](../host/serve.md)
> **搬运**：以下正文自 DESIGN.md L403–L419 逐字迁移，不重写语义。

### 3.2 Mod 包与多文件纯度契约（`pack.json`、`manifest.json`、`data/*`）

一个 Mod 包目录包含：

1. **`pack.json`（编写与宿主契约声明）**：
   - `id`、`version`、`engineVersion`（semver 范围）、`seed`、`limits`（`maxFuelPerDispatch`、`maxCascadeDepth`、`maxMemoryBytes`）。
   - `code`（可选）：`{ "entry": "mod.lua", "modules": ["src/core/util.lua", ...] }`。当存在时，`entry` 不得出现在 `modules` 中，且每个模块 chunk 顶层由 `ygg_modules::effects` 强制为纯。
   - `entries`：声明式资源清单（数组，每项 `{ id, kind, schema, path }`），逻辑资源按域拆片声明：一个 `agent-config`（`data/config.json`）+ 若干 `scene-texts-fragment`（`data/texts/*.json`，由 `mod.lua` 装配回 TEXTS）+ 若干 `agent-playbook-fragment`（`原 playbook 目录（已删除）*.json`）；片段数随内容演进，以 `pack.json` 为准。
   - `agent`：8 个必填字段（`entryEvent`、`choiceEvent`、`summaryEvent`、`endingPrefix`、`metaKey`、`rejectionPrefixes`、`replayPrefix`、`defaultPayload`）。
   - `inboxRoutes`：声明 `idField`、`externalIdField`、`kindField`，以及每 kind 的 `{ event, fields[] }` 映射，用于异步 JSONL 收件箱摄入；需要宿主注入提交者身份的动作可声明 `actor` / `actor_session` 布尔（r3，缺省 false = 功能关闭）。
   - `tools`（可选，**Layer 1 能力白名单**）：将 `<namespace>.<name>` 映射到 `{ kind: "cli" | "http", program, args[], timeoutMs, maxOutputBytes, readOnly, resultEvent }`。占位符用 `{{arg}}` 表示调用参数，用 `$ENV{VAR}` 表示宿主侧密钥解析。
   - `mounts`（可选，**宿主侧文件挂载声明**）：数组，每项 `{ name, dir?, mode? }`——把宿主目录 bind 成沙箱可见的 `mounts/<name>/…`（沙箱侧路径文法与读写面见 [lua-api.md §3.1.2](../kernel/lua-api.md)，run 目录落点见 [cli-daemon.md](../host/cli-daemon.md)）。字段与校验见 §3.2.2。
   - `packages`（可选，**能力包依赖声明**）：数组，每项 `{ id, version }`——声明本 Mod 消费哪些 `packages/libs/<id>/` 下的能力包，包模块以 `pkg.<id>.*` 进同一张模块表（见 [lua-api.md §3.1](../kernel/lua-api.md)）。字段、闭包与失败面见 §3.2.3。
2. **`data/config.json`（运行时默认值与 Layer 2 发布白名单）**：
   - 依据 `packages/schema/schemas/agent-config.json` 校验。
   - 声明 `play.mode`（`"guide"` | `"goal"`）、`evolution`（`mode`、`allow_paths`、`max_per_run`）、`gates.require_all_green`、`drive`（`maxSteps`、`stagnationLimit`、`decideTimeoutMs`）、`settlement`（`requireNonEmptyEvents`、`requireZeroFailedSteps`、`requireEnding`、`requireInboxConsumed`）、`allowedTools`（**Layer 2 发布白名单**，必须是 `pack.json` `tools` 键的超集），以及可选的 `harness` 适配器元数据。
3. **`manifest.json`（内容寻址完整性锁）**：
   - 依据 `packages/schema/schemas/manifest.json` 校验。为每个代码文件与数据资源记录 `{ path, sha256, byteLength }`；声明了 `packages` 段时另有 `packages: [{ id, files[] }]`（路径相对**包根**、以包 id 打头，见 §3.2.3）。经 `./scripts/sync-seed-mod.sh` 维护。
   - `.ygg/mods/agent-seed` 是唯一事实源，`packages/mods/agent-seed -> ../../.ygg/mods/agent-seed` 是相对软链；`./scripts/sync-seed-mod.sh` 重算 manifest，`./scripts/sync-seed-mod.sh --check` 同时核验软链与漂移。绕开单源在软链另一端独立修改会造成双源漂移，禁止验收。

#### 3.2.1 设计依据（Lua 为唯一可执行接口 / Schema 单一来源 / 多文件纯度）

**Lua 脚本是 Mod 的唯一可执行接口**：叙事流、效果与条件一律写成 Lua，静态内容放 JSON 资源经 `ygg.data(...)` 加载。**Schema 只有一个来源**：`packages/schema/schemas/`，Rust 侧（`jsonschema`）与 TS 侧（`ajv`）必须给出完全一致的判定；`scene-texts.json` 与 `agent-config.json` 的模板也躺在同一目录，LLM 只填数据、绝不自创 Schema 定义。**多文件纯度**：`code.{entry,modules[]}` 显式枚举全部 Lua 模块，只有 `mod.lua` 可以在顶层注册处理器或改变状态，`src/**` 导出纯函数（`M.install(deps)`）并带真实 `@src/<file>.lua:line` 诊断。

**依据**：反方案是声明式 JSON 条件/对话树——它把“表达力”锁在了一种固定的 schema 里，每加一种叙事花样就要改 schema，而 schema 是双语言共同产物，改动成本远高于改一段 Lua。Schema 单一来源是因为双端各写一份校验必然随时间漂移，而“同一个文件在宿主与工具链里判定不同”是最难查的一类缺陷。多文件纯度是为了让模块化不以“谁都能改状态”为代价。

**判据**：① 阻断——在 `src/**` 顶层注册处理器或改 KV、在 `packages/schema/schemas/` 之外新增 Schema 定义、或新增一个未被 `modules[]` 枚举的 Lua 文件；② 症状——加载期 `YGG-MOD-EFFECT` / 模块未声明报错，或 Rust 与 TS 校验同一资源结论不同；③ 确认——`cargo test --workspace`（含 `ygg-schema` / `ygg-lint` 用例）与 `./scripts/sync-seed-mod.sh --check`。

#### 3.2.2 mounts 段（宿主侧文件挂载声明）

```json
"mounts": [
  { "name": "work", "dir": "../biz", "mode": "ro" },
  { "name": "out" }
]
```

| 字段 | 类型 | 必填 | 规则（违规 → `YGG-FS-MOUNT`，attach 期、零写入） |
| --- | --- | --- | --- |
| `name` | string | 是 | `^[a-z][a-z0-9_-]{0,31}$`；同一 pack 内唯一；不得等于 `artifacts` |
| `dir` | string | 否 | 1..=4096 字节、无 NUL；绝对路径原样，相对路径对 `<mod dir>` 解析；`canonicalize` 后必须是**已存在目录**（非文件、非断链）。缺省 = **私有挂载**：内容是本局产物 |
| `mode` | string | 否 | `"ro"`（**缺省**）或 `"rw"`；`ro` 下任何写 → `YGG-FS-MOUNT` |

- 段缺失或 `[]` → 零挂载，不报错（旧 mod 100% 兼容）；
- 段存在但不是数组、或元素不是对象 → `YGG-FS-MOUNT`；
- **未知键忽略**（宽松 Value 读取）：键集宽松、**已知键的取值严格**——与 `panel` 段同源（`pack.json` 解析是宽松 Value 读取，旧宿主忽略未知顶层段）；
- 校验发生在 **attach（宿主侧）**，不是 `ModBundle::load`：`mounts` 与 `panel` 同族（宿主面声明），声明坏了不能炸 mod 装载；`ModBundle` / `modload.rs` 不因本段新增字段；
- **r2 兼容条款**：本表三个字段与上述规则即 r2 `packages/schema/schemas/` 要编码的规范原文。r1 落 Rust 层（宿主可执行）、r2 落 Schema 层（pack 时可校验），**两层判定必须逐条一致**；r1 不预先冻结未来字段，r2 加字段时按「键集宽松」向后兼容。

① 阻断：声明一个 `dir` 指向不存在 / 非目录 / 断链的路径、重名的 `name`、或 `mode` 取值不在 `ro`/`rw` 内而 attach 仍成功；② 症状：挂载点被隐形创建、或声明错了却只在该次 `ygg.fs.*` 调用时才报错（错误面应在 attach 期、零写入）；③ 确认——`cargo test -p ygg-store --test mounts` 的声明校验与三态幂等用例，以及 `git-ls <run-dir>/mounts/` 显示声明与磁盘一致。

#### 3.2.3 packages 段与包装载面（`pkg.*`）

```json
"packages": [
  { "id": "entity", "version": "0.1.0" },
  { "id": "role-skill", "version": "0.1.0" }
]
```

**它是什么**：`packages/libs/<id>/` 下的**能力包**是第二个受控模块命名空间。包模块与本 Mod 的 `src.*` 进**同一张** `ModuleSet`（只是名字前缀不同），所以 `require` 的查表机制一个字不改。

| 面 | 规则 |
| --- | --- |
| 声明形状 | 段存在即**全量校验**：每项必须是 `{id, version}` 对象、两字段非空字符串、`id` 匹配 `^[a-z][a-z0-9_-]*$`、同一 `id` 不得重复。段缺失或 `[]` → 零包，manifest 逐字节不变 |
| **完整闭包** | `packages` 段是**唯一装载清单**。包的 `pkg.json.deps` 只声明依赖关系（用于校验与判环），**不**把未声明的包拉进来；一个 dep 未在本段出现即 `MissingTransitive`。与本仓既有纪律同形：`code.modules` 也必须枚举每一个可 `require` 的模块，没有隐式发现面 |
| 版本 | **精确 pin**（字符串相等）。不符是**声明错**而非完整性错，文案报双方值。无 `semver` 依赖（零新增依赖惯例）；范围语义等到出现真实消费者再立 |
| id → 模块名 | `-` 折叠为 `_`：`role-skill` → `pkg.role_skill`。模块名段字符集只有 `[A-Za-z0-9_]`，`pkg.role-skill` 会被 `normalize` 拒掉——折叠是唯一可行映射，不是风格选择 |
| 文件 → 模块名 | 相对包根去 `.lua` 后按 `/` 拆段，复用 `ygg_modules::name_of_path`（不另写字符规则）：`init.lua` → `pkg.<id>`（包根模块）；`words.lua` → `pkg.<id>.words`；`a/b.lua` → `pkg.<id>.a.b` |
| 允许的文件 | `pkg.json` + 非 `test/**` 的 `*.lua`。其它任何文件可在**打包期**可读拒绝；`.` 开头的条目静默跳过（`.DS_Store` 等）。`test/**` 是开发面：不进模块表、不进 manifest |
| 包根 | `--packages` / `$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs`；不可读**即失败**并点名推导值与两个来源，不静默降级为零包。包根是 **host 环境态**而非 run 身份态，所以本段只写 id/version 不写路径；**内容**由 manifest 的 sha256 钉死（同字节换根行为等价，异字节装载期 hash 失配） |
| 内容寻址 | `manifest.json` 的 `packages: [{id, files[]}]` 逐文件记录 `{path, sha256, byteLength}`，`path` 相对包根且以包 id 打头（段内因而没有 `.`/`..`）。纳管规则：一个包的全部非 `test/**` 文件 + `pkg.json`，按 `(id, path)` 排序（重算幂等） |
| 预算 | 模块数与代码总量按 **mod ∪ 包**的**并集**统一判（`MAX_MODULES=128` / 1 MiB），不分开算 |
| 纯度与 KV | 包模块走同一个装载器，`ShadowBanned` 影子表自动生效：包模块顶层 `ygg.kv.set` / `ygg.on` 等一律拒绝。KV 前缀纪律：包只写**自己契约 §2 声明的前缀**（单写者；文档纪律，装载期不校验）——本轮实例：`role-skill` 写 `role.*`。`pkg.<id>.*` 是**模块命名空间**（`require` 面），不是 KV 前缀；两者别混 |

**失败面**（全在进入内核之前，零副作用）：段不是数组 / 项非对象或缺字段 / id 非法 / 重复 id / 包不存在（报出根下实际有哪些）/ 版本不符（报双方值）/ 缺传递依赖 / 循环（链式文案）/ `pkg.json` 坏（先点名是哪个包）/ manifest 缺段或字节漂移 / 并集超预算。

**诊断面**：`require("pkg.fixture")` 的 chunk 名是**相对包根的真实路径**（`@fixture/init.lua`），不是 `path_of` 会推出的 `pkg/fixture.lua`——否则 traceback 会指一个不存在的文件。

① 阻断：让包源码绕过模块表直接读盘（把包源码复制进每个 mod 目录，改包不重 pack 即静默漂移）、或只声明间接依赖而指望隐式拉包；② 症状：声明的包没生效却无报错（只在运行期表现为一个看似无关的 `NotFound`），或同一包在两个 mod 里装到不同字节；③ 确认——`cargo test -p ygg-sandbox --test pack_packages`（含 8 条拒绝矩阵与纯度负例）、`cargo run --example pack -- <mod> --packages packages/libs` 后 `sh .pi/tools/manifest-verify.sh` 的 `pkg_drift=0`。

**为什么不字面实现「挂入 `package.path`」**：沙箱里**没有** `package` 全局（白名单测试钉死），而且造一个会把「解析顺序」交给 mod 可变状态（同一 seed 两次回放可能装到不同模块，破确定性判据）。所以两个载体两件实事：沙箱 = 模块表注入；普通 Lua（`selftest` / `ygg-lint` 静态面）= `package.searchers` 追加项。两者读**同一份**包索引（`模块名 → 相对包根路径`），结构上不可能漂移。

### pack 声明变更的连带义务

**因 `pack.json` 声明形状变化而必须改的宿主 `#[cfg(test)]` 断言（含 `crates/ygg-sandbox/examples/egg/` 的测试模块）属维护，不是能力扩张**：在同一 commit 改掉，并写明是哪一条声明逼它改。其余一律仍被禁止——生产代码、新宿主能力、任何运行时行为变化。`packages` 段（§3.2.3）的声明形状变更同受本条约束（不新开节）。

**依据**：声明与断言是同一枚硬币的两面；若“改声明”不算理由，每次声明演进都会撞上“不能改断言”的红线，只能绕道或留下过时断言（后者更贵：它把“测试绿”变成历史遗迹）。但它必须与“顺手把测试改到能过”严格区分——后者是能力扩张。

**判据**：① 阻断——因修改 pack 声明而导致断言失败却不同批修断言，或借机改动生产代码 / 新增宿主能力；② 症状——`pack.json` 改了而 `--example egg` 套件变红，或 commit 里同时出现断言改动与未声明的行为改动；③ 确认——`cargo test --example egg -p ygg-sandbox` 绿，且 commit message 点名了逼它改的那条声明。

### Playbook 消费契约（phase-entry + 全局索引）

`src/view/guide.lua` 的 `STAGE_GUIDE` 必须覆盖装配后的全部章 id；每个相位渲染播出**全部候选章**，不是只播首章。`map.show` 保持单一操作视图且忽略 payload，但在 `render()` 产生 choice 之前额外播出 21 章全局索引；因此当前相位章在一次 `map.show` 回执中出现两次，其余章出现一次。退役幕同样播 `pointers`，不留只在全局索引可见的死角。

规范陈述的执行核验是 `selftest` 的静态章集合相等断言与逐相位回执探针；增删章、改章 id/title、改映射或改渲染序必须在同批更新该探针并重算 manifest。违反时测试红，未接线的章不得被宣称为 phase-entry 可达。负责人 = 修改 playbook/guide/render 的执行者；触发 = 任一上述变更；检查 = `cargo test -p ygg-sandbox --test selftest` 与 playtest。

### Mod 文案路径卫生（retired path literals）

`data/texts/**` 的散文不得含退休路径字面量 `ygg-agent` 或 `pi-agent-seed`；必须使用当前 `agent-seed` / `egg`，避免执行者被文案引到不存在的路径。宿主兼容层是旧名唯一合法消费者，且只能在 `crates/**`，不能在 Mod 文案。确实需要历史引用时，该行必须带字面标记 `[deprecated-ref]`，其余出现均违规。

① 阻止任何把旧 mod/CLI 路径重新引入 Mod texts、brief 或 playbook 的编辑；② 症状是执行者对不存在的 `…/mods/pi-agent-seed` 或 `ygg-agent` 执行门禁而 fail-closed，或 serve 面板解析陈旧别名；③ 仓库根运行以下命令，空输出为通过：

```bash
grep -rn 'ygg-agent\|pi-agent-seed' .ygg/mods/agent-seed/data/texts/ | grep -v '\[deprecated-ref\]'
```

任何幸存行都必须加历史标记或改为当前名称。维护责任 = Mod 文案编辑者；触发 = 每个 texts/playbook 文案 commit；机器复核 = `selftest::mod_text_fragments_carry_no_retired_paths`。违规文案不得发布，因为它会把执行者导向 fail-closed 路径。


### 工程命令与多文件约束（原 playbook `mod_engineering` 章，2026-11-15 原样归位）

> 逐条自 原 playbook 章 `ch4-mod-host.json`（已删除） 搬入，**未改写**（保真优先）。
> 其中 `mod.pack` / `mod.manifest-content-addressed` 的操作面已在上一节「pack 声明变更的连带义务」。

- **`mod.pack`**（must）：After touching `data/` or `schemas/` you must run: `cargo run --example pack -- .ygg/mods/agent-seed`.
- **`mod.lint`**（must）：Lua static checks: `cargo run -p ygg-lint -- --pack .ygg/mods/agent-seed` (whole-pack mode, which also validates the `code.modules` declarations) or `cargo run -p ygg-lint -- .ygg/mods/agent-seed/mod.lua` (single-file mode); the result must be 0 errors / 0 warnings.
- **`mod.playtest`**（must）：Smoke walk: `cargo run --example playtest -- .ygg/mods/agent-seed` (the all-1 walk must reach `[ENDING`). All-1 covers the happy path only — the acceptance standard for branches and map wording is `gates.mod-semantics-matrix` (`cargo test -p ygg-sandbox --test selftest`).
- **`mod.manifest-content-addressed`**（must）：The manifest is content-addressed (sha256 + byteLength): editing `data/*.json` without recomputing the pack makes loading fail with a content-hash mismatch.
- **`mod.multi-script-load`**（must）：A multi-file Mod is registered only in the `code` section of `pack.json`: `entry` is not registered as a module, `modules` enumerates every file explicitly (no glob), and a missing `code` section means a single-file Mod (no migration needed for existing Mods). `require` works both at load time and at runtime (inside a handler) — `host.seal` only seals `ygg.on`, so a pure library may be loaded lazily; the price is that a module's top level may be a pure export surface only (see `mod.pure-export`). After changing any module file you must rerun pack (content-addressed: a byte-count or hash mismatch fails loading).
- **`mod.pure-export`**（must）：A multi-file Mod's module chunk performs pure exports only (function and constant definitions) and does nothing with side effects: registration / state writes / output all live in the entry `mod.lua`. A module exposes registration as a function (`function M.install(deps)`) that the entry calls explicitly in order; touching `ygg.on` / `ygg.emit` / `ygg.narrate` / `ygg.choice` / `ygg.meta` / `ygg.kv.set` / `ygg.entity.spawn` / `ygg.entity.set_attribute` / `ygg.entity.add_tag` / `ygg.entity.remove_tag` / `ygg.fsm.define` / `ygg.narrative.node` / `ygg.narrative["goto"]` at a module's top level is refused (the static `YGG-MOD-EFFECT` rule and the runtime shadow table share the single list `ygg_modules::effects`); at the top level only pure reads are allowed (`ygg.data` / `ygg.ticks` / `ygg.kv.get` / `ygg.entity.has_tag` / `ygg.entity.get_attribute`); the entry chunk is exempt (it is the assembly manifest). Why: a handler's dispatch order equals the `ygg.on` registration order — letting a module register itself means "how the files are split / who is required first" decides runtime behavior, so splitting a file or reordering a `require` would change the dispatch order and refactoring would not preserve behavior. `require` is therefore usable at any time (including lazy-loading a pure library inside a handler), but lazy loading never relaxes the boundary: a lazily loaded module's top level is still pure exports. See `docs/design/mod/mod-package.md` (ex-DESIGN §3.2 "Mod Package & Multi-File Purity Contract").
- **`mod.kv-prefix-owner`**（must）：In a multi-file Mod the KV prefixes are partitioned by ownership and each prefix has exactly one writer (a documentation convention; loading does not validate it): `fsm.*` is written only by the stdlib `ygg.fsm` and read-only for the Mod; `settings.*` is written only by `config.set`; `req.*` = the requirement table, `prop.*` = the proposal table, `lane.*` = the lane ledger, `ui.*` = presentation only (never in the summary, never affecting dispatch), and `task.*` / `verify.*` / `review.*` / `inbox.*` plus the bare key `ending` each have their own single writer. Two modules writing one prefix is implicit coupling — share through a module's return value instead.
- **`mod.error-locates`**（must-not）：Never "glue a multi-file Mod back into one file and then report errors": a module chunk name must carry the real file (`@src/core/util.lua`), a refusal the loader decides itself must carry the call site (`src/a.lua:3: <reason>`) and the require chain, and static checks must parse each module independently (a module's `local` never spans chunks, so gluing would invent a shared scope that does not exist at load time). A diagnostic whose line number does not match the real file is a defect.
- **`mod.hot-reload`**（must）：Hot reload: single-shot mode loads from disk in every new process; an `open` (attach) receipt with `hotReload.source="resume"` proves the change is in effect.
- **`mod.two-mods-layering`**（must）：The two mod paths are one Mod, not two copies: `.ygg/mods/agent-seed` is the single source of truth (the installed instance), and `packages/mods/agent-seed` is a git-tracked relative symlink to `../../.ygg/mods/agent-seed` — the stable packages-side entry, never an independent copy. `scripts/sync-seed-mod.sh` verifies the link target and the schema single-source (`packages/schema/schemas/`) and recomputes the content-addressed `manifest.json` (`--check` = read-only); the CI skeleton guard in `check.yml` still runs its diffs, but through the symlink they now compare the same files with themselves. The playbook is assembled, not duplicated: nine fragments under 原 playbook 目录（已删除） (a head plus eight chapter files) are joined by `mod.lua` in fixed order into 21 chapters — one fragment carries one to five chapters, section ids are globally unique across fragments, and this rule lives in the assembled `mod_engineering` chapter.

### pack / config 契约四段（原 playbook `host_contract` 章，2026-11-15 原样归位）

> 逐条自 原 playbook 章 `ch4-mod-host.json`（已删除） 搬入，**未改写**（保真优先）。
> `contract.serve-read-only` 的正文权威在 [host/serve.md](../host/serve.md) §3.7.6。

- **`contract.required`**（must）：The generic tooling (`src/playtest.rs`) hard-codes no event name / narrative prefix / summary key / entry payload / inbox field name / settlement switch / advancement threshold — it always reads the contract of the loaded Mod. The contract is mandatory: a missing section, a missing field, a wrong type or a non-positive integer fails loading (non-zero exit or `ok=false`); that is the deliberate fail-closed choice.
- **`contract.agent-seg`**（must）：The `agent` section of `pack.json` has eight fields: entryEvent / choiceEvent / summaryEvent / endingPrefix / metaKey / rejectionPrefixes (at least one) / replayPrefix / defaultPayload.
- **`contract.inbox-seg`**（must）：The `inboxRoutes` section of `pack.json`: reserved keys externalIdField / payloadField / extMarker plus `<kind>` → `{action, fields[, write]}` (zero kinds is legal).
- **`contract.settlement-seg`**（must）：The `settlement` section (in `<config.json>`): requireEvents / requireNoFailedSteps / requireEnding / requireInboxConsumed.
- **`contract.drive-seg`**（must）：The `drive` section (in `<config.json>`): maxSteps / stallLimit / stepsPerRequirement (positive integers).
- **`contract.config-path`**（must）：`<config.json>` is the path declared by the `ygg:agent/config` entry in `pack.json` (`data/config.json` for the seed Mod).
- **`contract.upgrade`**（must）：Upgrading an old Mod means filling in the four sections (two in `pack.json`, two in `config.json`); `cargo run --example pack -- <mod dir>` validates their shape at pack time (only what is declared is validated).
- **`contract.host-vs-contract`**（must）：Ownership boundary: event names / ending prefix / summary key / rejection and replay prefixes / inbox fields and routes / settlement switches / advancement thresholds belong to the contract and must be declared by the Mod; what the host may keep as literals is the run-directory file names, the coordination phase names running / closing / finalized, the decision-command timeout (`DECIDE_TIMEOUT`) and poll interval (`DECIDE_POLL_INTERVAL`), the three `--drive` modes, the five-file set (`pack.json` / `manifest.json` / `mod.lua` / `schemas/` / `data/`), and the kernel seams (`save` / `load` / `state`, and the reserved `ygg.choice_selected` route). Criterion: would this literal change with another Mod? If yes, it belongs to the contract.
- **`contract.literal-ownership`**（must-not）：Only applications bound to a specific Mod and the generator side may keep event-name / ending-prefix literals, and every such file declares "bound" or "generator-side" in its header: adventure.rs, clients/godot/rust/src/logic.rs, clients/godot/project/main.gd|session.gd, spike/godot/**, packages/client-web/src/{kernel,session,kernel-wasm}.ts and crates/ygg-wasm/smoke.mjs are bound to `packages/mods/test-adventure`; packages/pipeline/src/{prompt,types,pack}.ts and scripts-dev/mock-llm-stub.mjs are generator-side; the generic tooling of crates/ygg-sandbox|ygg-lint and the test fixtures under tests/ may not add literals, and `ygg.choice_selected` is the reserved event name checked by the kernel domain (the contract's `choiceEvent` must use it).
- **`contract.choice-payload-type`**（must）：The payload of `ygg.choice_selected` must be a **JSON number** (`--payload '1'`); sending it as a string (`--payload '"1"'`) is rejected by the kernel domain as VALIDATION "option does not exist". Every other action's payload is a string. Menu selection is the **only** passing channel for the human-action gate — dispatching `proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note` directly through `--event` is always refused (the receipt's first line is `[REJECTED:ACTION]`).
- **`contract.serve-read-only`**（must-not）：Runtime projection of `docs/design/host/serve.md` §3.7.6: never add a run-state write endpoint to serve; human gates remain CLI/daemon-ticketed. POST/DELETE `/api/roots` is only volatile observation configuration and must write no save, mod KV, run.json/coordination or directory; the zero-disk-write tests pin the boundary.

**host 契约模块的模块化拆分（S-25 批四）**：`host_contract` 由单文件 2084 行（800 行上限的 2.6×）拆为 `host_contract/` 目录 7 文件——mod.rs 装配（doc + `CONTRACT_DOC` / `read_json` / `json_kind` 共享件 + 5 个 `mod` 声明 + 全量 `pub use`）+ agent / inbox / tools / settlement / drive 五个契约面文件 + `tools/tool_contract_tests.rs` 测试子文件（24 测随迁；inbox 5 测仍内嵌）。这是**纯搬移**，不是重写：五个面的形状常量、解析规则、错误文案、白名单校验逐字节原样迁移，零逻辑、零可见性放宽（跨面共享的 `json_kind` 归位装配面，与 `read_json` 同类——子模块可见父模块私有项，无需 `pub(super)`）。`ygg_sandbox::host_contract::{…}` 的 pub 路径经 re-export 逐字节不变（egg 13 符号 use、`playtest.rs`、`pack.rs` 零改动），29 个内嵌测试逐名守恒。行为不变量由门禁计数守恒背书：workspace 581、egg 158、pkg selftest 199、agent-seed 79、superagent 183，五门禁 before/after 逐项相等。拆分动机只有一个：单文件超限使契约面边界不可读——拆分后「契约 = 装配（mod.rs）+ 五个独立面文件」的形状与本节 `contract.agent-seg`…`contract.drive-seg` 的分段叙述一一对应（批一 egg.rs、批二 serve、批三 host_api 同法）。
