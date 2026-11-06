# 从单文件 Mod 迁移到多文件

> 适用：把已经能跑的单文件 `mod.lua`（数百到数千行）拆成 `src/*.lua`。
> 前置：宿主已支持 `code` 段（`ygg-lint --pack` 可用、`--drive` 存在）。
> 本文以 `mxbc-data-gov-workbench` 的 `agent-seed`（4661 行 / 25 个 `§` 分区）为实例，其他 Mod 照此调整。

---

## 0. 迁移后的形态（先看清终点）

```lua
-- mod.lua —— 只剩装配：读资源 → require 模块 → 按序 install → 初始化状态
local TEXTS = ygg.data("ygg:scene/texts")
local CONFIG = ygg.data("ygg:agent/config")

local util    = require("src.util")
local config  = require("src.config")
local reqs    = require("src.requirements")
local lanes   = require("src.lanes")
local props   = require("src.proposals")
local render  = require("src.render")
local menus   = require("src.menus")
local fsms    = require("src.fsm")

-- 派发序 = 下面的调用序（不是 require 序）
config.install({ texts = TEXTS, config = CONFIG })
reqs.install({ util = util, config = config })
lanes.install({ util = util, reqs = reqs })
props.install({ util = util, reqs = reqs })
fsms.install({ reqs = reqs, props = props, lanes = lanes })
render.install({ reqs = reqs, props = props, menus = menus, texts = TEXTS })
menus.install({ reqs = reqs, render = render, util = util })

ygg.fsm.define("stage", { initial = "intake", events = { "stage.advance" } })
```

```json
// pack.json 新增
"code": {
  "entry": "mod.lua",
  "modules": [
    "src/util.lua", "src/config.lua", "src/requirements.lua",
    "src/lanes.lua", "src/proposals.lua", "src/render.lua",
    "src/menus.lua", "src/fsm.lua"
  ]
}
```

**模块顶层只做定义，不做任何 ygg 副作用**（`ygg.on` / `kv.set` / `narrate` / `emit` / `choice` / `meta` / `entity.*` / `fsm.define` / `narrative.*` 全部禁止；`ygg.data` / `kv.get` / `entity.has_tag` 等纯读允许）。

---

## 1. 动手前：摸清耦合面（三条命令）

```bash
MOD=.ygg/mods/pi-agent-seed

# ① 全局函数（`x = function(...)` 而非 `local function`）——它们跨分区被调用，是真正的耦合点
rg -n '^[a-z_]+ = function\(' $MOD/mod.lua

# ② 闭包上值（模块私有的可变状态）——**拆到不同文件就断了**
rg -n '^local [a-z_]+ = (nil|0|""|false)\s*$' $MOD/mod.lua

# ③ 每个 § 分区的行数——决定拆分边界与阶段划分
rg -n '^-- §' $MOD/mod.lua
```

mxbc 的实测结果（迁移前先知道这三件事，能省掉大量返工）：

| 项 | 数量 | 结论 |
| --- | --- | --- |
| 全局函数 | **4 个**：`pick_integrated` / `prompt_vars` / `req_release` / `req_satisfied` | 改成模块导出（`reqs.release`），调用点跟着改 |
| 闭包上值 | **2 个**：`stage_ticket`（2384 行）/ `choice_ticket`（2457 行） | 必须与使用它的函数**放在同一模块**，见 §4 |
| `§` 分区 | 25 个 | 天然模块边界，见 §3 的映射表 |

耦合面这么小是好消息：4661 行里真正「跨文件会断」的只有 6 个点。

---

## 2. 迁移铁律

1. **一阶段一提交**，每阶段都能独立验收、独立回退。不要一次拆完再跑门禁。
2. **不在同一阶段既搬代码又改逻辑**。搬就是搬，改就是改。
3. **每阶段结束都跑**：
   ```bash
   cargo run -p ygg-lint -- --pack $MOD                    # 0 error
   ygg-agent --mod $MOD --run-id probe --runs-root /tmp/r --event event.start   # 装载成功
   ```
4. **改完源码必须重算 manifest**（内容寻址，不重算会按字节数拒绝装载）：
   ```bash
   cargo run --example pack -- $MOD
   ```
5. **派发序对照要在每个涉及 `ygg.on` 的阶段做**（见 §5），这是唯一的「拆错了」探测器。

---

## 3. 五阶段拆分（按风险递增）

### 阶段 1 · 纯工具（零风险）

| 搬什么 | mxbc 的 `§工具`（87 行）：`fill` / `split` / `cut` / `trim` / `join` / `contains` / `to_bool` / `spaces` |
| --- | --- |

```lua
-- 前：mod.lua 里
local function fill(template, vars) ... end
local function split(text, sep) ... end

-- 后：src/util.lua
local M = {}
function M.fill(template, vars) ... end
function M.split(text, sep) ... end
return M

-- 入口
local util = require("src.util")
-- 全文件把 `fill(` → `util.fill(`（`rg -c '\bfill\('` 确认调用点数）
```

**为什么零风险**：这些函数不碰 `ygg`，搬完行为完全等价。

**验收**：`--pack` 0 error + 装载成功 + playtest 与拆前一致。

---

### 阶段 2 · 资源与配置（低风险）

| 搬什么 | `§资源`（7 行）+ `§配置`（95 行） |
| --- | --- |

```lua
-- src/config.lua
local M = {}
local TEXTS, CONFIG        -- 模块私有：装载期快照，读档时随模块重跑重建

function M.install(deps)
  TEXTS, CONFIG = deps.texts, deps.config   -- 由入口注入（模块顶层不做 ygg 调用）
end

function M.effective(path)   -- 内部用 ygg.kv.get：纯读，函数体内合法
  return ygg.kv.get("settings." .. path) or M.static(path)
end
return M
```

**注意**：`§资源` 现在是 `local TEXTS = ygg.data(...)` 在顶层。`ygg.data` 属纯读面，放在模块顶层**是允许的**；但更干净的做法是入口读、注入（如上），这样模块不依赖装载时机。

---

### 阶段 3 · 状态访问器（中风险）

| 搬什么 | `§状态`（22）+ `§需求`（319）+ `§泳道`（301）+ `§提案`（440）= **1082 行** |
| --- | --- |

这一层的共同特征：**读 KV 为主，写也在函数体内**，几乎不注册 handler。所以迁移主要是机械劳动：

- `req_*` 系列函数 → `src/requirements.lua` 导出 `M.*`
- `lane_*` / `LANES.*` → `src/lanes.lua`
- `prop_*` → `src/proposals.lua`
- **`req_release` / `req_satisfied`**（两个全局函数）→ 收进 `requirements.lua`，调用点从 `req_release(i)` 改成 `reqs.release(i)`

```bash
# 看清这两个全局函数被谁调用（决定改动面）
rg -c '\breq_release\(|\breq_satisfied\(' .ygg/mods/pi-agent-seed/mod.lua
```

---

### 阶段 4 · 展示层（中风险）

| 搬什么 | `§地图`（183）+ `§指南`（113）+ `§二级菜单页`（32）+ `§菜单`（412）+ `§渲染`（182）+ `§摘要`（127）= **1049 行** |
| --- | --- |

特征：大量 `ygg.narrate` 与 `ygg.meta` 调用——**全部在函数体内**，所以搬进模块函数后依然合法。

```lua
-- src/summary.lua
local M = {}
function M.emit(reqs, props, lanes)     -- 内部 ygg.meta(...)：函数体内，合法
  ...
end
return M
```

`§菜单`（412 行）是「选项文本 ↔ 动作载荷」的单一来源，建议单独成模块：它是**数据 + 纯函数**，几乎无副作用。

---

### 阶段 5 · 注册层（高风险，唯一需要改结构的一层）

| 搬什么 | `§动作解析`（278）+ `§门禁`（87）+ `§里程碑`（41）+ `§规则闸`（41）+ `§动作 handler`（227）+ `§泳道动作`（957）+ `§集成`（175）+ `§发布`（352）+ `§状态机`（66）= **2224 行** |
| --- | --- |

**这层的代码全都以 `ygg.on(...)` 结尾，必须改形态**：

```lua
-- 前：mod.lua 顶层
ygg.on("task.intake", function(payload) ... end)
ygg.on("requirement.add", function(payload) ... end)
-- ... 几十个

-- 后：src/handlers/actions.lua
local M = {}
function M.install(deps)
  local reqs, props, util = deps.reqs, deps.props, deps.util

  ygg.on("task.intake", function(payload)
    ...  -- 函数体里可以随便用 reqs.* / ygg.kv.set / ygg.narrate
  end)
  ygg.on("requirement.add", function(payload) ... end)
end
return M
```

`§状态机` 的 `ygg.fsm.define(...)` 同理——搬进某个 `install()`，或直接在入口调（入口不受限）。

**建议再细分**（mxbc 的 957 行 `§泳道动作` 太胖）：

```text
src/handlers/
├── actions.lua       动作解析 + 通用动作 handler
├── lanes.lua         泳道动作（957 行可以再拆成 lanes/*.lua）
├── integration.lua   集成阶段
└── release.lua       发布阶段
```

---

## 4. 两类硬骨头

### 4.1 全局函数（mxbc 有 4 个）

```lua
-- 前：全局，任何分区都能直接调
req_release = function(index) ... end

-- 后：模块导出
-- src/requirements.lua
function M.release(index) ... end
return M

-- 调用点（用 rg 找全部）
req_release(3)  →  reqs.release(3)
```

**为什么要改**：全局函数在单文件里是「隐式依赖」，拆文件后必须显式化。如果嫌改动量大，也可以先把它们塞进一个 `src/globals.lua` 导出，逐步收编——但不推荐，等于把旧债带进新结构。

### 4.2 闭包上值（mxbc 有 2 个：`stage_ticket` / `choice_ticket`）

这是**唯一无法机械搬运**的东西。`local stage_ticket = nil` 是两个函数的共享可变状态，拆到不同文件就再也共享不到。

两种解法：

**解法 A（推荐）**：把共享上值的函数放进同一模块

```lua
-- src/fsm.lua —— advance() 与 stage_allowed() 都在这里，stage_ticket 是它们的共享上值
local stage_ticket = nil
local function stage_allowed(target, expected) ... stage_ticket = nil ... end
local function advance(target) ... stage_ticket = expected ... end
function M.install(deps) ygg.on("stage.advance", ...) end
return M
```

前提：这两个函数本来就该在一起（就 mxbc 而言，`stage_ticket` 只服务于转场盖章，天然同属 `§状态机`）。

**解法 B**：改成显式状态容器

```lua
local S = { stage_ticket = nil }        -- 通过 deps 注入到需要它的模块
function M.stamp(t) S.stage_ticket = t end
function M.allowed(target, expected) ... end
```

只在「共享上值的函数确实分属不同职责」时才用。代价是状态变公开，容易被误改。

**不要去改 `ygg.kv`**——`stage_ticket` 是瞬时校验章（不是持久状态），写进 KV 会让它跨转换存活，语义就错了。

---

## 5. 派发序对照（核心验收）

改造后模块不再注册，**派发序只由入口 `install()` 的调用序决定**。所以每拆完一个涉及 `ygg.on` 的阶段，都要确认派发序没变。

**方法**：拆前跑一次基准局，拆后跑同样输入，比对叙事序列。

```bash
MOD=.ygg/mods/pi-agent-seed
BIN=.ygg/bin/ygg-agent

# 拆前：录基线（把每一步的叙事与 pendingChoice 存下来）
rm -rf /tmp/base && $BIN --mod $MOD --run-id base --runs-root /tmp/base --event event.start > /dev/null
for i in $(seq 1 12); do
  $BIN --mod $MOD --run-id base --runs-root /tmp/base --resume --event ygg.choice_selected --payload '1' > /dev/null
done
cp /tmp/base/base/events.jsonl /tmp/before-events.jsonl

# 拆后：同样跑一遍
rm -rf /tmp/after && $BIN --mod $MOD --run-id after --runs-root /tmp/after --event event.start > /dev/null
for i in $(seq 1 12); do
  $BIN --mod $MOD --run-id after --runs-root /tmp/after --resume --event ygg.choice_selected --payload '1' > /dev/null
done

# 比对：去掉 seq/timestamp/run 相关字段后，叙事与动作序列必须逐条相同
python3 - <<'PY'
import json
def norm(p):
    out = []
    for line in open(p):
        e = json.loads(line)
        out.append((e["event"], e["payload"], tuple(n for n in (e.get("narrative") or [])))
    )
    return out
a, b = norm('/tmp/before-events.jsonl'), norm('/tmp/after/after/events.jsonl')
print("一致" if a == b else f"不一致：首个差异 #{next(i for i,(x,y) in enumerate(zip(a,b)) if x!=y)}")
PY
```

**若基线本身就是坏的**（拆前就停在某处不动），先修好再拆——否则无从判断是否拆坏。

---

## 6. 常见错误对照表

| 报错 | 原因 | 修法 |
| --- | --- | --- |
| `YGG-MOD-EFFECT` | 模块顶层调了 `ygg.on` / `kv.set` / `narrate` / `emit` / `choice` / `meta` / `entity.*` / `fsm.define` / `narrative.*` | 搬进 `function M.install()` 体内；或直接放到入口 |
| `YGG-MOD-NAME` | 模块路径非法（绝对路径 / `..` / 反斜杠 / **段内带点** / 非 `.lua`） | 改名；`src/a.b.lua` → `src/a_b.lua` |
| `YGG-MOD-MISSING` | `code.modules` 列了但文件不存在 | 补文件，或从声明里删掉 |
| `YGG-MOD-UNDECLARED` | 代码里 `require("x.y")` 但没登记 | 加进 `code.modules` |
| `YGG-MOD-UNUSED` | 登记了但入口不可达 | 删声明（warning，不阻断） |
| `YGG-MOD-CYCLE` | 模块互相 require | 把共享部分下沉到第三个模块 |
| `content size mismatch` | 改了模块但没重算 manifest | `cargo run --example pack -- $MOD` |
| `attempt to index a nil value`（`M.install`） | 入口没 `require` 就调 `M.install` | 先 require |
| 装载期 `ygg.narrate` 无输出 | 模块顶层调 narrate（已被影子表拒绝，理论上到不了这一步） | 搬进函数体 |

---

## 7. 回退

任一阶段出问题：

```bash
git checkout -- .ygg/mods/pi-agent-seed   # 丢掉本阶段改动
```

**不要**试图靠「删掉 `code` 段」回退——代码已经拆到 `src/` 了，入口还在 `require`，删声明只会让 `require` 全部失败。真正的回退是 `git revert` 那个阶段提交。

---

## 8. 完成后

- `pack.json` 的 `code.modules` 与磁盘逐项一致（`ygg-lint --pack` 会查）
- `manifest.json` 的 `code` 数组已重算
- 派发序对照通过（§5）
- `playtest` 到结局，`ygg-lint` 0 error
- 若有 CI 骨架守卫（种子/项目双层结构），确认 `sync-seed-mod.sh` 已支持 `src/` 目录：
  已完成（脚本同步 `mod.lua` + `src/**` + `schemas/`，并检查两侧 `pack.json` 的 `code` 段一致；
  CI 的骨架守卫四查：`mod.lua` / `src/` / `schemas/` / `code` 段）。**mxbc 不是双层结构，与它无关**；
  双层实例见下一节的 Yggdrasill 映射。

---

## 附：mxbc 的拆分映射表（4661 行 → 8 个模块）

| 目标模块 | 来源 `§` 分区 | 行数 | 风险 |
| --- | --- | --- | --- |
| `src/util.lua` | `§工具` | 87 | 零 |
| `src/config.lua` | `§资源` + `§配置` | 102 | 低 |
| `src/requirements.lua` | `§状态` + `§需求` | 341 | 中 |
| `src/lanes.lua` | `§泳道` | 301 | 中 |
| `src/proposals.lua` | `§提案` | 440 | 中 |
| `src/render.lua` | `§地图` + `§渲染` + `§摘要` | 492 | 中 |
| `src/menus.lua` | `§指南` + `§二级菜单页` + `§菜单` | 557 | 中 |
| `src/handlers/*.lua` | `§动作解析` + `§门禁` + `§里程碑` + `§规则闸` + `§动作 handler` + `§泳道动作` + `§集成` + `§发布` + `§状态机` | 2224 | 高 |
| `mod.lua`（入口） | 装配 + 资源读取 + `fsm.define` | ~60 | — |

`§泳道动作` 957 行是最大单块，建议再拆成 `handlers/lanes/{stamp,clear,render}.lua`。

---

## 附二：Yggdrasill pi-agent-seed 的拆分映射表（3630 行 → 21 模块）

双层结构（种子模板 + 项目实例）的实例：两侧骨架逐字节一致，`data/` 各自维护。

| 目标模块 | 来源 `§` 分区 | 风险 | 备注 |
| --- | --- | --- | --- |
| `src/util.lua` | `§工具` | 零 | 纯函数，无 install |
| `src/config.lua` | `§配置` | 低 | 两层配置 + `PLAY_MODES` |
| `src/requirements.lua` | `§状态` + `§需求` | 中 | `§状态` 与 `§需求` 合一块；`req_satisfied` → `reqs.satisfied` |
| `src/lanes.lua` | `§泳道`（LANES 表 + lane_text） | 中 | `LANES` 由入口注入 requirements（避免环） |
| `src/proposals.lua` | `§提案` | 中 | 含 `apply_changes` 与 split/merge/conflict 落地 |
| `src/gates.lua` | `§门禁` 的函数部分 | 低 | 只读选择器；同段的 `MENU[review]` 归 `menus` |
| `src/guide.lua` | `§指南` | 低 | 章节索引放进 `install()`（依赖注入后的 PLAYBOOK） |
| `src/menus.lua` | `§二级菜单页` + `§菜单` + `MENU[STAGE.review]` | 中 | `CONFIG_UI` 折叠成 `menus.*` |
| `src/render.lua` | `§地图` + `§渲染` + `§摘要` | 中 | `prompt_vars` → `render.prompt_vars`；`agent.summary` 在 `render.register()` |
| `src/wire.lua` | `§动作解析` | 中 | payload 解析 + `@ext=` 幂等键 |
| `src/flow.lua` | `§推进` | 中 | `advance` / `refuse` 唯一出口；`stage_ticket` 同模块 |
| `src/handlers/choice.lua` | `§人类动作门禁` + `ygg.choice_selected` | 高 | **票据与盖章/取证必须同模块** |
| `src/handlers/*.lua`（其余 9 个） | `§动作 handler` + `§泳道动作` | 中 | 按动作族切：actions / lanes / proposals / review / requirement_ops / gate_actions / config / views / lifecycle |
| `mod.lua`（入口） | `§资源` + `§状态机` | — | 装配 + `fsm.define`，≈110 行 |

经验：`§泳道动作` 这类「什么都装」的大块按**动作族**切；两处无法机械搬运的耦合是
`HUMAN.ticket`（与 choice 路由同模块）和 `stage_ticket`（与 advance 同模块）。
