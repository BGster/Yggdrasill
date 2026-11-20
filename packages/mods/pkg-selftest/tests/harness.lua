-- `packages/mods/pkg-selftest/tests/harness.lua` —— 夹具自己的计数垫片。
--
-- 它**不属于任何包**：包自测（`packages/libs/<id>/test/*.lua`）必须自带 mock 与计数，
-- 否则「包不依赖消费它的 mod」那条纪律当场破（`packages/README.md` §5）。这份
-- harness 只服务夹具自己的两条冒烟用例。
--
-- 与 `agent-seed` 的 harness 有刻意的三处不同：
--   1. 不注册任何 handler、不 mock 业务域（夹具没有业务域）；
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
