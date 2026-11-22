-- src/runtime/tools/auto.lua —— S32 纯搬移自 tools.lua（零逻辑变更；接线见交付回执）。

local M = {}
local config = require("src.core.config")
local lanes = require("src.domain.lanes")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")
local state = require("src.runtime.tools.state")
local TEXTS = setmetatable({}, { __index = function(_, k) return state.TEXTS[k] end })
local mod_parse = require("src.runtime.tools.parse")
local mod_sync = require("src.runtime.tools.sync")
local num = config.num
local text_of = config.text_of
local LANES = lanes.LANES
local req_id = reqs.req_id
local req_key = reqs.req_key
local req_lane = reqs.req_lane
local req_release = reqs.req_release
local contains = util.contains
local fill = util.fill
local allowed, whole = mod_parse.allowed, mod_parse.whole
local try_sync_call = mod_sync.try_sync_call


local function auto_claim_worktree(pool, lane, req_index)
  local k = whole(lane)
  if k == nil or k <= 0 or req_index == nil then
    return nil
  end
  local id = req_id(req_index)
  -- r2 rev2：默认分支名生成机制已删除——分支由主 Agent 预挂牌自报（账本 `lane.<pool>.<k>.branch`），
  -- 空 = 可读拒绝不发起宿主调用（宿主先执行、反应器后折叠，发空名调用会让宿主 git 报错，
  -- 叙事不可读——必须在 mod 层拦）。
  local branch = text_of(LANES.key(pool, k, "branch"), "")
  if branch == "" then
    ygg.narrate(fill(TEXTS.beats.worktree_claim_refused, {
      lane = LANES.label(pool, k),
      reason = fill(TEXTS.beats.lane_branch_invalid, { branch = "(missing)" }),
    }))
    return nil
  end
  -- r1 分池：pool == "" 走默认工具（宿主以 CWD toplevel 落位）；具名池走 *_pool 工具，
  -- repo 取注册表路径；路径缺失 = 未注册/注册不全 → 可读拒绝，不发起宿主调用。
  local tool_id = "git.worktree_claim"
  local args_map = { lane = tostring(k), req = id, branch = branch }
  local raw_args = "lane=" .. tostring(k) .. ";req=" .. id .. ";branch=" .. branch
  if pool ~= "" then
    local repo = LANES.pool_path(pool)
    if repo == "" then
      ygg.narrate(fill(TEXTS.beats.worktree_claim_refused, {
        lane = LANES.label(pool, k),
        reason = fill(TEXTS.beats.lane_pool_path_missing, { pool = pool }),
      }))
      return nil
    end
    tool_id = "git.worktree_claim_pool"
    args_map.repo = repo
    raw_args = raw_args .. ";repo=" .. repo
  end
  if not contains(allowed(), tool_id) then
    return nil
  end
  return try_sync_call(tool_id, args_map, raw_args, "", "")
end

local function auto_probe_lane(pool, lane)
  local k = whole(lane)
  if k == nil or k <= 0 then
    return nil
  end
  local base = text_of(LANES.key(pool, k, "base"), "HEAD")
  if base == "" then
    base = "HEAD"
  end
  local branch = text_of(LANES.key(pool, k, "branch"), "")
  if branch == "" then
    -- r2 rev2：不再用 `feature/<holder>` 兑底虚构名字；账本分支空 = 可读提示并跳过探针
    -- （探针是警告面，不阻塞——但也不该拿编造的分支名去探测）。
    ygg.narrate(fill(TEXTS.beats.lane_branch_invalid, { branch = "(missing)" }))
    return nil
  end
  local tool_id = "git.lane_probe"
  local args_map = { lane = tostring(k), base = base, branch = branch }
  local raw_args = "lane=" .. tostring(k) .. ";base=" .. base .. ";branch=" .. branch
  if pool ~= "" then
    local repo = LANES.pool_path(pool)
    if repo == "" then
      ygg.narrate(fill(TEXTS.beats.lane_pool_path_missing, { pool = pool }))
      return nil
    end
    tool_id = "git.lane_probe_pool"
    args_map.repo = repo
    raw_args = raw_args .. ";repo=" .. repo
  end
  if not contains(allowed(), tool_id) then
    return nil
  end
  return try_sync_call(tool_id, args_map, raw_args, "", "")
end

local function auto_release_worktree(pool, lane)
  local k = whole(lane)
  if k == nil or k <= 0 then
    return nil
  end
  local tool_id = "git.worktree_release"
  local args_map = { lane = tostring(k) }
  local raw_args = "lane=" .. tostring(k)
  if pool ~= "" then
    local repo = LANES.pool_path(pool)
    if repo == "" then
      ygg.narrate(fill(TEXTS.beats.worktree_release_refused, {
        reason = fill(TEXTS.beats.lane_pool_path_missing, { pool = pool }),
      }))
      return nil
    end
    tool_id = "git.worktree_release_pool"
    args_map.repo = repo
    raw_args = raw_args .. ";repo=" .. repo
  end
  if not contains(allowed(), tool_id) then
    return nil
  end
  return try_sync_call(tool_id, args_map, raw_args, "", "")
end

-- R7 泳道释放（连工作树停车）：交付受理 reject / 验收门禁 reject / 集成 confirm·reject / done
-- 四个释放点共用——先用 (池, lane 号) 唤宿主 git.worktree_release 停车，再清 KV 账本。
-- 泳道绑定窗口保持到这些点才释放（交付受理→验收门禁→集成三幕期间不释放，执行者要工作树做集成）。
local function release_lane_with_worktree(index)
  local lane_idx = req_lane(index)
  local pool = LANES.req_pool(index)
  if lane_idx <= 0 then
    lane_idx = num(req_key(index, "last_lane"), 0)
    pool = text_of(req_key(index, "last_lane_pool"), "")
  end
  if lane_idx > 0 then
    auto_release_worktree(pool, lane_idx)
    ygg.kv.set(req_key(index, "last_lane"), 0)
    ygg.kv.set(req_key(index, "last_lane_pool"), "")
  end
  req_release(index)
end

M.auto_claim_worktree = auto_claim_worktree
M.auto_probe_lane = auto_probe_lane
M.auto_release_worktree = auto_release_worktree
M.release_lane_with_worktree = release_lane_with_worktree

return M
