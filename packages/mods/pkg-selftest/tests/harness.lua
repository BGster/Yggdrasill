-- `packages/mods/pkg-selftest/tests/harness.lua` —— 夹具自己的计数垫片。
--
-- 它**不属于任何包**：包自测（`packages/libs/<id>/test/*.lua`）必须自带 mock 与计数，
-- 否则「包不依赖消费它的 mod」那条纪律当场破（`packages/README.md` §5）。这份
-- harness 只服务夹具自己的两条冒烟用例。
--
-- 与业务 Mod 的 harness 有刻意的三处不同：
--   1. 不注册业务 handler、不 mock 业务域（夹具没有业务域）；只补一个**只够装配**
--      的 `ygg` 替身（见文件末「最小 ygg 替身」）；
--   2. 不读盘、不起子进程（`io.popen` / `os.getenv` 一概不用）——夹具要在任何
--      沙箱外环境跑得动；
--   3. 计数面与包自测**同一个** `__SELFTEST`（`examples/selftest` 的 `counters()`
--      只认这一张表），否则夹具的 2 条会掉出 `cases passed=`。
--
-- 接口只有一个：`check(name, body)`，`body` 返回**问题清单**（空串/nil = 通过）
-- 或直接抛错。这样一条用例的判定与它的证据写在同一个函数里，没有「先算布尔再
-- 拼文案」的中间态。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

--- 记一条用例：`body()` 返回非空字符串（或 nil 以外的问题清单）即失败。
function check(name, body)
  local ok, verdict = pcall(body)
  local detail = nil
  if not ok then
    detail = verdict
  elseif type(verdict) == "string" and #verdict > 0 then
    detail = verdict
  end
  if detail == nil then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(detail))
  end
end

-- ── 最小 `ygg` 替身（普通 Lua 里的装配面）───────────────────────────────
--
-- `mod.lua` 在真沙箱里跑时 `ygg` 由完整宿主面提供（`crates/ygg-sandbox/src/host_api.rs`）；
-- 普通 Lua 自检里没有它，所以这里补一个**只够装配**的替身：`on` 收 handler 进表（不派发）、
-- `narrate` 收行进缓冲、`emit` 照 `value_from_lua` 的口径**只收标量**。
--
-- 第三条是有意的严格：普通 Lua 的 mock 若吃表，role-skill 那类「表载荷」缺陷在这一面
-- 就永远测不出来（真内核报 `unsupported payload type: table`）。真值面仍然是
-- `pack_packages.rs` 的真沙箱用例——这个替身只是在装载期给 `ygg.on` 一个落点。

local __ygg_handlers = {}
local __ygg_narrated = {}
local __ygg_emitted = {}

ygg = {
  engine_version = "selftest",
  on = function(name, handler)
    assert(type(name) == "string" and #name > 0, "ygg.on 需要非空事件名")
    assert(type(handler) == "function", "ygg.on 需要 handler 函数")
    local bucket = __ygg_handlers[name]
    if bucket == nil then
      bucket = {}
      __ygg_handlers[name] = bucket
    end
    bucket[#bucket + 1] = handler
  end,
  narrate = function(text)
    __ygg_narrated[#__ygg_narrated + 1] = tostring(text)
  end,
  emit = function(name, payload)
    local kind = type(payload)
    assert(
      kind ~= "table" and kind ~= "function",
      "ygg.emit 的载荷只收标量（与 value_from_lua 同口径），得到 " .. kind
    )
    __ygg_emitted[#__ygg_emitted + 1] = { name = name, payload = payload }
  end,
}

--- 已登记的事件名（升序；装载面断言用）。
function ygg_event_names()
  local names = {}
  for name in pairs(__ygg_handlers) do
    names[#names + 1] = name
  end
  table.sort(names)
  return names
end

--- 已登记 handler 的条数（`on` 能被调几次的观测面）。
function ygg_handler_count(name)
  local bucket = __ygg_handlers[name]
  return bucket == nil and 0 or #bucket
end

--- 已收集的叙事行副本。
function ygg_narrated_lines()
  local out = {}
  for i = 1, #__ygg_narrated do
    out[i] = __ygg_narrated[i]
  end
  return out
end

--- 已收集的派生事件副本。
function ygg_emitted_events()
  local out = {}
  for i = 1, #__ygg_emitted do
    out[i] = { name = __ygg_emitted[i].name, payload = __ygg_emitted[i].payload }
  end
  return out
end
