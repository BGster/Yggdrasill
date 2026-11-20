-- `packages/libs/assetgen/join.lua` —— 字符串两两合并（沙箱里 `table.concat` 不可达）。
--
-- 契约：`docs/design/packages/assetgen.md` §2 / §8。
--
-- # 为什么不写 `out = out .. chunk`
--
-- Lua 的字符串不可变，逐段追加是 O(n²)：树内实测 1.6 MB 的 bundle 跑了 43 秒
-- （codec.md §8）。两两合并是 O(n·log n)，且只在最后一轮触碰整串。纹理按行、
-- PCM 按 256 样本、OBJ 按顶点块分块后都走这里——**唯一的拼接入口**。
--
-- 每个包各自持有这几行（codec / quest / entity / dialogue 同一先例）：为一个
-- 十几行的工具多开一个包，换来的是多一条依赖边、多一份契约、多一格计数。

local M = {}

--- 合并数组里的字符串。**原地覆写**入参：调用方交出 parts 的所有权。
function M.concat(parts)
  local n = #parts
  if n == 0 then
    return ""
  end
  local width = 1
  while width < n do
    local index = 1
    while index <= n do
      local right = parts[index + width]
      if right ~= nil then
        parts[index] = parts[index] .. right
      end
      index = index + width * 2
    end
    width = width * 2
  end
  return parts[1]
end

return M
