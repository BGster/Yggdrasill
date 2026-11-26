---
type: "Reference"
title: "Mod 包与多文件纯度契约"
openwiki_generated: true
verified:
  - by: openwiki/0.9.0
    at: 2026-11-26T05:58:54.950Z
sources:
  - id: openwiki-source-8339725b1afcf7133cd86688
    resource: repo://.ygg/mods/agent-seed/pack.json
  - id: openwiki-source-3ce02e4845420a1c74760ca3
    resource: repo://crates/ygg-sandbox/src/modload.rs
  - id: openwiki-source-6556f80f4ad5bd14105c33c6
    resource: repo://crates/ygg-sandbox/src/pack.rs
  - id: openwiki-source-c865b58db132b51b98081857
    resource: repo://crates/ygg-sandbox/src/packages/assemble.rs
  - id: openwiki-source-bdffe0f278ee83561481503b
    resource: repo://crates/ygg-sandbox/src/packages/closure.rs
  - id: openwiki-source-8ecd38c9620c19978dec4473
    resource: repo://scripts/sync-seed-mod.sh
generated: { by: "pi", at: "2026-11-26T05:58:54.950Z" }
---

# Mod 包与多文件纯度契约

> **id** mod/mod-package · **模块** ③ agent-seed mod · **触发** 改 pack.json / manifest / 多文件纯度契约之前
> **读不到代价** hard-refusal（包契约不合法 → 加载被拒）
> **replaces**：DESIGN §3.2（L403–419）+ playbook `mod_engineering` / `host_contract`（搬运基线 `3a2536f`）
> **上游**：[architecture/README.md](../../architecture/README.md) · **相关**：[../kernel/lua-api.md](../kernel/lua-api.md) · [tool-bridge.md](tool-bridge.md) · [../host/serve.md](../host/serve.md)

## 1. 三段包契约

一个 Mod 包目录 = **`pack.json`（声明）+ `data/*`（资源）+ `manifest.json`（内容寻址锁）**。

### 1.1 `pack.json` 必填与可选段

| 段 | 内容 |
| --- | --- |
| 顶层 | `id`、`version`、`engineVersion`（semver 范围）、`seed`、`limits`（`maxCascadeDepth`、`maxMemoryBytes`；`maxFuelPerDispatch` 是 spec-only 键、crates 零解析，已随 S34 燃料退役删除） |
| `code`（可选） | `{ "entry": "mod.lua", "modules": ["src/core/util.lua", …] }`；`entry` 不得出现在 `modules`；每个模块 chunk 顶层由 `ygg_modules::effects` 强制为纯 |
| `entries` | 声明式资源清单 `{ id, kind, schema, path }`，按域拆片：一个 `agent-config`（`data/config.json`）+ 若干 `scene-texts-fragment`（`data/texts/*.json`，由 `mod.lua` 装配回 TEXTS）+ 若干 `agent-playbook-fragment` |
| `agent` | **8 个必填字段**：`entryEvent` / `choiceEvent` / `summaryEvent` / `endingPrefix` / `metaKey` / `rejectionPrefixes` / `replayPrefix` / `defaultPayload` |
| `inboxRoutes` | `idField` / `externalIdField` / `kindField` + 每 kind 的 `{ event, fields[] }`；需要身份的动作可声明 `actor` / `actor_session` 布尔（缺省 false = 功能关闭） |
| `tools`（可选） | **Layer 1 能力白名单**：`<namespace>.<name>` → `{ kind: "cli"\|"http", program, args[], timeoutMs, maxOutputBytes, readOnly, resultEvent }`；占位 `{{arg}}` = 调用参数，`$ENV{VAR}` = 宿主侧密钥解析 |
| `mounts`（可选） | 宿主侧文件挂载声明（§2） |
| `packages`（可选） | 能力包依赖（§3） |

### 1.2 `data/config.json`（运行时默认值 + Layer 2 发布白名单）

依 `packages/schema/schemas/agent-config.json` 校验。声明 `play.mode`（`guide` \| `goal`）、`evolution`（`mode` / `allow_paths` / `max_per_run`）、`gates.require_all_green`、`drive`（`maxSteps` / `stagnationLimit` / `decideTimeoutMs`）、`settlement`（`requireNonEmptyEvents` / `requireZeroFailedSteps` / `requireEnding` / `requireInboxConsumed`）、`allowedTools`（**Layer 2 白名单，必须是 `pack.json` `tools` 键的超集**），以及可选 `harness` 适配器元数据。

### 1.3 `manifest.json`（内容寻址完整性锁）

依 `packages/schema/schemas/manifest.json` 校验，为每个代码文件与数据资源记 `{ path, sha256, byteLength }`；声明了 `packages` 段时另有 `packages: [{ id, files[] }]`（路径相对**包根**、以包 id 打头）。经 `./scripts/sync-seed-mod.sh` 维护。

`.ygg/mods/agent-seed` 是**唯一事实源**，`packages/mods/agent-seed -> ../../.ygg/mods/agent-seed` 是相对软链；`sync-seed-mod.sh` 重算 manifest，`--check` 同时核验软链与漂移。**绕开单源在软链另一端独立修改 = 双源漂移，禁止验收。**

## 2. `mounts` 段（宿主侧挂载声明）

```json
"mounts": [ { "name": "work", "dir": "../biz", "mode": "ro" }, { "name": "out" } ]
```

| 字段 | 必填 | 规则（违规 → `YGG-FS-MOUNT`，attach 期、零写入） |
| --- | --- | --- |
| `name` | 是 | `^[a-z][a-z0-9_-]{0,31}$`；同一 pack 内唯一；不得等于 `artifacts` |
| `dir` | 否 | 1..=4096 字节、无 NUL；绝对路径原样，相对路径对 `<mod dir>` 解析；`canonicalize` 后必须是**已存在目录**（非文件、非断链）。缺省 = **私有挂载**（内容为本局产物） |
| `mode` | 否 | `"ro"`（**缺省**）或 `"rw"`；`ro` 下任何写 → `YGG-FS-MOUNT` |

段缺失或 `[]` → 零挂载、不报错（旧 mod 100% 兼容）；段非数组或元素非对象 → `YGG-FS-MOUNT`；**未知键忽略**（宽松 Value 读取：键集宽松、已知键取值严格）。校验发生在 **attach（宿主侧）**，不是 `ModBundle::load`——`mounts` 与 `panel` 同族，声明坏了不能炸 mod 装载。

## 3. `packages` 段与包装载面（`pkg.*`）

```json
"packages": [ { "id": "entity", "version": "0.1.0" }, { "id": "role-skill", "version": "0.1.0" } ]
```

`packages/libs/<id>/` 是第二个受控模块命名空间；包模块与本 Mod 的 `src.*` 进**同一张** `ModuleSet`（只是名字前缀不同），`require` 查表机制一个字不改。

| 面 | 规则 |
| --- | --- |
| 声明形状 | 段存在即**全量校验**：每项 `{id, version}`、两字段非空字符串、`id` 匹配 `^[a-z][a-z0-9_-]*$`、同一 `id` 不重复。段缺失或 `[]` → 零包，manifest 逐字节不变 |
| **完整闭包** | 本段是**唯一装载清单**；`pkg.json.deps` 只声明依赖关系（校验与判环），**不**把未声明的包拉进来——dep 未出现即 `MissingTransitive`。同形纪律：`code.modules` 也必须枚举每一个可 `require` 的模块，没有隐式发现面 |
| 版本 | **精确 pin**（字符串相等）；不符是**声明错**而非完整性错，文案报双方值。无 `semver` 依赖，范围语义等真实消费者再立 |
| id → 模块名 | `-` 折叠为 `_`：`role-skill` → `pkg.role_skill`（模块名段字符集只有 `[A-Za-z0-9_]`，折叠是唯一可行映射） |
| 文件 → 模块名 | 相对包根去 `.lua` 按 `/` 拆段，复用 `ygg_modules::name_of_path`：`init.lua` → `pkg.<id>`；`words.lua` → `pkg.<id>.words`；`a/b.lua` → `pkg.<id>.a.b` |
| 允许的文件 | `pkg.json` + 非 `test/**` 的 `*.lua`；其它文件打包期可读拒绝；`.` 开头条目静默跳过；`test/**` 是开发面（不进模块表、不进 manifest） |
| 包根 | `--packages` / `$YGG_PACKAGES_ROOT` > `<mod>/../../packages/libs`；不可读**即失败**并点名推导值与两个来源，不静默降级为零包。包根是 **host 环境态**而非 run 身份态，故本段只写 id/version 不写路径；**内容**由 manifest sha256 钉死 |
| 内容寻址 | `manifest.json` 的 `packages: [{id, files[]}]` 逐文件记 `{path, sha256, byteLength}`，`path` 相对包根、以包 id 打头；纳管 = 全部非 `test/**` 文件 + `pkg.json`，按 `(id, path)` 排序（重算幂等） |
| 预算 | 模块数与代码总量按 **mod ∪ 包** 的并集统一判（`MAX_MODULES=128` / 1 MiB） |
| 纯度与 KV | 包模块走同一装载器，`ShadowBanned` 影子表自动生效（包模块顶层 `ygg.kv.set` / `ygg.on` 一律拒绝）。KV 前缀：包只写自己契约 §2 声明的前缀（单写者，装载期不校验）。`pkg.<id>.*` 是**模块命名空间**，不是 KV 前缀 |

**失败面**（全在进入内核之前、零副作用）：段非数组 / 项非对象或缺字段 / id 非法 / 重复 id / 包不存在（报出根下实际有哪些）/ 版本不符（报双方值）/ 缺传递依赖 / 循环（链式文案）/ `pkg.json` 坏（先点名哪个包）/ manifest 缺段或字节漂移 / 并集超预算。

**诊断面**：`require("pkg.fixture")` 的 chunk 名是**相对包根的真实路径**（`@fixture/init.lua`），不是 `path_of` 会推出的 `pkg/fixture.lua`——否则 traceback 指一个不存在的文件。

**为什么不挂 `package.path`**：沙箱里**没有** `package` 全局（白名单测试钉死），造一个还会把解析顺序交给 mod 可变状态（破确定性）。两个载体两件实事：沙箱 = 模块表注入；普通 Lua（`selftest` / `ygg-lint` 静态面）= `package.searchers` 追加项。两者读**同一份**包索引（`模块名 → 相对包根路径`），结构上不可能漂移。

## 4. 多文件纯度契约

- `code.{entry,modules[]}` 显式枚举全部 Lua 模块；**只有 `mod.lua` 可在顶层注册处理器或改变状态**；`src/**` 导出纯函数（`M.install(deps)`）并带真实 `@src/<file>.lua:line` 诊断。
- 模块顶层禁止：`ygg.on` / `ygg.emit` / `ygg.narrate` / `ygg.choice` / `ygg.meta` / `ygg.kv.set` / `ygg.entity.spawn|set_attribute|add_tag|remove_tag` / `ygg.fsm.define` / `ygg.narrative.node` / `ygg.narrative.goto`（静态 `YGG-MOD-EFFECT` 与运行时影子表共用同一份 `ygg_modules::effects` 清单）；顶层只允许纯读（`ygg.data` / `ygg.ticks` / `ygg.kv.get` / `ygg.entity.has_tag` / `ygg.entity.get_attribute`）；入口 chunk 豁免（它是装配清单）。
- **为什么**：handler 的 dispatch 序 = `ygg.on` 注册序；让模块自注册等于「文件怎么拆 / 谁先 require」决定运行时行为，拆文件或调 `require` 顺序就会改行为，重构不再保语义。`require` 可在任意时刻（含 handler 内惰性加载纯库），但惰性加载**不放宽**边界。
- **文件拆分上限**：源码文件 800 行上限（超限触发按面拆分，如 `host_contract/` 七文件、`packages/` 六文件——纯搬移，pub 路径与测试名逐字节守恒，五门禁计数 before/after 相等）。
- **KV 前缀归属**（文档纪律，装载期不校验）：`fsm.*` 只由 stdlib `ygg.fsm` 写、Mod 只读；`settings.*` 只由 `config.set` 写；`req.*` = 需求表；`prop.*` = 提案表；`lane.*` = 泳道台账；`ui.*` = 纯呈现（不进 summary、不影响 dispatch）；`task.*` / `verify.*` / `review.*` / `inbox.*` 与裸键 `ending` 各有单写者。两模块写一前缀 = 隐式耦合，应经模块返回值共享。
- **错误可定位**：不得「把多文件 Mod 粘回一个文件再报错」——模块 chunk 名必须带真实文件（`@src/core/util.lua`），装载器自身决定的拒绝必须带调用点（`src/a.lua:3: <reason>`）与 require 链，静态检查逐模块独立解析（模块 `local` 不跨 chunk）。
- **热重载**：单发模式每个新进程从盘加载；`open`（attach）回执带 `hotReload.source="resume"` 才证明改动生效。
- **两条 mod 路径是一个 Mod**：`.ygg/mods/agent-seed` 是唯一事实源（安装实例），`packages/mods/agent-seed` 是指向它的 git 跟踪相对软链（稳定入口，**不是独立副本**）。playbook 是装配不是复制：九个片段按固定顺序拼成 21 章，一个片段带 1–5 章，section id 全局唯一。

## 5. 工程命令（改 `data/` 或 `schemas/` 后必跑）

| 规则 | 命令 / 判据 |
| --- | --- |
| `mod.pack` | `cargo run --example pack -- .ygg/mods/agent-seed` |
| `mod.lint` | `cargo run -p ygg-lint -- --pack .ygg/mods/agent-seed`（整包模式，兼校 `code.modules` 声明）或单文件模式；必须 0 error / 0 warning |
| `mod.playtest` | `cargo run --example playtest -- .ygg/mods/agent-seed`（all-1 走查必须到 `[ENDING`）；分支与地图文案以 `gates.mod-semantics-matrix` 为准 |
| `mod.manifest-content-addressed` | 改 `data/*.json` 不重算 pack → 加载期 content-hash mismatch |
| `mod.multi-script-load` | 多文件只在 `code` 段登记；`entry` 不作为模块登记；`modules` 逐文件枚举（无 glob）；缺 `code` 段 = 单文件 Mod |
| `mod.pure-export` | 见 §4 |

## 6. pack 声明变更的连带义务

**因 `pack.json` 声明形状变化而必须改的宿主 `#[cfg(test)]` 断言（含 `crates/ygg-sandbox/examples/egg/` 的测试模块）属维护，不是能力扩张**：在同一 commit 改掉，并写明是哪一条声明逼它改。其余一律仍禁止——生产代码、新宿主能力、任何运行时行为变化。`packages` 段（§3）同样受本条约束。

**判据**：① 阻断——因改 pack 声明导致断言失败却不同批修断言，或借机改生产代码 / 新增宿主能力；② 症状——`pack.json` 改了而 `--example egg` 套件变红，或 commit 里同时出现断言改动与未声明的行为改动；③ 确认——`cargo test --example egg -p ygg-sandbox` 绿，且 commit message 点名逼它改的那条声明。

## 7. Playbook 消费契约与文案卫生

`src/view/guide.lua` 的 `STAGE_GUIDE` 必须覆盖装配后的**全部章 id**；每个相位渲染播出**全部候选章**，不是只播首章。`map.show` 保持单一操作视图且忽略 payload，但在 `render()` 产生 choice 之前额外播出 21 章全局索引——因此当前相位章在一次 `map.show` 回执里出现两次，其余章一次。退役幕同样播 `pointers`，不留只在全局索引可见的死角。增删章、改章 id/title、改映射或渲染序必须在同批更新静态章集合相等断言与逐相位回执探针，并重算 manifest。

**文案路径卫生**：`data/texts/**` 不得含退休路径字面量 `ygg-agent` 或 `pi-agent-seed`，必须用当前 `agent-seed` / `egg`；确实需要历史引用时该行须带字面标记 `[deprecated-ref]`。仓库根运行、空输出为通过：

```bash
grep -rn 'ygg-agent\|pi-agent-seed' .ygg/mods/agent-seed/data/texts/ | grep -v '\[deprecated-ref\]'
```

机检 = `selftest::mod_text_fragments_carry_no_retired_paths`。违规文案会把执行者导向 fail-closed 路径。

## 8. host 契约四段（ownership 边界）

- **`contract.required`**：通用工具（`src/playtest.rs`）**不硬编码**任何事件名 / 叙事前缀 / summary 键 / 入口载荷 / 收件箱字段 / 结算开关 / 推进阈值，一律读已装载 Mod 的契约；契约缺失 / 字段缺失 / 类型错 / 非正整数 → 加载失败（fail-closed）。
- **`contract.agent-seg`** / **`contract.inbox-seg`**：见 §1.1 的 `agent`（8 字段）与 `inboxRoutes`（保留键 `externalIdField` / `payloadField` / `extMarker` + `<kind>` → `{action, fields[, write]}`，零 kind 合法）。
- **`contract.settlement-seg`** / **`contract.drive-seg`**：见 §1.2 的 `settlement`（`requireEvents` / `requireNoFailedSteps` / `requireEnding` / `requireInboxConsumed`）与 `drive`（`maxSteps` / `stallLimit` / `stepsPerRequirement`）。
- **`contract.config-path`**：`<config.json>` 是 `pack.json` 里 `ygg:agent/config` 条目的路径（seed Mod = `data/config.json`）。
- **`contract.upgrade`**：升级旧 Mod = 补齐四段（两段在 `pack.json`、两段在 `config.json`），`cargo run --example pack -- <mod dir>` 在打包期校验形状（只校验声明了的）。
- **`contract.host-vs-contract`**：判据一句——**这个字面量会随换一个 Mod 而变吗？** 会 → 属契约，由 Mod 声明；不会 → 可留宿主字面量。宿主可留：run 目录文件名、协调相位名 `running`/`closing`/`finalized`、`DECIDE_TIMEOUT` / `DECIDE_POLL_INTERVAL`、三种 `--drive` 模式、五文件集（`pack.json` / `manifest.json` / `mod.lua` / `schemas/` / `data/`）、内核接缝（`save` / `load` / `state` 与保留路由 `ygg.choice_selected`）。
- **`contract.literal-ownership`**：只有绑定特定 Mod 的应用与生成器侧可留事件名 / 结尾前缀字面量，且每个文件头声明 "bound" 或 "generator-side"（`adventure.rs`、`clients/godot/**`、`packages/client-web/src/{kernel,session,kernel-wasm}.ts`、`crates/ygg-wasm/smoke.mjs` 绑 `packages/mods/test-adventure`；`packages/pipeline/src/{prompt,types,pack}.ts`、`scripts-dev/mock-llm-stub.mjs` 生成器侧）；通用工具（`crates/ygg-sandbox` / `ygg-lint`）与 `tests/` 夹具**不得新增字面量**。
- **`contract.choice-payload-type`**：`ygg.choice_selected` 的 payload 必须是 **JSON 数字**（`--payload '1'`）；传字符串（`--payload '"1"'`）被内核判 VALIDATION「option does not exist」。菜单选择是人机门禁的**唯一**通过通道——直发 `proposal.accept` / `gate.approve` / `coordinate.submit` / `verify.submit` / `review.note` 一律拒绝（回执首行 `[REJECTED:ACTION]`）。
- **`contract.serve-read-only`**：正文权威在 [../host/serve.md](../host/serve.md) §3.7.6——绝不给 serve 加 run-state 写端点。
