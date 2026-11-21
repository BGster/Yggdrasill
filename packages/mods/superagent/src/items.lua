-- `src/items.lua` —— 道具系统：属性袋 + 转移语义 + codec 存档往返（模块顶层纯导出）。
--
-- # 职责（计划 §5 C3）
--
--   * **属性袋**：道具 = `pkg.entity` 的 `bureau.item` 实体（world.seed 已铺），
--     属性 name/category/weight + holder（ref → 持有者实体 id）。本文件是
--     道具语义（转移/读数/存档）的唯一出口，不重复建实体。
--   * **转移**：`transfer(item_key, to_npc_key)`——holder ref 改写 + 派生事件
--     `item.transferred`（载荷标量 `item|from|to`，实体 id 逗号前用 key 可读）。
--     转移只改 holder，不动其余属性（部分转移/堆叠 = 计划 §9 不做清单）。
--   * **codec 往返**：`save_items(path)` 把道具袋编码落盘（`pkg.codec.write`，
--     带 checksum）→ `load_items(path)` 读回并**逐道具**与在册实体对账
--     （codec.read 的强制 checksum 是越信任边界的第一道门，对账是第二道）。
--
-- # KV（前缀 sg.）
--
--   sg.items            换行道具 key 名表（升序；world.seed 写入，此处只读）
--   sg.transfers        换行转移账（"<item>|<from-key>|<to-key>"，升序追加）

local pkg_codec = require("pkg.codec")

local M = { VERSION = "1.0.0" }

local K_ITEMS = "sg.items"
local K_TRANSFERS = "sg.transfers"

local function host()
  return ygg
end

--- 名表切分（与 world/quests 同语义；避免跨模块循环依赖）。
local function name_table(text)
  local out = {}
  if type(text) ~= "string" or #text == 0 then
    return out
  end
  for key in (text .. "\n"):gmatch("([^\n]*)\n") do
    if #key > 0 then
      out[#out + 1] = key
    end
  end
  return out
end

--- 道具 key 名表（升序，world.seed 写入的名表直读）。
function M.keys()
  return name_table(host().kv.get(K_ITEMS))
end

--- 实体 id → npc key（world 的 sg.npc.<key> 反查；找不到返回 nil）。
function M.npc_key_of(id)
  local h = host()
  local npcs = name_table(h.kv.get("sg.npcs"))
  for i = 1, #npcs do
    if h.kv.get("sg.npc." .. npcs[i]) == id then
      return npcs[i]
    end
  end
  return nil
end

--- `item_id(key)` / `holder_of(item_key)` → 实体 id | nil。
function M.item_id(key)
  return host().kv.get("sg.item." .. key)
end

--- `holder_of(item_key)` → 持有者实体 id | nil。
function M.holder_of(item_key)
  local id = M.item_id(item_key)
  if id == nil then
    return nil
  end
  return host().entity.get_attribute(id, "holder")
end

--- `bag_of(npc_key)` → 该持有者名下道具 key 数组（升序；读数面）。
function M.bag_of(npc_key)
  local h = host()
  local target = h.kv.get("sg.npc." .. npc_key)
  local out = {}
  local keys = M.keys()
  for i = 1, #keys do
    if M.holder_of(keys[i]) == target then
      out[#out + 1] = keys[i]
    end
  end
  return out
end

--- 转移账追加（"<item>|<from-key>|<to-key>"；升序写，读数确定性）。
local function append_transfer(line)
  local h = host()
  local rows = {}
  local raw = h.kv.get(K_TRANSFERS)
  if type(raw) == "string" and #raw > 0 then
    for row in (raw .. "\n"):gmatch("([^\n]*)\n") do
      if #row > 0 then
        rows[#rows + 1] = row
      end
    end
  end
  rows[#rows + 1] = line
  for i = 1, #rows do
    local min = i
    for j = i + 1, #rows do
      if rows[j] < rows[min] then
        min = j
      end
    end
    if min ~= i then
      rows[i], rows[min] = rows[min], rows[i]
    end
  end
  local out = ""
  for i = 1, #rows do
    if i > 1 then
      out = out .. "\n"
    end
    out = out .. rows[i]
  end
  h.kv.set(K_TRANSFERS, out)
end

--- `transfer(item_key, to_npc_key)` → `{item, from, to}`（实体 id 对）。
---
--- 同主转移（from == to）是显式拒绝（事件账只记真实移动）；未知 key 报错。
function M.transfer(item_key, to_npc_key)
  local h = host()
  local item_id = M.item_id(item_key)
  if item_id == nil then
    error("YGG-MOD-ITEM-UNKNOWN: 未知道具 key：" .. tostring(item_key), 0)
  end
  local to_id = h.kv.get("sg.npc." .. tostring(to_npc_key))
  if to_id == nil then
    error("YGG-MOD-ITEM-TARGET: 未知持有者 key：" .. tostring(to_npc_key), 0)
  end
  local from_id = h.entity.get_attribute(item_id, "holder")
  if from_id == to_id then
    error("YGG-MOD-ITEM-SAME: 道具已在 " .. tostring(to_npc_key) .. " 名下", 0)
  end
  h.entity.set_attribute(item_id, "holder", to_id)
  append_transfer(item_key .. "|" .. tostring(M.npc_key_of(from_id) or from_id) .. "|" .. tostring(to_npc_key))
  h.emit("item.transferred", item_key .. "|" .. tostring(from_id) .. "|" .. tostring(to_id))
  return { item = item_id, from = from_id, to = to_id }
end

--- 转移账读数（升序行数组）。
function M.transfers()
  return name_table(host().kv.get(K_TRANSFERS))
end

-- ── codec 往返（存档 → 读回对账）────────────────────────────────────────────

--- `bag_bundle()` → 道具袋 bundle（纯数据；encode 形状由 codec 钉死）。
function M.bag_bundle()
  local h = host()
  local keys = M.keys()
  local items = {}
  for i = 1, #keys do
    local id = M.item_id(keys[i])
    items[keys[i]] = {
      name = h.entity.get_attribute(id, "name"),
      category = h.entity.get_attribute(id, "category"),
      weight = h.entity.get_attribute(id, "weight"),
      holder = h.entity.get_attribute(id, "holder"),
    }
  end
  return { v = 1, items = items }
end

--- `save_items(path)` → `{bytes, sha256}`（codec.write 带 checksum 落盘）。
function M.save_items(path)
  return pkg_codec.write(path, M.bag_bundle())
end

--- `load_items(path)` → `{read=n, mismatch=k, problems={…}}`：读回并对账。
---
--- 对账口径：每个在册 key 的四属性与 bundle 逐项相等；bundle 里多出的 key
--- 记 mismatch（在册少 = 漏铺，多 = 脏档）。
function M.load_items(path)
  local h = host()
  local bundle = pkg_codec.read(path)
  local read_bundle = bundle
  local problems = {}
  local matched = 0
  local keys = M.keys()
  local items = read_bundle.items or {}
  for i = 1, #keys do
    local key = keys[i]
    local id = M.item_id(key)
    local live = {
      name = h.entity.get_attribute(id, "name"),
      category = h.entity.get_attribute(id, "category"),
      weight = h.entity.get_attribute(id, "weight"),
      holder = h.entity.get_attribute(id, "holder"),
    }
    local saved = items[key]
    if saved == nil then
      problems[#problems + 1] = key .. ": 档里缺失"
    else
      local ok = true
      local fields = { "name", "category", "weight", "holder" }
      for f = 1, #fields do
        if live[fields[f]] ~= saved[fields[f]] then
          ok = false
        end
      end
      if ok then
        matched = matched + 1
      else
        problems[#problems + 1] = key .. ": 属性不符"
      end
    end
  end
  return { read = matched, total = #keys, mismatch = #problems, problems = problems }
end

return M
