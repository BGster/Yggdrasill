# agent-seed · 种子模板（双层状态机 + 提案验收的通用元工作流）

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
| `mod.lua` | **骨架**（机制） | 不改。它只读资源、不读项目语义 |
| `schemas/*.json` | **骨架**（资源契约） | 不改。改资源格式属于机制变更，回种子模板改 |
| `pack.json` | **骨架**（声明结构） | 不放宽不改：`entries` / `schemaFiles` / `agent` / `inboxRoutes` 四段结构固定 |
| `data/playbook.json` | **实例**（现场规则） | **必改**：五章通用纪律可原样留，另加本项目章节 |
| `data/config.json` | **实例**（静态默认） | **必改**：`drive` 阈值、`settlement` 口径、`play.mode` 默认 |
| `data/scenes.json` | **实例**（叙事文案） | 可改：换世界观只改这里，不碰 `mod.lua` |
| `manifest.json` | 生成物 | 永不手改（`pack` 现算 sha256 + byteLength） |
| `README.md` | 文档 | 改：换成你项目的接入说明 |

**仓库内的两份 Mod 不是同一份东西**：

```text
packages/mods/agent-seed/   种子模板（本目录）——通用，对外分发
.ygg/mods/agent-seed/       本项目实例     ——playbook / config 装的是本仓库自己的规则
```

- **骨架面逐字节一致**（`mod.lua` + `schemas/`），由 `scripts/sync-seed-mod.sh` 单向同步，
  并被 CI 的骨架守卫拦（不一致即失败）。
- **实例面必然不同**（`data/`），任何同步都会毁掉其中一侧，所以脚本不碰 `data/`、CI 也不校验它。

---

## 一、当新项目起点：四步

### 1. 复制

```bash
mkdir -p <目标仓>/.ygg/mods
cp -R packages/mods/agent-seed <目标仓>/.ygg/mods/agent-seed
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
| `evolution` | 局末复盘口径：`off` 只报告 / `suggest` 出建议包 / `auto` 白名单内自动修订 + `max_per_run` |
| `gates.require_all_green` | 门禁失败是否打回 |

### 4. 跑三件套（缺一不算接入完成）

```bash
mod=.ygg/mods/agent-seed
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

---

## 三、`data/` 三份资源各管什么

| 文件 | 作用 | 改动后果 |
| --- | --- | --- |
| `playbook.json` | 现场规则资源，`guide.show` 按 section id 取用，按阶段呈现 | 改完只是叙事变，不动状态机 |
| `config.json` | 静态默认（下层），运行期 KV `settings.*` 是上层（`config.set` 写入），判定恒为「动态覆盖 > 静态默认」 | 改 `drive` / `settlement` 会影响宿主如何推进与结算 |
| `scenes.json` | 全部文案与调参（scenes / endings / prompts / options / beats / tuning） | 换世界观只改这里 |

---

## 四、骨架面不要按项目改

`mod.lua` / `schemas/` 是**机制**：两个 Mod 之间必须逐字节一致，否则「同一个 harness」就有两种行为。
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

脚本只碰 `mod.lua` + `schemas/`，幂等（内容一致时不写文件），同步完自动重算**两侧** `manifest.json`。
CI 里有一道骨架守卫：两侧不一致直接失败并提示运行该脚本——所以「只改一侧」在合入前一定会被抓到。

---

## 五、已知边界

- **单仓版**：泳道账本是 `lane.<k>.*`，没有 `repo` 层（多仓版另有分层，不在本模板内）。
- **`pack` 不校验资源内容**：它只校验 `pack.json` 结构、`schemaFiles` 存在性与内容哈希；
  `data/*.json` 是否合 schema 由**装载期**校验。所以改了资源要跑 `playtest`，光跑 `pack` 不够。
- **按阶段提示是 `mod.lua` 里写死的映射**（intake → `output` / coordinate → `dispatch` /
  review → `gates` / done → `pointers`）：项目的 playbook 没有对应章节时，回落到第一章的标题与摘要。
  这是优雅降级，不是错误——但如果你希望 review / done 阶段提示自己的章节，就补上
  `gates` / `pointers` 这两章。
