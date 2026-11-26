-- packages/mods/tla-models/mod.lua
-- 《TLA+ 模型移植复现》——S26-4 的形式化模型台（装配入口）。
--
-- 契约：openwiki/contracts/mod/tla-models.md（映射表 / 语义子集边界 / 验收映射）。
-- 形态：headless 模型台（事件直发驱动）。四个教科书级 TLA 模型
-- （GameOfLife / TwoPhase / Klotski / EWD998）各移植为
-- 「状态变量 + 初始谓词 + Next 关系 + 不变量」，与引擎
-- entity/KV + tick + selftest + 回放哈希一一对应。
--
-- 本文件是**装配清单**（多文件纯度契约：入口可注册，模块只导出）：
--   §1 装载            pkg.* install + src 模块 require + 模型注册
--   §2 派生事件登记    包侧全部派生事件名（空缺 = unregistered 当场红）
--   §3 游戏事件        开局 / reset / step / auto / check / summary
--
-- KV 键全部在本 mod 单写者命名空间 `tla.*` 下（mod-package.md §3.2.3）。

-- ============================================================================
-- §1 装载
-- ============================================================================
local entity = require("pkg.entity")
local procgen = require("pkg.procgen")
local codec = require("pkg.codec")

local gol = require("src.gol")
local twophase = require("src.twophase")
local klotski = require("src.klotski")
local ewd998 = require("src.ewd998")
local replay = require("src.replay")
local inv = require("src.inv")

entity.install()
procgen.install()
codec.install()

local MODELS = { gol = gol, twophase = twophase, klotski = klotski, ewd998 = ewd998 }
for _, mod in pairs(MODELS) do
  replay.register(mod)
end

--- 全模型装配 + 钉规范初态（幂等；schema 同 spec 重 define 零写）。
local function setup_all()
  for name in pairs(MODELS) do
    replay.reset(MODELS[name])
  end
end

-- ============================================================================
-- §2 派生事件登记：包侧固定名。真内核只派发已注册事件名，缺一条 =
--    派发时 unregistered 当场失败（pkg-selftest 同纪律）。
--    本 mod 不派生自有事件（轨道反馈走 narrate / meta）。
-- ============================================================================
local DERIVED_EVENTS = {
  -- entity（entity/init.lua / record.lua）
  "entity.created", "entity.attr_changed", "entity.destroyed",
  "entity.tagged", "entity.untagged",
}
for i = 1, #DERIVED_EVENTS do
  ygg.on(DERIVED_EVENTS[i], function() end)
end

-- ============================================================================
-- §3 游戏事件（headless 直发面；载荷 JSON 标量串）
-- ============================================================================

-- ── 开局：全模型钉规范初态 ──────────────────────────────────────────────────
ygg.on("event.start", function()
  setup_all()
  ygg.narrate("tla-models 开局：gol/twophase/klotski/ewd998 四模型已钉规范初态"
    .. "（哈希口径=procgen.hash(snapshot)，锚间隔 " .. replay.ANCHOR_EVERY .. " 拍）")
  return { ok = true }
end)

-- ── reset：payload = 模型名 | "all" ─────────────────────────────────────────
ygg.on("tla.reset", function(payload)
  local name = tostring(payload or "all")
  if name == "all" then
    setup_all()
    ygg.narrate("【重放】全模型已重置规范初态")
    return
  end
  local mod = MODELS[name]
  if mod == nil then
    ygg.narrate("【拒绝】tla.reset 未知模型：" .. name)
    return
  end
  replay.reset(mod)
  ygg.narrate("【重放】" .. name .. " 已重置规范初态")
end)

-- ── step：payload = "model|action"（action 空 = stutter）─────────────────────
ygg.on("tla.step", function(payload)
  local name, action = tostring(payload or ""):match("^([%w_]+)|(.*)$")
  local mod = name and MODELS[name] or nil
  if mod == nil then
    ygg.narrate("【拒绝】tla.step 载荷应为 \"model|action\"：" .. tostring(payload))
    return
  end
  if action == "" then
    action = nil
  end
  local ok, result = pcall(replay.tick, mod, action)
  if not ok then
    ygg.narrate("【拒绝】tla.step：" .. tostring(result))
    return
  end
  ygg.narrate(name .. " #" .. result.tick .. " hash=" .. result.hash)
end)

-- ── auto：payload = "model|n"（自选步 n 拍；确定性 f(seed,tick,state)）────────
ygg.on("tla.auto", function(payload)
  local name, count = tostring(payload or ""):match("^([%w_]+)|(%d+)$")
  local mod = name and MODELS[name] or nil
  if mod == nil then
    ygg.narrate("【拒绝】tla.auto 载荷应为 \"model|n\"：" .. tostring(payload))
    return
  end
  local last
  for _ = 1, tonumber(count) do
    last = replay.auto_tick(mod, "tla")
  end
  if last ~= nil then
    ygg.narrate(name .. " auto×" .. count .. " -> #" .. last.tick .. " hash=" .. last.hash)
  end
end)

-- ── check：payload = 模型名 → 不变量裁决（空 = 绿）───────────────────────────
ygg.on("tla.check", function(payload)
  local name = tostring(payload or "")
  if MODELS[name] == nil then
    ygg.narrate("【拒绝】tla.check 未知模型：" .. name)
    return
  end
  local violations = inv.check(name)
  if #violations == 0 then
    ygg.narrate(name .. " 不变量全绿（" .. #inv.names(name) .. " 条注册）")
  else
    for i = 1, #violations do
      ygg.narrate("【拒绝】" .. name .. " 不变量违例：" .. violations[i])
    end
  end
end)

-- ── 摘要（summaryEvent：只读；daemon / 面板读它；不落 narrative）────────────
ygg.on("tla.summary", function()
  local out = {}
  for _, name in ipairs(replay.model_names()) do
    local tick = ygg.kv.get("tla.tick." .. name) or 0
    out[#out + 1] = name .. "#" .. tick .. "=" .. tostring(ygg.kv.get("tla.hash." .. name .. "." .. tick))
  end
  local summary = "tla-models：" .. table.concat(out, " ")
  ygg.meta("summary", summary)
  ygg.meta("tla_models", summary)
  return summary
end)

return {
  gol = gol, twophase = twophase, klotski = klotski, ewd998 = ewd998,
  replay = replay, inv = inv,
}
