# pi-agent-seed · 种子模板（双层状态机 + 提案验收的通用元工作流）

**这是对外分发的种子模板，不是某个仓库的实例。** 它把「一次完整的 Agent 工作」配置成一个可游玩的
Mod：协调器派活 / 验收，每条需求各自跑 analyze → plan → 门禁 → develop → 交付 → 门禁。目标形态见
`docs/vision.md` 的「冷启动 seed mod：通用元工作流，新 Agent 零定制接入」——**换成任何一个新项目，
只需要填项目章节与配置，机制一行都不用改**。

本文件面向「我要拿它开一个新项目」；机制细节（状态机 / 动作协议 / inbox / 结算 / host 契约）只看
本文件与仓库根 `AGENTS.md`。

---

## 零、哪些文件该改、哪些不该改

| 文件 | 面 | 新项目要不要改 |
| --- | --- | --- |
| `mod.lua` | **骨架**（装配入口） | 不改。只读资源、require 模块、按序 install，不装业务逻辑 |
| `src/**/*.lua` | **骨架**（机制实现） | 不改。12 个纯导出 + 10 个注册模块；改机制回种子模板改，再同步 |
| `schemas/*.json` | **骨架**（资源契约） | 不改。改资源格式属于机制变更，回种子模板改 |
| `pack.json` | **骨架 + 实例** | `code` 段（entry + modules）与 `tools` 段（能力声明）不改；`entries` / `schemaFiles` 结构固定；`agent` / `inboxRoutes` 按本项目实际文案与动作集填 |
| `data/playbook.json` | **实例**（现场规则） | **必改**：五章通用纪律可原样留，另加本项目章节 |
| `data/config.json` | **实例**（静态默认） | **必改**：`drive` 阈值、`settlement` 口径、`play.mode` 默认 |
| `data/scenes.json` | **实例**（叙事文案） | 可改：换世界观只改这里，不碰 `mod.lua` |
| `manifest.json` | 生成物 | 永不手改（`pack` 现算 sha256 + byteLength） |
| `README.md` | 文档 | 改：换成你项目的接入说明 |

**仓库内的两份 Mod 不是同一份东西**：

```text
packages/mods/pi-agent-seed/   种子模板（本目录）——通用，对外分发
.ygg/mods/pi-agent-seed/       本项目实例     ——playbook / config 装的是本仓库自己的规则
```

- **骨架面逐字节一致**（`mod.lua` + `src/**` + `schemas/`，以及 `pack.json` 的 `code` 段），
  由 `scripts/sync-seed-mod.sh` 单向同步，并被 CI 的骨架守卫拦（不一致即失败）。
- **实例面必然不同**（`data/`，以及 `pack.json` 的 `agent` / `inboxRoutes`——前缀集必须与各自
  `data/scenes.json` 的实际用词一致），任何同步都会毁掉其中一侧，所以脚本不碰它们、CI 也不校验。

---

## 一、当新项目起点：四步

### 1. 复制

```bash
mkdir -p <目标仓>/.ygg/mods
cp -R packages/mods/pi-agent-seed <目标仓>/.ygg/mods/pi-agent-seed
```

把 `.ygg/bin/`（CLI 可执行）与 `.ygg/runs/`（本局产物）加进 `.gitignore`，`.ygg/mods/` 入库。
之后所有命令的 `--mod` 都指向这个目录。

### 2. 填 `data/playbook.json` 的项目章节

五章通用纪律（`credentials` / `dispatch` / `evidence` / `output` / `harness`）**可原样沿用**——
它们是元工作流纪律，与具体仓库无关。本项目的规则另起章节加进去，常见的六章：

| 建议 id | 装什么 |
| --- | --- |
| `repository` | 仓库形态：目录 → 职责（schema 允许 `layout` 数组直接渲染成树） |
| `branches` / `lanes` | 分支纪律与工作区（泳道）挂牌规则 |
| `integration` | 开发 → 集成 → 合并 → 部署的流程与审批点 |
| `gates` | 验证命令门禁（schema 允许 `commands` 数组，可原样拷进终端） |
| `pointers` | 「想做什么 → 去哪看」（只放指针，不复述被指向文档的正文） |
| `hard_constraints` | 「不要做的事」清单（索引式，指向承载细节的章节） |

三条纪律：

- **只写本项目真正独有的约束**。通用纪律已经在前五章，抄一遍只会两处漂移。
- `id` 是取用键：`guide.show` 按 section id 取章节，**项目章节不要占用前五章的 id**。
- 把 `source.document` / `source.revision` 改成你自己的文档与修订日期（schema 要求 `YYYY-MM-DD`）。

### 3. 改 `data/config.json`

| 段 | 要定的东西 |
| --- | --- |
| `play.mode` | `guide`（逐步确认，稳妥）还是 `goal`（挂机，快） |
| `drive` | `maxSteps`（基础步数硬顶）/ `stallLimit`（连续无进展判停）/ `stepsPerRequirement`（每条需求追加步数），三项必须是正整数 |
| `settlement` | 四个开关：事件日志非空 / 无失败步 / 出现结局行 / inbox 已消费 |
| `evolution` | **仅叙事偏好**：`off` / `suggest` / `auto` 只影响建议口径的旁白；本 Mod 不自动修订、不登记修订请求、不参与任何判定（`max_per_run` 只是标注值）。局末进化声明走独立动作 `settlement.evolution`，与这三项无关 |
| `gates.require_all_green` | 门禁失败是否打回 |

### 4. 跑三件套（缺一不算接入完成）

```bash
mod=.ygg/mods/pi-agent-seed
cargo run --example pack -- "$mod"              # ① 重算 manifest（内容寻址）
cargo run -p ygg-lint -- "$mod/mod.lua"         # ② 静态检查：应 0 error / 0 warning
cargo run --example playtest -- "$mod"          # ③ 冒烟：必须 "ok": true + "reached": "ending"
```

三件套的判据是**报告内容**而不是退出码观感：`playtest` 打出 `"ok": true` 且 `"reached": "ending"`
才叫跑通；`warnings` 非空要逐条看清楚再收工。

---

## 二、host 契约（必填；宿主不含任何默认值）

宿主（`--example agent`、`--example playtest`）**不含任何事件名字面量与回退默认值**：事件名、inbox
路由、结算口径、推进阈值全部从本 Mod 读。缺段 / 缺字段 / 类型不符 / 非正整数 → **装载期失败**
（非零退出或 `ok=false` 报告），错误文案点名缺失字段。这是 fail-closed 的刻意选择：宿主宁可拒绝启动，
也不猜 Mod 的意图。

### 1. `pack.json` → `agent`（八个字段全部必填）

```json
"agent": {
  "entryEvent": "event.start",
  "choiceEvent": "ygg.choice_selected",
  "summaryEvent": "agent.summary",
  "endingPrefix": "【结局",
  "metaKey": "summary",
  "rejectionPrefixes": ["【动作被拒】", "【载荷被拒】", "【需求待办】"],
  "replayPrefix": "【需求重放】",
  "defaultPayload": "0"
}
```

| 字段 | 含义 | 宿主用途 |
| --- | --- | --- |
| `entryEvent` | 新局入口事件 | 交互模式开局自动派发；`--event` 省略时的缺省 |
| `choiceEvent` | 玩家选择事件 | 自动推进每步派发；stdin 数字输入 |
| `summaryEvent` | Mod 自报摘要的只读动作 | 自动推进的唯一判定面 |
| `endingPrefix` | 结局叙事行前缀 | 收口判据（回执 `ending` / 事件日志 / 结算） |
| `metaKey` | 摘要结构化字段在回执信封 `meta` 里的键名 | 宿主只读 `meta.<metaKey>`，不读 Mod 私有 KV |
| `rejectionPrefixes` | 「动作未被接受」的叙事行前缀集合（≥1 条） | 宿主据此判定**未生效**（不推 cursor、不算成功） |
| `replayPrefix` | 「幂等重放」的叙事行前缀 | 宿主据此判定重放（算成功消费，只是没新增） |
| `defaultPayload` | 入口事件首次派发的载荷（JSON 标量文本；可空串） | 开局派发 / `--payload` 缺省 |

- `choiceEvent` 必须与**内核保留路由**同名（`ygg.choice_selected`）：内核只对该事件名做
  「序号在 `1..=选项数` 内」的域校验，改名即失去这层校验。
- `rejectionPrefixes` 必须是 **Mod 真实在用的叙事前缀**：写错了宿主就把被拒派发当成已生效。

### 2. `pack.json` → `inboxRoutes`（必填；三个保留键与 kind 同层）

```json
"inboxRoutes": {
  "externalIdField": "external_id",
  "payloadField": "payload",
  "extMarker": "@ext=",
  "add": {
    "action": "requirement.add",
    "fields": ["title", "description", "external_id", "depends_on"],
    "write": { "flag": "requirement", "shape": "named" }
  }
}
```

工具结果走**同一条** inbox 通道（kind 由 Mod 声明），所以 `tool_result` 只是一条普通路由：

```json
"tool_result": {
  "action": "tool.result",
  "fields": ["call_id", "tool_id", "status", "code", "output", "external_id"],
  "write": { "flag": "tool-result", "shape": "payload" }
}
```

| 键 | 必填 | 含义 |
| --- | --- | --- |
| `externalIdField` | 是 | inbox 记录里**幂等键**的字段名（也用于 `fields` 里标记幂等键段） |
| `payloadField` | 是 | inbox 记录里**预成形载荷**的字段名（整串 wire，宿主不解析） |
| `extMarker` | 是 | 幂等键拼到预成形载荷末尾时的**后缀标记** |
| 路由键（`add` …） | 是 | inbox 行的 `kind` 字面值（**不得与三个保留键同名**） |
| `action` | 是 | 派发的内核动作名 |
| `fields` | 是 | wire format 的段序：按此序从记录取值、用竖线连接；值等于 `externalIdField` 的段填幂等键 |
| `write.flag` | 否 | 宿主 CLI 旗标名（`--requirement` → `requirement`）；不写则宿主不提供该写命令 |
| `write.shape` | 否 | `named`（前两个非幂等键段依次填「首段 / 余下」）或 `payload`（整串进 `payloadField`） |

三个保留键**必填**，但**允许一个 kind 都不声明**（= 本 Mod 不支持异步 inbox，写命令退出码 3）。

### 3. `data/config.json` → `settlement` / `drive`（必填）

```json
"settlement": {
  "requireEvents": true,
  "requireNoFailedSteps": true,
  "requireEnding": true,
  "requireInboxConsumed": true
},
"drive": { "maxSteps": 64, "stallLimit": 12, "stepsPerRequirement": 16 }
```

`settlement` 关掉的项不参与结算 `ok`（但仍逐项记入 `checks`）；`drive` 三项必须正整数。

### 4. `data/config.json` → `allowedTools`（可选；**两层白名单的 Layer 2**）

```json
"allowedTools": ["git.head_short", "git.status"]
```

- 缺失 / 非数组 / 元素非法 = **一个也不放行**（fail-closed：抄一份 Mod 不会突然能跑 `git`）。
- 必须 **⊇** `pack.json` 的 `tools` 键（宿主装载期硬校验，缺一个即拒绝装载）。
- Mod **读不到** `pack.json`（`ygg.data` 只读 `entries` 声明的资源），所以 Mod 只认这一层；跨层一致性由
  `agent_seed_semantics` 的声明一致性用例钉住（两侧 `tools` 深相等、`allowedTools` 与 `tools` 同集合、
  `tool_*` beat 齐全）。
- **不是**安全边界，而是**防漂移**门禁：能改 `data/config.json` 的人也能改 `pack.json`。真正的边界是
  宿主的 Layer 1 校验（程序白名单 / 无 shell / 无绝对路径 / 无 `..`）加上沙箱本身没有 IO。

---

## 三、`data/` 三份资源各管什么

| 文件 | 作用 | 改动后果 |
| --- | --- | --- |
| `playbook.json` | 现场规则资源，`guide.show` 按 section id 取用，按阶段呈现 | 改完只是叙事变，不动状态机 |
| `config.json` | 静态默认（下层），运行期 KV `settings.*` 是上层（`config.set` 写入），判定恒为「动态覆盖 > 静态默认」 | 改 `drive` / `settlement` / `allowedTools` 会影响宿主如何推进、结算与放行 |
| `scenes.json` | 全部文案与调参（scenes / endings / prompts / options / beats / tuning） | 换世界观只改这里 |

### `settlement.evolution` 登记与 `review.note` 归档（骨架职责，接入时不要改）

```text
settlement.evolution "artifact|summary[|c=<kept>/<screened>[|cls=<kind>:<n>,...]]"
review.note "lesson|score"
```

| 段 | 形状 | 缺省 | 归档到 |
| --- | --- | --- | --- |
| `artifact` | 可空；非空须为 `artifacts/<file>.md`，禁 `..` 与空白 | 空 | `review.evolution.artifact` |
| `summary` | 去空白后必填，不能含 `|` | 菜单用中性默认文案 | `review.evolution.summary` |
| `c` | 两个非负整数，`kept ≤ screened` | 先回落预登记，再 `0/0` | `review.evolution.candidates` |
| `cls` | `<kind>:<n>` 逗号分隔，kind ∈ `long_term` / `unused` / `one_off` / `temp_tool` / `redundant` / `no_scene` | 先回落预登记，再空 | `review.evolution.classes` |

- **每轮强制登记**：只在 review 且 `verify.passed=true` 后可用；成功先写
  `settlement.evolution.{artifact,summary,candidates,classes}`。`c=0/0` 允许 artifact 为空，
  不为零候选伪造文档；`screened > 0` 时 artifact 与 `cls` 必填。
- **guide 预登记**：`config.set review.evolution.artifact|summary|candidates|classes=<值>` 写
  `settings.review.evolution.*`，只逐键校形状；summary 空值可清除，跨字段完整性只在
  `settlement.evolution` 校验。未预登记时菜单给空 artifact、中性 summary 与 `c=0/0`。
- **单源归档**：`review.note` 只接受 `lesson|score`，旧 `ev=` / `c=` / `cls=` 段一律拒绝并提示先登记；
  它只把本轮 settlement 四键复制到 `review.evolution.*`，不再从载荷或 settings 重算。
- **重开一轮**：`coordinate.submit` 重入 review、以及 `verify.submit=false` 打回时，清空本轮
  settlement 四键，但保留 settings 预登记。
- **声明 ≠ 证明**：沙箱没有文件系统（无 `io`/`os`），Mod 读不到 artifact、也不认识 subagent 会话，
  只校验声明形状与计数自洽；`agent.summary.evolution` 回显归档的 artifact / summary / candidates /
  classes 四键，随 `settlement.json` 自然可见。

### goal 授权与执行者账本（骨架职责，接入时不要改）

```text
config.set goal.authorization=<单行说明>                                  # run 级 goal 授权；空值 = 清除
config.set executor.<rN>.<analysis|plan|development>=<agent_id>[|<原因>]   # 执行者账本；空值 = 解绑
```

- **goal 授权**：落 `settings.goal.authorization`。只有**运行期进入 goal**（`settings.play.mode=goal`）的局在
  `review.note` 收口前校验（缺记录 → 旁白阻断）；静态 `data/config.json` 预设 goal 不在校验范围（仓库所有者常设配置，
  否则全 1 冒烟会断）；切回 guide 不校验，resume 保留。它只证明「登记过一句话」，不是身份认证，不进 `agent.summary`。
- **执行者账本**：落 `settings.executor.<rN>.<stage>`。同一 (req,stage) 换人与**跨阶段换人**（plan ≠ analysis、
  development ≠ plan 且无 plan 时比 analysis）都必须带原因；换人成功写 `id|原因` 并向 `req.<i>.hist` 追加一条
  （滚动上限沿用）；首次干净绑定不写 hist。账本不带 rev：`amend` 不变、`split`/`merge` 不继承。
  `proposal.submit` **不做硬校验**（外部提案与同 ext 重放照旧），未登记只在 `requirement.list` 显示「未登记」。
- **同为自声明**：两者都只证明主 Agent 登记了什么，不证明宿主身份或真实执行者。

---

## 四、骨架面不要按项目改

`mod.lua` / `src/` / `schemas/` 是**机制**：两个 Mod 之间必须逐字节一致，否则「同一个 harness」就有两种行为。
`mod.lua` 只是装配入口（资源快照 → require → install → `render.register()` → `fsm.define`），业务实现都在 `src/`。
改机制的正确路径：

```bash
# 1. 在种子模板侧改骨架
# 2. 推到项目实例侧（同时重算两侧 manifest）
scripts/sync-seed-mod.sh

# 反向（项目侧先改的，回流到模板）
scripts/sync-seed-mod.sh --from project

# 只同步文件、暂不重算 manifest
scripts/sync-seed-mod.sh --no-pack
```

脚本只碰 `mod.lua` + `src/**` + `schemas/`，并检查两侧 `pack.json` 的 `code` 段一致；
幂等（内容一致时不写文件），同步完自动重算**两侧** `manifest.json`。
CI 里有一道骨架守卫：两侧不一致直接失败并提示运行该脚本——所以「只改一侧」在合入前一定会被抓到。

---

## 五、模块结构（原生多文件）

`pack.json` 的 `code` 段声明入口与全部模块，宿主按 `require` **原生装载**——没有构建期拼接、
没有 build 脚本、也没有单文件产物。`mod.lua` 是装配清单，不是 Mod 本体。

依赖方向（左 → 右；`require` 只向下，无环）：

```text
util ← config ← requirements ← lanes ← proposals ← gates ← guide / menus ← render
     ← wire ← flow ← handlers/*
```

`src/tools.lua` 是叶子：require `config` / `flow` / `hooks` / `render` / `requirements` / `util` / `wire`，
**没有任何模块 require 它**（反向也不行：`render.lua` 若要工具计数只能读 KV `tool.pending`，否则立刻成环）。

| 模块 | 职责 |
| --- | --- |
| `src/util.lua` | 纯函数（fill / split / cut / trim / join / contains / to_bool / spaces），无 install |
| `src/config.lua` | 两层配置（`settings.*` > `data/config.json`）+ `PLAY_MODES` / 标量读取 |
| `src/requirements.lua` | 阶段常量与 `stage()`、需求表 `req.*`、依赖校验、摘牌唯一出口 `release` |
| `src/lanes.lua` | 泳道账本 `lane.<k>.*` + job / 容量计数 |
| `src/proposals.lua` | 提案表 `prop.*` + `changes`（split / merge / amend / conflict）落地规则 |
| `src/gates.lua` | 门禁与收口判据（只读选择器） |
| `src/guide.lua` | 现场规则章节索引与渲染（`guide.show <section_id>`） |
| `src/menus.lua` | 阶段菜单 + 配置二级页（选项文本 ↔ 动作载荷的唯一来源） |
| `src/render.lua` | 回执渲染 + 双层图（local/full）+ 结构化摘要（`agent.summary` 的注册面） |
| `src/wire.lua` | payload wire format + inbox 幂等键（`@ext=`） |
| `src/tools.lua` | 工具桥的 KV 面与回执面（`tool.*` 唯一写方）+ `tool.call` / `tool.result` 两个 handler + 回执 effect |
| `src/flow.lua` | 转场 / 拒绝的唯二出口 + 状态机票据（`stage_ticket` 与用它的函数同模块） |
| `src/handlers/*.lua` | 10 个注册模块：actions / lanes / proposals / review / requirement_ops / gate_actions / config / views / choice / lifecycle |

三条硬规则：

1. **模块顶层纯导出**。只定义函数与常量：不注册（`ygg.on` / `ygg.fsm.define` / `ygg.narrative.node`）、
   不写状态（`kv.set` / `entity.*`）、不输出（`narrate` / `emit` / `choice` / `meta`）。所有 ygg 调用都在
   函数体内。`ygg-lint --pack` 的 `YGG-MOD-EFFECT` 就是这条的静态门。
2. **派发序 = 入口的 `install()` 调用序**，既不是 `require` 序，也不是 `pack.json` 的 `code.modules` 声明序。
   入口里那 10 行 `handlers.<x>.install()` 的先后就是 handler 的注册序；调整顺序等于改运行时行为。
   `agent.summary` 由 `render.register()` 注册，入口把它排在所有 handler 之前（与拆分前逐点一致）。
3. **改任何模块都必须重跑 `cargo run -p ygg-sandbox --example pack -- <MOD>`**。manifest 的 `code` 数组是
   内容寻址的（逐模块 sha256 + byteLength），不重算会按字节数 / 哈希拒绝装载。

依赖注入：模块不在顶层读资源，入口把 `TEXTS` / `CONFIG` / `PLAYBOOK` 注入各自的
`install(deps)`。`requirements` 要清泳道账本而 `lanes` 又要读需求，为避免 require 环，
`lanes` 的 `LANES` 表由入口注入（`reqs.install{ ..., lanes = lanes.LANES }`）。

---

## 六、已知边界

- **单仓版**：泳道账本是 `lane.<k>.*`，没有 `repo` 层（多仓版另有分层，不在本模板内）。
- **`pack` 不校验资源内容**：它只校验 `pack.json` 结构、`schemaFiles` 存在性与内容哈希；
  `data/*.json` 是否合 schema 由**装载期**校验。所以改了资源要跑 `playtest`，光跑 `pack` 不够。
- **菜单不变式（骨架级，接入时不要破）**：review 在 verify 已通过且未登记时，首项是
  `settlement.evolution`；登记后首项才是 `review.note`（宿主 `nextEndsRun` 契约保持不变）。
  未预登记时的空 artifact + 中性 summary + `c=0/0` 默认载荷，保证 `playtest` 恒选 1 能如实走
  「零候选登记 → review.note」；骨架菜单没有 `enabled` 概念，不引入 disabled 项。
- **按阶段提示是 `src/guide.lua` 里写死的映射**（intake → `output` / coordinate → `dispatch` /
  review → `evolution` / done → `pointers`）：项目的 playbook 没有对应章节时，回落到第一章的标题与摘要。
  这是优雅降级，不是错误——但如果你希望 review / done 阶段提示自己的章节，就补上
  `evolution` / `pointers` 这两章（种子模板自带 `evolution` 章，可直接沿用）。
- **没有规则闸**：不设「已读规则」标记，也不做「未读规则即拒绝」——本模板里没有任何「先读才放行」的前件。
  规则提示每步一行，要全文就 `guide.show <section_id>`。
- **`map.show` 两个视图**：默认只出「当前节点 + 出边」（剧情为点、选择为边），`map.show full` 才出设计全图。
- **节点文案只有一段**（`<节点>_common`）：goal / guide 的差异由 `play_receipt` 每条回执报一次，
  不在每个节点重复。
- **工具桥不设人类菜单项**：`tool.call` / `tool.result` 是**机器动作**（外部派发），不是人类里程碑动作。
  往菜单里插一条会动 `summary_next_ends_run`（它读阶段菜单首项是否 `review.note`），一旦插到首位就
  **静默破坏收口判定**。同理不引入第 5 个结算门禁（会让所有既有 Mod 的 `config.json` 失效），
  工具债务只走 `meta.summary.pendingTools` 回显。

---

## 七、工具桥（可选能力：`tool.call` / `tool.result`）

Mod **永不做 IO**（沙箱无 `io` / `os`，也没有 process / 网络 API）：一个工具调用是**请求**，不是副作用。

```text
① Mod 登记请求：ygg.meta("tool_calls", { { id = "t1", tool = "git.head_short", args = { rev = "HEAD" } } })
② 宿主执行（Layer 1 形状 + Layer 2 放行），把结果作为**普通事件**回投
③ tool.result "t1|git.head_short|ok|0|abc1234|ext-1"
```

- **请求 wire**（`tool.call`）：`<tool_id>[|<k=v;...>][|<call_id>]`（位次固定；只要 call_id 就写 `git.status||t2`）。
- **结果 wire**（`tool.result`）：`<call_id>|<tool_id>|<ok|err>|<code>|<output>[|<external_id>]`
  （5 或 6 段；也认 `|@ext=` 后缀形态）。`output` 内不得含竖线，CR/LF 折成 ` / `，超 `tuning.tool_output_max`
  截断并附 `…[truncated N chars]`。
- **`meta.tool_calls`**：稠密数组、序 = 分配序、只列 `pending` 行；无参数时**省略** `args` 键（Lua 空表
  只能序列化成 `[]`）。终态后为 `[]`（不是缺键）。它与 `meta.summary` 是 `ygg.meta` 的两个独立槽位。
- **幂等**：同一 `external_id` 重投 = `[REPLAY:TOOL]`，不改状态、不减计数、不重出 `[TOOL SETTLED]`；
  一个 call 只有一个终态（换状态是冲突，不静默覆盖）。
- **两个执行场景**（宿主侧，Mod 看到的形状完全相同）：
  - **场景 1（同进程内回投）**：`inlineWaitMs > 0` 且 worker 在窗口内写完结果 → 父进程在**同一次派发**里
    就地回投 `tool.result`（用户可见：一步之内看到 `[TOOL RESULT]` + `[TOOL SETTLED]`）。
  - **场景 2（异步 inbox 回写）**：`inlineWaitMs = 0` 或超时 → 回执标 `detached`，worker 完成后把同样一段
    wire 写进 inbox（`tool_result` 路由）→ 下一次 drain 把它作为普通事件回投（用户可见：多一个回合）。
- **凭据红线**：`pack.json` 的 `args` / `headers` 可写 `$ENV{VAR}`，宿主按
  `进程环境 > <mod_dir>/.env.local > <mod_dir>/.env` 解析；解析后的值**绝不**进载荷、回执或 notice
  （报错只出现变量名）。Mod 资源、配置与报告只出现**键名**，`.env*` 必须在 `.gitignore` 里。
- **可调项**：`tuning.tool_capacity`（生命周期上限，缺省 64）、`tuning.tool_output_max`（缺省 4096，
  必须 ≥ `pack.json` 里最大的 `maxOutputBytes`）、`tuning.artifact_capacity`（artifact 账本上限，缺省 64）。

### 七之二、响应式后置效应（可选能力：`tools.reactive`）

`tool.result` 落库为 `ok` + `code 0` 时，第二条 effect（`hooks.useEffect("tools.reactive")`）把 `output`
（`k=v;k=v` 平坦映射）**折叠成领域状态**；回执行（`tools.receipt`）仍只说「结果到了」。两条同处一次
`flush_effects`（同回合、产出 choice 之前），卿锁 `tool.reacted.<call_id>` 保证一生只折叠一次。

通用形状，与本实例无关：

- **注册面**：`REACTORS[tool_id] = function(pairs, call_id) -> changed`。不认识的 tool id **不占卿锁、不产生 deps**。
  新增一个反应器只改一个表项，不动 effect 本体。
- **可写面**：`lane.<k>.*`（与 `lane.claim` 共用 `LANES`）、`req.<i>.conflict{,_peer,_note,_source}`（必须经
  `props.req_do_conflict`，它是冲突的唯一写方）、`artifact.*`（必须经 `record_artifact`，它是账本的唯一写点）、
  `verify.{passed,report,source}`、`settlement.sessions.*`。
- **禁写面**：不 `render()`、不 `ygg.choice`、不改 stage、不改 proposal / gate 状态、不调 `flow.refuse`。
  拒绝行会被宿主记成失败步——「探针失败」是事实报告，所以走纯叙事；写盘类动作才走事件 handler + `reject_payload`。
- **截断判据**：`parse_result` 把 `truncated` 作为**布尔字段**返回，并落 `tool.<call_id>.truncated`；
  任何集合比对（如文件交集）必须先看它，宁可漏报不可误报。
- **VFS 面**：Mod 仍不做 IO。写盘一律委托 `ygg.fs`（若宿主提供），调用前先用路径形状守门（`artifacts/*.md`，
  无 `..`、无空白、≤128 B）；宿主没提供 `ygg.fs` 时**不假装写过**（叙事 `[VFS LEDGER ONLY]`），
  但账本照记，所以 KV 与叙事在两种宿主下都言行一致。

把这条能力引入新项目时，只需两件事：`pack.json` 的 `tools` 段声明工具（`allowedTools` 同步），
以及 `data/scenes.json` 补齐 `src/` 引用的 beat 名（`agent_seed_semantics` 的声明完整性用例会逐条比对）。

### 七之三、上层扩展（项目侧 `.pi/extensions/ygg-pi-harness`）

骨架包本身**不含**扩展：`packages/mods/pi-agent-seed` 是五件套模板（`pack.json` / `manifest.json` /
`mod.lua` / `schemas/` / `data/` + `src/`），扩展属于**装载它的项目**，随项目仓库走。项目实例
`.ygg/mods/pi-agent-seed` 的 README「十三」章记录了本仓库这一份的装配；这里只写**契约**：

Mod 对上层扩展的承诺（扩展可以依赖的，恰好只有这些）：

- 状态推进只有一条路：`daemon.dispatch(op="dispatch", event, payload)` → `ygg.on(event)` handler。
  扩展不需要（也不应该）读写 `.ygg/runs/**`。
- 只读处境只有一个面：`agent.summary` 的 `meta.summary`（`stage` / `ending` / `verifyPassed` /
  `nextEndsRun` / `pendingProposals` / `requirementCount` / `conflicts` / `pendingTools`，以及 add-on 的
  `jobs` / `gates` 与 `evolution` 四键）。**缺字段时必须取保守值**（不猜 jobs / gates）。
- 工具桥只认 `tool.call` 的 `<tool_id>|<k=v;…>`：扩展不直接执行工具，它只声明请求；执行在宿主，
  结果以 `tool.result` 回投，再由 `REACTORS` 折叠成领域状态。
- 人类动作门禁（`play.mode=guide`）对扩展**同样生效**：`proposal.accept` / `gate.approve` /
  `coordinate.submit` / `verify.submit` / `review.note` 直派会被 `[REJECTED:ACTION]` 拒。
  扩展要推进就把 `play.mode=goal`（`config.set`，允许直派且有 handover 叙事）显式登记后再走。

Mod 对扩展的要求（越权即错）：

- **不得**把 `gate.approve` 当成流水线的一步：门禁是「有人看过」的物理证据，自动批等于没有门禁。
- **不得**批量刷事件：`--events` / shell 循环 / `xargs` 属于永久禁令，扩展里的 `firewall.ts` 就拦这条。
- 回执 `narrative` **整段**呈现（禁止解析式截断）；`pendingChoice` 原样交给用户。
