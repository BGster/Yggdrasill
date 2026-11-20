-- `packages/mods/pkg-selftest/mod.lua` —— **包自测的消费夹具**，不是产品 Mod。
--
-- 作用有两个，且都只围绕「包能不能真的装上」：
--
--   ① 让 `examples/selftest` 有一个**声明了 `packages` 段**的 Mod 可跑，
--      于是 `<包根>/<id>/test/*.lua` 会被收集执行（`packages::resolve` 的闭包决定
--      收集哪些包，见 `crates/ygg-sandbox/examples/selftest.rs` 的 `package_plan`）。
--   ② 提供一个 `pkg.smoke` handler，作为**真沙箱**冒烟的被测面
--      （`crates/ygg-sandbox/tests/packages_sandbox.rs` 用例 ①）。
--
-- 它不写 KV、不注册业务 handler：包自测自带 mock `ygg`（`packages/README.md`
-- §5「包测试不依赖消费 mod 的 harness」），入口只做装配。
--
-- 为什么必须存在这一层：生产 Mod（`agent-seed`）**不声明包**，所以它的
-- `cases passed=` 永远看不到 `pkg.*`。夹具把「包装载面」变成一条可复算的命令：
--   cargo run -q --example selftest -p ygg-sandbox -- \
--     --mod packages/mods/pkg-selftest --packages packages/libs --filter pkg-

local entity = require("pkg.entity")
local codec = require("pkg.codec")
local quest = require("pkg.quest")
local dialogue = require("pkg.dialogue")
local procgen = require("pkg.procgen")
local role_skill = require("pkg.role_skill")
local assetgen = require("pkg.assetgen")

-- 声明闭包的同构清单：装配顺序即本表顺序，`#PACKAGES` 就是冒烟收尾行里的 N。
-- 它刻意**不是**一份手抄的数字——加一个包 = 表里加一项，冒烟收尾行自动跟上一格，
-- 而 `packages_sandbox.rs` 再把 N 与 `pack.json` 的 `packages[]`、与
-- `packages/libs/` 下的产品目录两两对照，任一方向不等即红（R9）。
local PACKAGES = {
  entity,
  codec,
  quest,
  dialogue,
  procgen,
  role_skill,
  assetgen,
}

-- 装配：包的 `install` 都只记 deps / 注注册表（不预读、不预写，契约 I7 / I8）。
-- procgen / assetgen 不接宿主面（不写 KV、不发事件、不注册 handler），所以
-- `install()` 无参；其余四包在缺 `deps` 时回落到 `ygg` 全局（真沙箱里就在场）。
for i = 1, #PACKAGES do
  PACKAGES[i].install()
end

-- ── 真沙箱冒烟（r2）────────────────────────────────────────────────────────

-- 这条 handler 只有走 `ModBundle::load` → `build_kernel` 的真内核（inject 的 math、
-- 完整 host API、单张模块表）才跑得到，所以它钉的是**生产装载面**，不是普通 Lua 的
-- 近似。每包一次最小调用，但必须穿过该包唯一的宿主写路径——载荷类型 / 事件登记 /
-- KV 标量这一整类缺陷只有在真宿主面才会露头（role-skill 的表载荷 bug 就是这么抓到的：
-- 普通 Lua 的 mock `ygg.emit` 吃表，真内核硬拒 `unsupported payload type: table`）。

--- 派生事件面（包经 `ygg.emit` 入队的固定名字集合，逐包列出）。
---
--- 空订阅是**故意的**：真内核只派发已注册的事件名，于是「新增派生事件忘了登记」
--- 会在冒烟里以 `unregistered event` 当场失败，而不是等某个 mod 订阅时才暴露。
local DERIVED_EVENTS = {
  -- entity（entity/init.lua）：建立 / 属性变更 / 销毁 / 标签增删
  "entity.created",
  "entity.attr_changed",
  "entity.destroyed",
  "entity.tagged",
  "entity.untagged",
  -- quest（quest/flow.lua 的 TRANSITIONS + 显式跳步 / 放弃）
  "quest.opened",
  "quest.advanced",
  "quest.completed",
  "quest.abandoned",
  "quest.jumped",
  -- dialogue（dialogue/dispatch.lua）
  "dialogue.failed",
  "dialogue.depth_capped",
  -- role-skill（role-skill/actors.lua + skills.lua，载荷全是标量串——D11）
  "role.granted",
  "role.revoked",
  "role.skill_gained",
  "role.skill_lost",
}

for i = 1, #DERIVED_EVENTS do
  ygg.on(DERIVED_EVENTS[i], function() end)
end

ygg.on("pkg.smoke", function()
  local lines = {}

  -- ── entity：schema 编译 → spawn → 同值写（走整/浮判据）→ ref 值域拒绝 ──
  entity.define("smoke", {
    attrs = {
      hp = { type = "number" },
      atk = { type = "number" },
      ref = { type = "ref" },
    },
  })
  local id = entity.spawn("smoke", { hp = 2.5, atk = 0 })
  -- 同值写必须返回 `false` 且不发事件（契约 I6）——这条分支就是旧 `math.type`
  -- 比较的落点之一（`1` 与 `1.0` 在存诸面是两个值）。
  assert(entity.set(id, "hp", 2.5) == false, "同值同型写必须返回 false")
  assert(entity.set(id, "hp", 2) == true, "整值写必须判为变更")
  -- `2` 与 `2.0` 在 `==` 下相等，但存诸面是两个值：必须再判一次变更。这是整/浮
  -- **身份**判据的唯一落点——`v % 1 == 0` 会把它吞成同值、静默丢一次写入。
  assert(entity.set(id, "hp", 2.0) == true, "`2` 与 `2.0` 必须判为异值")
  assert(entity.set(id, "hp", 2.5) == true, "回到 2.5 必须判为变更（叙述行读它）")
  -- ref 值域：整值浮点 `2.0` 也必须被拒——`v % 1 == 0` 会把它吞成整型，
  -- 而 entity 的 KV 键是 `tostring(id)`，`2` 与 `2.0` 是两个键，字节稳定当场破。
  local non_int = pcall(entity.set, id, "ref", 1.5)
  local whole_float = pcall(entity.set, id, "ref", 2.0)
  local as_int = pcall(entity.set, id, "ref", 2)
  assert(as_int, "ref 值域必须收正整数")
  lines[#lines + 1] = "pkg.entity ok id="
    .. tostring(id)
    .. " hp="
    .. tostring(entity.get(id, "hp"))
    .. " ref_reject="
    .. tostring(not non_int and not whole_float)

  -- ── codec：数组键面 / 整浮分写 / 解码器 / sha256 已知向量 ────────────────
  local array_text = codec.encode({ 1, 2, 3 })
  local object_text = codec.encode({ i = 1, f = 1.5 })
  local roundtrip = codec.encode(codec.decode(object_text)) == object_text
  lines[#lines + 1] = "pkg.codec ok array="
    .. array_text
    .. " obj="
    .. object_text
    .. " roundtrip="
    .. tostring(roundtrip)
    .. " sha="
    .. codec.sha256("abc")

  -- ── quest：模板编译 → open → advance → get（KV 字符串载荷 + 派生事件）──
  quest.define("smoke", {
    steps = {
      { id = "one", guide = "smoke.one" },
      { id = "two", guide = "smoke.two" },
    },
  })
  local quest_id = quest.open("smoke")
  quest.advance(quest_id)
  local inst = quest.get(quest_id)
  lines[#lines + 1] = "pkg.quest ok id="
    .. tostring(quest_id)
    .. " state="
    .. inst.state
    .. " step="
    .. tostring(inst.step)

  -- ── dialogue：记忆窗口写读（KV 前缀面）──────────────────────────────────
  dialogue.say("smoke.thread", "hello")
  lines[#lines + 1] = "pkg.dialogue ok history=" .. tostring(#dialogue.history("smoke.thread"))

  -- ── role-skill：引用面校验 + 合成（四个派生事件全是标量载荷）────────────
  role_skill.define("smoke.role", { attrs_spec = { atk = 3 } })
  local granted = role_skill.grant(id, "smoke.role")
  local has = role_skill.has(id, "smoke.role")
  local atk = role_skill.effective(id, "atk")
  lines[#lines + 1] = "pkg.role-skill ok granted="
    .. tostring(granted)
    .. " has="
    .. tostring(has)
    .. " atk="
    .. tostring(atk)

  -- ── procgen：无时钟 PRNG + codec 摘要面 ────────────────────────────────
  local handle = procgen.rng(42, "smoke")
  local next32 = procgen.next32(handle)
  local digest = procgen.hash({ "a", 1 })
  lines[#lines + 1] = "pkg.procgen ok next32=" .. tostring(next32) .. " hash=" .. tostring(digest)

  -- ── assetgen：纯导出（自省面 + 产物 canon 行）───────────────────────────
  local algos = assetgen.algo_list("texture")
  local texture = assetgen.texture({ algo = "texture.fbm", params = { w = 4, h = 4, ch = 1 } }, "smoke")
  lines[#lines + 1] = "pkg.assetgen ok algos="
    .. tostring(#algos)
    .. " canon="
    .. assetgen.canon(texture)

  lines[#lines + 1] = "pkg.smoke packages=" .. tostring(#PACKAGES)
  for i = 1, #lines do
    ygg.narrate(lines[i])
  end
end)
