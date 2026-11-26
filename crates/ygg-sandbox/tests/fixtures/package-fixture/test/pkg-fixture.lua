-- 包自测（`pkg.fixture.*`）：**不进模块表、不进 manifest**。
--
-- 由 `examples/selftest --packages <包根>` 收集执行，与 mod 的 `tests/*.lua` 走
-- 同一个 `__SELFTEST` 计数面（`cases passed=` 里一并计）。包不应该依赖消费它的
-- mod 的 harness，所以这里自带 8 行计数垫片：宿主给了 `__SELFTEST` 就用它，没给
-- 就自己建一个——两种情形下用例都被计入同一次汇总。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function check(name, ok, detail)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. (detail and (": " .. tostring(detail)) or ""))
  end
end

-- `pkg.fixture.greet`：包根模块可 require，且导出真的可调用。
check("pkg.fixture.greet", require("pkg.fixture").greet(" ygg ") == "hello, ygg")

-- `pkg.fixture.submodule`：包内子模块解析（同一份包索引，不是路径拼接）。
check("pkg.fixture.submodule", require("pkg.fixture.words").shout("hi") == "HI!")

-- `pkg.fixture.cache-identity`：装载器缓存生效——两次 require 是同一个 table。
check(
  "pkg.fixture.cache-identity",
  require("pkg.fixture") == require("pkg.fixture") and require("pkg.fixture").VERSION == "0.1.0"
)
