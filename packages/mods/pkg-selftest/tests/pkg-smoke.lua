-- 夹具冒烟（2 条）：证明「包装载面 + 顶层纯度」两条**装载契约**在真实 selftest 里成立。
--
-- 它们不是能力用例：能力用例在 `packages/libs/<id>/test/*.lua`，由包自己带 mock。
-- 这两条只问两件事——
--   ① 两个包能通过 `package.searchers` 的包索引被装载，并导出可调用面；
--   ② 包的模块顶层**没有**任何 `ygg` 成员访问（契约 I7 / codec I8 的最硬形态：
--      把 `ygg` 换成一个「一访问就报错」的代理，`require` 仍然成功）。
--
-- 第 ② 条的判据是**反证**：不是「我没在代码里写 ygg」，而是「运行期换掉 ygg 后
-- require 依然跑通」。前者是静态印象，后者是可复算的事实。

--- ① 装载面：两个包都 require 得到表，且导出面可调用。
test("pkg-smoke.require-and-export", function()
  local entity = require("pkg.entity")
  local codec = require("pkg.codec")
  assert_eq(type(entity), "table", "pkg.entity 必须是表")
  assert_eq(type(codec), "table", "pkg.codec 必须是表")
  assert_eq(type(entity.install), "function", "pkg.entity.install 必须可调用")
  assert_eq(type(codec.install), "function", "pkg.codec.install 必须可调用")
  assert_eq(type(entity.VERSION), "string", "pkg.entity.VERSION 必须是字符串")
  assert_eq(type(codec.VERSION), "string", "pkg.codec.VERSION 必须是字符串")
  -- codec 单向依赖 entity（README §3 的边）：反向不允许存在。
  assert_true(rawget(entity, "codec") == nil, "pkg.entity 不得持有 codec（依赖序单向）")
end)

--- ② 顶层纯度：清缓存 + 换掉 `ygg` + 重新 require，任何 ygg 访问都该炸。
test("pkg-smoke.top-level-pure", function()
  local saved = rawget(_G, "ygg")
  local touched = nil
  _G.ygg = setmetatable({}, {
    __index = function(_, key)
      touched = tostring(key)
      error("模块顶层访问了 ygg." .. touched, 0)
    end,
  })
  package.loaded["pkg.entity"] = nil
  package.loaded["pkg.codec"] = nil
  local ok, err = pcall(require, "pkg.entity")
  if ok then
    ok, err = pcall(require, "pkg.codec")
  end
  _G.ygg = saved
  -- 复原缓存：后续用例的 require 必须拿到**干净**装载（不是残留的半截模块）。
  package.loaded["pkg.entity"] = nil
  package.loaded["pkg.codec"] = nil
  assert_true(ok, "require 触发了 ygg 访问：" .. tostring(err))
  assert_true(touched == nil, "模块顶层访问了 ygg." .. tostring(touched))
end)
