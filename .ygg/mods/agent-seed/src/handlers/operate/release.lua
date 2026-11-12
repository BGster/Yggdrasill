-- src/handlers/operate/release.lua —— 发布幕两事件：release.confirm / release.skip。
--
-- 纯导出模块：顶层只做 require / 定义（不注册、不写状态、不输出）；
-- 注册面全部在 install() 内（派发序 = 入口 install 调用序）。
-- R5：两事件都不进票据门（由主代理直发，与 settlement.* 同口径）。
-- R-a/R10：两者任一成立即「发布已声明」，是 review.note（批结算 / 退役）的前置。

local M = {}

local config = require("src.core.config")
local flow = require("src.runtime.flow")
local render_mod = require("src.view.render")
local reqs = require("src.domain.requirements")
local util = require("src.core.util")

local TEXTS

local num =
  config.num
local phase_guard, reject_payload =
  flow.phase_guard, flow.reject_payload
local render =
  render_mod.receipt
local RUN_PHASE =
  reqs.RUN_PHASE
local cut, fill, trim =
  util.cut, util.fill, util.trim

-- master sha 形状：7-40 位十六进制（`master@` 前缀内可带）。
local function master_sha_ok(sha)
  return #sha >= 7 and #sha <= 40 and sha:match("^%x+$") ~= nil
end

-- 批次标签形状：`v` + 年.月.日.序号四段数字点分（vYYYY.MM.DD.N）。
local function release_tag_ok(tag)
  return tag:match("^v%d+%.%d+%.%d+%.%d$") ~= nil
end

-- 声明互斥：confirm 清 skip 理由、skip 清 sha/标签——「发布已声明」是单声明，
-- 不能同时挂着两条互相矛盾的声明。
local function reset_release()
  ygg.kv.set("run.release.master_sha", "")
  ygg.kv.set("run.release.tag", "")
  ygg.kv.set("run.release.skipped_reason", "")
end

function M.install(deps)
  TEXTS = deps.texts
  -- 声明本批 develop→master 合并结果：`master@<sha>|vYYYY.MM.DD.N`。
  ygg.on("release.confirm", function(payload)
    if not phase_guard("release.confirm", { RUN_PHASE.release }) then
      return
    end
    if type(payload) ~= "string" then
      return reject_payload("release.confirm", fill(TEXTS.beats.bad_payload, { action = "release.confirm" }))
    end
    local evidence, tag = cut(trim(payload), "|")
    evidence = trim(evidence or "")
    tag = trim(tag or "")
    local sha = evidence
    if sha:sub(1, 7) == "master@" then
      sha = trim(sha:sub(8))
    end
    if not master_sha_ok(sha) or not release_tag_ok(tag) then
      return reject_payload("release.confirm", TEXTS.beats.release_usage)
    end
    reset_release()
    ygg.kv.set("run.release.master_sha", sha)
    ygg.kv.set("run.release.tag", tag)
    ygg.narrate(fill(TEXTS.beats.release_confirmed, {
      sha = sha,
      tag = tag,
      batch = tostring(num("run.batch", 1)),
    }))
    render()
  end)
  -- 本批无需发布的显式声明：非空理由必填（R10）。
  ygg.on("release.skip", function(payload)
    if not phase_guard("release.skip", { RUN_PHASE.release }) then
      return
    end
    local reason = trim(tostring(payload or ""))
    if reason == "" then
      return reject_payload("release.skip", TEXTS.beats.release_skip_usage)
    end
    reset_release()
    ygg.kv.set("run.release.skipped_reason", reason)
    ygg.narrate(fill(TEXTS.beats.release_skipped, {
      reason = reason,
      batch = tostring(num("run.batch", 1)),
    }))
    render()
  end)
end

M.reset_release = reset_release

return M
