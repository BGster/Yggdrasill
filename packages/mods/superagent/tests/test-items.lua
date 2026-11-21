-- `tests/test-items.lua` —— 道具系统自检（C3）。
--
-- 断言面（计划 C3 验收「codec roundtrip 用例过；转移→事件→读数一致」）：
--   1. bag_of/holder_of 读数与铺场数据一致；
--   2. transfer：holder 改写 + item.transferred 恰一条（载荷三段）+ 转移账一行
--      + 读数一致（新持有者名下可见）；
--   3. 同主转移拒绝、未知 key 拒绝；
--   4. codec 往返：save→load 全量对账相等；转移后再存再读，档随最新状态。

local world = require("src.world")
local items = require("src.items")

if ygg.kv.get("sg.seeded") == nil then
  world.seed()
end

local function emitted_of(name)
  local out = {}
  local events = ygg_emitted_events()
  for i = 1, #events do
    if events[i].name == name then
      out[#out + 1] = events[i].payload
    end
  end
  return out
end

check("items.bag.readout-matches-seed", function()
  local problems = {}
  -- world.json：seal/dossier 在 archivist 名下。
  local bag = items.bag_of("archivist")
  if #bag ~= 2 then
    problems[#problems + 1] = "archivist 名下应 2 件（seal/dossier），实得 " .. tostring(#bag)
  end
  if items.holder_of("seal") ~= world.npc_id("archivist") then
    problems[#problems + 1] = "seal holder 应为 archivist 实体"
  end
  local name = ygg.entity.get_attribute(items.item_id("seal"), "name")
  if name ~= "局印" then
    problems[#problems + 1] = "属性袋读数（name）不符：" .. tostring(name)
  end
  return table.concat(problems, "; ")
end)

check("items.transfer.moves-holder-with-event-and-ledger", function()
  local problems = {}
  local before = #items.transfers()
  local result = items.transfer("dossier", "captain")
  if result.to ~= world.npc_id("captain") then
    problems[#problems + 1] = "转移目标实体不符"
  end
  if items.holder_of("dossier") ~= world.npc_id("captain") then
    problems[#problems + 1] = "转移后 holder 应指向 captain"
  end
  -- 读数一致：archivist 名下少一件，captain 名下多一件。
  if #items.bag_of("archivist") ~= 1 then
    problems[#problems + 1] = "archivist 名下应剩 1 件"
  end
  local captain_bag = items.bag_of("captain")
  local found = false
  for i = 1, #captain_bag do
    if captain_bag[i] == "dossier" then
      found = true
    end
  end
  if not found then
    problems[#problems + 1] = "captain 名下应见 dossier"
  end
  -- 事件恰一条，载荷三段（item|from|to）。
  local events = emitted_of("item.transferred")
  if #events ~= 1 then
    problems[#problems + 1] = "item.transferred 应恰一条，实得 " .. tostring(#events)
  else
    local segs = 1
    for _ in (tostring(events[1]) .. "|"):gmatch("([^|]*)|") do
      segs = segs + 1
    end
    if segs ~= 4 then
      problems[#problems + 1] = "载荷应为三段，实得 " .. tostring(segs - 1)
    end
  end
  -- 转移账 +1。
  if #items.transfers() ~= before + 1 then
    problems[#problems + 1] = "转移账应 +1"
  end
  return table.concat(problems, "; ")
end)

check("items.transfer.rejects-same-owner-and-unknown", function()
  local problems = {}
  local ok1, err1 = pcall(items.transfer, "dossier", "captain")
  if ok1 then
    problems[#problems + 1] = "同主转移应拒"
  elseif tostring(err1):find("YGG-MOD-ITEM-SAME", 1, true) == nil then
    problems[#problems + 1] = "同主拒绝码应为 YGG-MOD-ITEM-SAME，实得 " .. tostring(err1)
  end
  local ok2, err2 = pcall(items.transfer, "no-such-item", "captain")
  if ok2 then
    problems[#problems + 1] = "未知道具应拒"
  elseif tostring(err2):find("YGG-MOD-ITEM-UNKNOWN", 1, true) == nil then
    problems[#problems + 1] = "未知拒绝码应为 YGG-MOD-ITEM-UNKNOWN，实得 " .. tostring(err2)
  end
  local ok3, err3 = pcall(items.transfer, "seal", "no-such-npc")
  if ok3 then
    problems[#problems + 1] = "未知持有者应拒"
  elseif tostring(err3):find("YGG-MOD-ITEM-TARGET", 1, true) == nil then
    problems[#problems + 1] = "持有者拒绝码应为 YGG-MOD-ITEM-TARGET，实得 " .. tostring(err3)
  end
  return table.concat(problems, "; ")
end)

check("items.codec.roundtrip-preserves-bag", function()
  local problems = {}
  local saved = items.save_items("artifacts/biz/item-bag.json")
  if (saved.bytes or 0) == 0 then
    problems[#problems + 1] = "存档字节数应为正"
  end
  local report = items.load_items("artifacts/biz/item-bag.json")
  if report.mismatch ~= 0 then
    problems[#problems + 1] = "往返对账应零差异：" .. table.concat(report.problems, "; ")
  end
  if report.read ~= report.total then
    problems[#problems + 1] = "对账数应等于在册数：" .. tostring(report.read) .. "/" .. tostring(report.total)
  end
  -- 转移后再存再读：档随最新状态（holder 已是 captain 的那份）。
  items.transfer("seal", "clerk-mo")
  items.save_items("artifacts/biz/item-bag.json")
  local report2 = items.load_items("artifacts/biz/item-bag.json")
  if report2.mismatch ~= 0 then
    problems[#problems + 1] = "转移后往返应对账零差异：" .. table.concat(report2.problems, "; ")
  end
  return table.concat(problems, "; ")
end)
