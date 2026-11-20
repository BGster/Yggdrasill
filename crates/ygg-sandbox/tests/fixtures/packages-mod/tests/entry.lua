-- 硬断言冒烟（**0 个计入的用例**）。
--
-- 入口 `mod.lua` 顶层已经 `assert(fixture.greet("mod") == "hello, mod")`，装载失败
-- 会在那里就红。本文件只补一条「装载后缓存身份仍然成立」的硬断言，用 `error`
-- 而不是 `test()`：本夹具的被测面已经由包自测（3 条）与沙箱 e2e 覆盖，这里不再
-- 增计数，好让 `--packages` 的 `cases passed=` 精确等于包自测的条数。

local fixture = require("pkg.fixture")
assert(fixture.VERSION == "0.1.0", "包版本漂移：" .. tostring(fixture.VERSION))
assert(require("pkg.fixture") == fixture, "装载器缓存失效：两次 require 不是同一 table")
