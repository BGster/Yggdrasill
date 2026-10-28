-- packages/mods/test-adventure/mod.lua
-- 无头测试冒险《灯下谜径》——M0 内核创作面验证 Mod。
--
-- 验证面（每一条都对应内核 API 的实际使用）：
--   1. 叙事流   ygg.narrative.node / ygg.narrative["goto"]（goto 是 Lua 5.4
--               保留字，括号形态强制——本文件全程使用括号形态）；
--   2. 多分支   ygg.choice + "ygg.choice_selected" 索引路由（选择 = 新外部
--               事件；handler 挂起处用 kv "stage" 记住待回应的选择点）；
--   3. 角色道具 ygg.entity.spawn / set_attribute / add_tag / has_tag——
--               主角（hp 属性）与铜提灯（item:lantern 标签门）；
--   4. 状态机   ygg.fsm.define——章节推进 tavern → road → cellar / inn；
--   5. 资源     ygg.data——全部场景文本 / 选项文案 / 数值调参来自资源包
--               （Mod 顶层装载期快照进 local，handler 闭包消费）；
--   6. 多结局   三个结局（A 秘藏 / B 平安 / C 折返），各以【结局X】叙述行
--               标识并落 kv "ending"（测试以此断言）。
--
-- 玩家可达路径（3 结局 × 2 选择点）：
--   开始 → 序章 → 酒馆(选择1: 提灯?)
--     带灯 → 岔路(选择2: 去哪?) → 地窖 → 【结局A】灯下秘藏
--     无灯 → 岔路 → 地窖 → 摔伤 → 【结局C】暗巷折返
--     任意 → 岔路 → 客房 → 【结局B】平安一夜
--
-- 注意：实体 id 不能存 Lua local（读档后 Mod chunk 重装载、旧 local 指向
-- 已被覆写的 store），一律落 kv 持久化（见 §实体 注册块注释）。

-- ============================================================================
-- §资源 装载期快照：resources::install 先于 Mod chunk，顶层即可读 ygg.data。
-- ============================================================================
local TEXTS = ygg.data("ygg:scene/texts")

-- ============================================================================
-- §状态机 章节推进。表达力边界（实证后沉淀）：fsm.define 一机一事件、
-- 转换无 payload/条件分派、声明序先到先得——同一 from 的第二条出边无法与
-- 第一条同机共存（road → cellar / road → inn 会永远先命中 cellar）。绕法：
-- 主线机（chapter）每个状态只留一条出边，客房支线独立成机、独立事件名。
-- 状态持久化在 kv "fsm.<name>"，随存档往返。
-- ============================================================================
ygg.fsm.define{
  name = "chapter",
  initial = "tavern",
  event = "chapter.advance",
  transitions = {
    { from = "tavern", to = "road" },   -- 告别酒馆（choice_selected @酒馆 发出）
    { from = "road",   to = "cellar" }, -- 下地窖（node:cellar_door 发出）
  },
}

ygg.fsm.define{
  name = "inn_route",
  initial = "road",
  event = "chapter.to_inn",
  transitions = {
    { from = "road", to = "inn" }, -- 上楼回房（node:inn 发出）
  },
}

-- ============================================================================
-- §实体 角色与道具：spawn 只做一次（kv 门防重复）；id 落 kv——读档后
-- handler 经 kv 取回同一实体（统一 store 代决：id 在存档内保持稳定）。
-- ============================================================================
ygg.narrative.node("node:prologue", function()
  if ygg.kv.get("hero.id") == nil then
    local hero = ygg.entity.spawn()
    ygg.entity.set_attribute(hero, "hp", 10)
    ygg.entity.add_tag(hero, "char:hero")
    ygg.kv.set("hero.id", hero)

    local lantern = ygg.entity.spawn()
    ygg.entity.add_tag(lantern, "item:copper") -- 材质标签（非持有判定）
    ygg.kv.set("lantern.id", lantern)
  end
  ygg.narrate(TEXTS.scenes.prologue)
  ygg.narrative["goto"]("node:tavern")
end)

-- 酒馆：第一个选择点。ygg.choice 会以哨兵错误终止本 handler——choice
-- 之后不得再有任何叙述 / 跳转（这不是风格问题，是内核终结语义）。
ygg.narrative.node("node:tavern", function()
  ygg.narrate(TEXTS.scenes.tavern)
  ygg.kv.set("stage", "tavern")
  ygg.choice(TEXTS.prompts.tavern, { TEXTS.options.tavern_yes, TEXTS.options.tavern_no })
end)

-- 岔路：第二个选择点（同上，choice 收尾）。章节 tavern → road 的推进
-- 不能在这里 emit——choice 终结转换会丢弃同 handler 先入队的 FIFO 派生
-- 事件（实证过的内核语义），故该推进移到 choice_selected handler。
ygg.narrative.node("node:crossroad", function()
  ygg.narrate(TEXTS.scenes.crossroad)
  ygg.kv.set("stage", "crossroad")
  ygg.choice(TEXTS.prompts.crossroad, { TEXTS.options.crossroad_cellar, TEXTS.options.crossroad_inn })
end)

-- 地窖门口：章节 road → cellar；道具判定面——提灯标签 "item:lantern" 门控暗门结局。
ygg.narrative.node("node:cellar_door", function()
  ygg.emit("chapter.advance", "to_cellar")
  local lantern = ygg.kv.get("lantern.id")
  if ygg.entity.has_tag(lantern, "item:lantern") then
    ygg.narrate(TEXTS.scenes.cellar_lit)
    ygg.narrative["goto"]("node:ending_a")
  else
    ygg.narrate(TEXTS.scenes.cellar_dark)
    ygg.narrative["goto"]("node:ending_c")
  end
end)

-- 客房：客房支线机 road → inn。
ygg.narrative.node("node:inn", function()
  ygg.emit("chapter.to_inn", "to_inn")
  ygg.narrate(TEXTS.scenes.inn)
  ygg.narrative["goto"]("node:ending_b")
end)

-- ============================================================================
-- §结局 三个结局节点：各自叙述标识行 + 落 kv "ending"。结局 C 额外消费
-- 资源数值（tuning.dark_fall_damage）扣主角 hp——数值调参面验证。
-- ============================================================================
ygg.narrative.node("node:ending_a", function()
  ygg.narrate(TEXTS.endings.a)
  ygg.kv.set("ending", "A")
end)

ygg.narrative.node("node:ending_b", function()
  ygg.narrate(TEXTS.endings.b)
  ygg.kv.set("ending", "B")
end)

ygg.narrative.node("node:ending_c", function()
  ygg.narrate(TEXTS.endings.c)
  local hero = ygg.kv.get("hero.id")
  local hp = ygg.entity.get_attribute(hero, "hp")
  ygg.entity.set_attribute(hero, "hp", hp - TEXTS.tuning.dark_fall_damage)
  ygg.narrate("（主角 hp 降至 " .. tostring(ygg.entity.get_attribute(hero, "hp")) .. "）")
  ygg.kv.set("ending", "C")
end)

-- ============================================================================
-- §入口与选择路由 event.start → 序章；ygg.choice_selected 以 payload 索引
-- 路由。两个选择点共用同一 respond handler——kv "stage" 区分当前待回应点。
-- ============================================================================
ygg.on("event.start", function()
  ygg.narrative["goto"]("node:prologue")
end)

ygg.on("ygg.choice_selected", function(index)
  local stage = ygg.kv.get("stage")
  if stage == "tavern" then
    local lantern = ygg.kv.get("lantern.id")
    if index == 1 then
      -- 持有判定标签：只有带上提灯才打上（item:copper 是出厂材质标签）。
      ygg.entity.add_tag(lantern, "item:lantern")
      ygg.kv.set("took_lantern", true)
      ygg.narrate(TEXTS.beats.lantern_taken)
    else
      ygg.kv.set("took_lantern", false)
      ygg.narrate(TEXTS.beats.lantern_left)
    end
    -- 章节 tavern → road：在本 handler emit（无 choice 终结，FIFO 完整排空）。
    ygg.emit("chapter.advance", "to_road")
    ygg.narrative["goto"]("node:crossroad")
  elseif stage == "crossroad" then
    ygg.narrate(TEXTS.beats.cellar_choice_hint)
    if index == 1 then
      ygg.narrative["goto"]("node:cellar_door")
    else
      ygg.narrative["goto"]("node:inn")
    end
  end
  -- 未知 stage / 越界索引：静默（与 fsm 无匹配 = 静默 no-op 同口径）。
end)
