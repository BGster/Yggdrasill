-- src/proposals.lua —— 提案表 `prop.*` + `changes`（split / merge / amend / conflict）的落地规则。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 资源由入口 install(deps) 注入。本模块唯一写的 KV 前缀：`prop.*`（唯一写方）。

local M = {}

local config = require("src.config")
local reqs = require("src.requirements")
local util = require("src.util")

local TEXTS

local num, text_of =
  config.num, config.text_of
local R_MERGED, R_SPLIT, req_append, req_deps, req_id, req_index_of, req_key, req_push_history =
  reqs.R_MERGED, reqs.R_SPLIT, reqs.req_append, reqs.req_deps, reqs.req_id, reqs.req_index_of, reqs.req_key, reqs.req_push_history
local cut, fill, join, split, trim =
  util.cut, util.fill, util.join, util.split, util.trim

local P_PENDING = "pending"

local P_ACCEPTED = "accepted"

local P_REJECTED = "rejected"

local P_STALE = "stale"

local function prop_count()
  return num("prop.count", 0)
end

local function prop_key(index, name)
  return "prop." .. tostring(index) .. "." .. name
end

local function prop_id(index)
  return "p" .. tostring(index)
end

local function prop_status(index)
  local value = ygg.kv.get(prop_key(index, "status"))
  if type(value) == "string" then
    return value
  end
  return ""
end

local function prop_req(index)
  return num(prop_key(index, "req"), 0)
end

local function prop_rev(index)
  return num(prop_key(index, "rev"), 0)
end

local function prop_stage(index)
  return text_of(prop_key(index, "stage"), "")
end

local function prop_append(req_index, rev, stage_name, summary, artifacts, ext, changes)
  local index = prop_count() + 1
  ygg.kv.set("prop.count", index)
  ygg.kv.set(prop_key(index, "req"), req_index)
  ygg.kv.set(prop_key(index, "rev"), rev)
  ygg.kv.set(prop_key(index, "stage"), stage_name)
  ygg.kv.set(prop_key(index, "summary"), summary)
  ygg.kv.set(prop_key(index, "artifacts"), artifacts)
  ygg.kv.set(prop_key(index, "status"), P_PENDING)
  ygg.kv.set(prop_key(index, "ext"), ext or "")
  ygg.kv.set(prop_key(index, "changes"), changes or "")
  return index
end

local function prop_pending_total()
  local total = 0
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING then
      total = total + 1
    end
  end
  return total
end

local function prop_next()
  local best = nil
  for i = 1, prop_count() do
    if prop_status(i) == P_PENDING then
      if best == nil then
        best = i
      else
        local a = prop_req(i)
        local b = prop_req(best)
        if a < b or (a == b and i < best) then
          best = i
        end
      end
    end
  end
  return best
end

local function prop_by_ext(ext)
  if type(ext) ~= "string" or ext == "" then
    return nil
  end
  for i = 1, prop_count() do
    if ygg.kv.get(prop_key(i, "ext")) == ext then
      return i
    end
  end
  return nil
end

local function req_do_split(parent, children)
  local deps = req_deps(parent)
  local ids = {}
  for i = 1, #children do
    local child = req_append(children[i], fill(TEXTS.beats.req_split_desc, { id = req_id(parent) }), "", deps, req_id(parent))
    ids[#ids + 1] = req_id(child)
  end
  ygg.kv.set(req_key(parent, "status"), R_SPLIT)
  ygg.kv.set(req_key(parent, "children"), join(ids, ","))
  -- 拆分的父需求进终态：已批准的计划随之作废（子需求各自从头走 plan 门禁）。
  ygg.kv.set(req_key(parent, "plan_ok"), false)
  req_push_history(parent, fill(TEXTS.beats.req_split_hist, { ids = join(ids, ",") }))
  return ids
end

local function req_do_merge(sources, title, description)
  local seen = {}
  for i = 1, #sources do
    seen[sources[i]] = true
  end
  local dep_seen = {}
  local dep_order = {}
  for i = 1, #sources do
    for _, dep in ipairs(req_deps(sources[i])) do
      local dep_index = req_index_of(dep)
      if dep_index ~= nil then
        if not seen[dep_index] and not dep_seen[dep_index] then
          dep_seen[dep_index] = true
          dep_order[#dep_order + 1] = req_id(dep_index)
        end
      elseif not dep_seen[dep] then
        dep_seen[dep] = true
        dep_order[#dep_order + 1] = dep
      end
    end
  end
  local ids = {}
  for i = 1, #sources do
    ids[#ids + 1] = req_id(sources[i])
  end
  local next_title = title
  if next_title == "" then
    next_title = fill(TEXTS.beats.req_merge_title, { ids = join(ids, ",") })
  end
  local next_desc = description
  if next_desc == "" then
    next_desc = fill(TEXTS.beats.req_merge_desc, { ids = join(ids, ",") })
  end
  local target = req_append(next_title, next_desc, "", dep_order, "")
  for i = 1, #sources do
    local index = sources[i]
    ygg.kv.set(req_key(index, "status"), R_MERGED)
    ygg.kv.set(req_key(index, "into"), req_id(target))
    ygg.kv.set(req_key(index, "conflict"), "")
    ygg.kv.set(req_key(index, "conflict_peer"), "")
    ygg.kv.set(req_key(index, "conflict_note"), "")
    ygg.kv.set(req_key(index, "conflict_source"), "")
    ygg.kv.set(req_key(index, "job"), "")
    req_push_history(index, fill(TEXTS.beats.req_merge_hist, { id = req_id(target) }))
  end
  return target, next_title, dep_order
end

local function req_do_conflict(first, second, note, source)
  -- 冲突的唯一写方（人类 `requirement.conflict` 与反应器的跨泳道雷达共用）：
  -- `conflict` 是**注释文本**（`req_conflict_total` 的计数口径是 `~= ""`，不是 bool），
  -- `conflict_peer` 是对端 id；`conflict_note` / `conflict_source` 是纯记录键（"human" / "radar"）。
  local text = (note ~= nil and note ~= "") and note or TEXTS.beats.req_conflict_default
  local origin = (source ~= nil and source ~= "") and source or "human"
  ygg.kv.set(req_key(first, "conflict"), text)
  ygg.kv.set(req_key(first, "conflict_peer"), req_id(second))
  ygg.kv.set(req_key(first, "conflict_note"), text)
  ygg.kv.set(req_key(first, "conflict_source"), origin)
  ygg.kv.set(req_key(second, "conflict"), text)
  ygg.kv.set(req_key(second, "conflict_peer"), req_id(first))
  ygg.kv.set(req_key(second, "conflict_note"), text)
  ygg.kv.set(req_key(second, "conflict_source"), origin)
  req_push_history(first, TEXTS.beats.req_conflict_hist)
  req_push_history(second, TEXTS.beats.req_conflict_hist)
end

local function apply_changes(index, changes)
  if type(changes) ~= "string" or trim(changes) == "" then
    return ""
  end
  local kind, body = cut(trim(changes), "=")
  kind = trim(kind)
  body = trim(body or "")
  if kind == "split" then
    local children = {}
    for _, part in ipairs(split(body, ",")) do
      local piece = trim(part)
      if piece ~= "" then
        children[#children + 1] = piece
      end
    end
    if #children < 2 then
      return ""
    end
    local ids = req_do_split(index, children)
    return fill(TEXTS.beats.req_split_ok, { id = req_id(index), ids = join(ids, ","), deps = join(req_deps(index), ",") })
  elseif kind == "merge" then
    local other = req_index_of(body)
    if other == nil or other == index then
      return ""
    end
    local sources = { index, other }
    local target, title = req_do_merge(sources, "", "")
    return fill(TEXTS.beats.req_merge_ok, {
      ids = req_id(index) .. "," .. req_id(other),
      id = req_id(target),
      title = title,
    })
  elseif kind == "amend" then
    local title, description = cut(body, ":")
    title = trim(title)
    description = trim(description or "")
    if title ~= "" then
      ygg.kv.set(req_key(index, "title"), title)
    end
    if description ~= "" then
      ygg.kv.set(req_key(index, "description"), description)
    end
    return fill(TEXTS.beats.req_amend_ok_short, { id = req_id(index) })
  elseif kind == "conflict" then
    local peer_id, note = cut(body, ":")
    local peer = req_index_of(trim(peer_id or ""))
    if peer == nil or peer == index then
      return ""
    end
    req_do_conflict(index, peer, trim(note or "") ~= "" and trim(note) or TEXTS.beats.req_conflict_default)
    return fill(TEXTS.beats.req_conflict_ok_short, { first = req_id(index), second = req_id(peer) })
  end
  return ""
end

function M.install(deps)
  TEXTS = deps.texts
end

M.P_ACCEPTED = P_ACCEPTED
M.P_PENDING = P_PENDING
M.P_REJECTED = P_REJECTED
M.P_STALE = P_STALE
M.apply_changes = apply_changes
M.prop_append = prop_append
M.prop_by_ext = prop_by_ext
M.prop_count = prop_count
M.prop_id = prop_id
M.prop_key = prop_key
M.prop_next = prop_next
M.prop_pending_total = prop_pending_total
M.prop_req = prop_req
M.prop_rev = prop_rev
M.prop_stage = prop_stage
M.prop_status = prop_status
M.req_do_conflict = req_do_conflict
M.req_do_merge = req_do_merge
M.req_do_split = req_do_split

return M
