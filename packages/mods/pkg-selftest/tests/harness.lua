-- `packages/mods/pkg-selftest/tests/harness.lua` —— 夹具自己的最小计数垫片。
--
-- 它**不属于任何包**：包自测(`packages/libs/<id>/test/*.lua`)必须自带 mock 与计数，
-- 否则「包不依赖消费它的 mod」那条纪律当场破（`packages/README.md` §5）。这份
-- harness 只服务夹具自己的两条冒烟用例。
--
-- 与 `agent-seed` 的 harness 有刻意的三处不同：
--   1. 不注册任何 handler、不 mock 业务域（夹具没有业务域）；
--   2. 不读盘、不起子进程（`io.popen` / `os.getenv` 一概不用）——夹具要在任何
--      沙箱外环境跑得动；
--   3. 计数面与包自测**同一个** `__SELFTEST`（`examples/selftest` 的 `counters()`
--      只认这一张表），否则夹具的 2 条会掉出 `cases passed=`。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

--- 记一条用例的结果。
function check(name, ok, detail)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. (detail and (": " .. tostring(detail)) or ""))
  end
end

--- 值相等（`math.type` 也比：`1` 与 `1.0` 在序列化面不是同一个字节）。
function assert_eq(actual, expected, label)
  if actual ~= expected or math.type(actual) ~= math.type(expected) then
    error(
      ("%s: expected %s (%s), got %s (%s)"):format(
        label or "assert_eq",
        tostring(expected),
        tostring(math.type(expected)),
        tostring(actual),
        tostring(math.type(actual))
      ),
      2
    )
  end
end

--- 真值断言。
function assert_true(value, label)
  if not value then
    error((label or "assert_true") .. ": expected truthy, got " .. tostring(value), 2)
  end
end

--- 报错文案断言：`fn` 必须抛错，且错误文本包含 `needle`。
function assert_error(needle, fn, label)
  local ok, err = pcall(fn)
  if ok then
    error((label or "assert_error") .. ": expected an error containing " .. needle, 2)
  end
  if not tostring(err):find(needle, 1, true) then
    error(
      ("%s: expected error containing %s, got %s"):format(
        label or "assert_error",
        needle,
        tostring(err)
      ),
      2
    )
  end
end

--- 收一条用例：`fn` 不抛错即通过（抛错则记 fail 并把错误文本写进明细）。
function test(name, fn)
  local ok, err = pcall(fn)
  check(name, ok, ok and nil or err)
end
