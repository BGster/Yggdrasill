-- packages 夹具入口。三层证据的第 1 层就在这里：**入口顶层**即可 require 包模块，
-- 走的是与沙箱完全同一条装载路径（模块表在入口 chunk 之前注入）。
--
-- 这条 `assert` 不是装饰：它让「包装不上」在入口装载期就红，而不是等到某个
-- handler 被派发才暴露。`--packages` 指错包根时，`selftest` 会在这里失败而不是
-- 静默跑过零条包用例。

local fixture = require("pkg.fixture") -- 包根模块（多文件包：内部再 require words）
assert(fixture.greet("mod") == "hello, mod", "包模块没能装上或导出不对")

local probe = require("src.probe") -- 本 Mod 自有模块（同一张表，前缀不同）

-- 沙箱面：handler 里再 require 一次（懒加载走同一份缓存与索引）。
ygg.on("probe.pkg", function()
  ygg.narrate(fixture.greet("sandbox"))
end)

-- 顺带证明包与 mod 模块共存不互相干扰。
probe.install()
