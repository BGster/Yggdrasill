-- `pkg.fixture.words`：子模块（证明包内多文件解析走同一份包索引）。
--
-- 它是夹具包内的 `words.lua`，模块名由「相对包根的路径」推导：
-- `words.lua` → `pkg.fixture.words`。chunk 名报的是 `@fixture/words.lua`
-- （相对包根的真实路径），不是 `path_of` 会推导出的 `pkg/fixture/words.lua`。

local M = {}

--- 去掉首尾空白。
function M.trim(s)
  return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

--- 全大写并加感叹号。
function M.shout(s)
  return s:upper() .. "!"
end

return M
