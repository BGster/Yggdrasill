-- `tests/test-0-load.lua` —— 装配冒烟：四模型注册面 + 规范初态 + 事件面。
--
-- 本文件是 selftest 的第一条门：mod.lua 装配成功、四模型全部注册进
-- replay/inv 两张表、event.start 直发后各模型规范初态可哈希。

local replay = require("src.replay")
local inv = require("src.inv")

check("load.four-models-registered", function()
  ygg_dispatch("event.start", "0") -- install（schema+不变量注册）在 reset 期完成
  local names = replay.model_names()
  assert_eq(#names, 4, "注册模型数")
  assert_eq(names[1], "ewd998", "升序首")
  assert_eq(names[4], "twophase", "升序尾")
  for i = 1, #names do
    assert_true(inv.known(names[i]), names[i] .. " 应有注册不变量")
  end
  return nil
end)

check("load.event-start-resets-all", function()
  ygg_dispatch("event.start", "0")
  local lines = ygg_narrated_lines()
  assert_true(#lines > 0 and lines[#lines]:find("开局", 1, true) ~= nil, "开局叙事行")
  for _, name in ipairs(replay.model_names()) do
    local mod = replay.model(name)
    local snap = mod.snapshot()
    assert_true(type(snap) == "table", name .. " 初态快照应为表")
  end
  return nil
end)

check("load.canonical-init-inv-green", function()
  ygg_dispatch("event.start", "0")
  for _, name in ipairs(replay.model_names()) do
    local violations = inv.check(name)
    assert_eq(#violations, 0, name .. " 规范初态不变量绿")
  end
  return nil
end)

check("load.hash-chain-and-anchors", function()
  ygg_dispatch("event.start", "0")
  -- twophase 走 4 拍（锚间隔 4）：第 4 拍应有 KV 锚。
  for _ = 1, 4 do
    ygg_dispatch("tla.step", "twophase|")
  end
  local tick = ygg.kv.get("tla.tick.twophase")
  assert_eq(tick, 4, "四拍后 tick")
  local anchor = ygg.kv.get("tla.anchor.twophase.4")
  assert_true(type(anchor) == "string" and #anchor == 16, "锚为 16 位十六进制")
  assert_eq(anchor, ygg.kv.get("tla.hash.twophase.4"), "锚=第 4 拍链哈希")
  -- 第 2 拍无锚（非 4 的倍数）。
  assert_true(ygg.kv.get("tla.anchor.twophase.2") == nil, "非锚拍无锚键")
  return nil
end)

check("load.summary-event-meta", function()
  ygg_dispatch("event.start", "0")
  ygg_dispatch("tla.step", "gol|step")
  local summary = ygg_dispatch("tla.summary", nil)
  assert_true(type(summary) == "string" and summary:find("gol#1", 1, true) ~= nil, "摘要含 gol#1")
  assert_true(ygg_meta_all()["summary"] ~= nil, "meta.summary 已落")
  return nil
end)

check("load.rejection-lines-carry-prefix", function()
  ygg_dispatch("event.start", "0")
  ygg_dispatch("tla.reset", "nosuch")
  ygg_dispatch("tla.step", "nosuch|x")
  ygg_dispatch("tla.step", "gol|wrong-action")
  ygg_dispatch("tla.check", "nosuch")
  local hits = 0
  for _, line in ipairs(ygg_narrated_lines()) do
    if line:find("【拒绝】", 1, true) ~= nil then
      hits = hits + 1
    end
  end
  assert_eq(hits, 4, "四条拒绝叙事（未知模型/步/动作/查）")
  return nil
end)
