-- `src.probe`：本 Mod 自有的一个普通模块，用来证明 `code` 段与 `packages` 段
-- 共存时进的是同一张模块表（`src.*` 与 `pkg.*` 只有前缀不同）。
--
-- 纯导出纪律：注册只发生在 `install()` 里，不在模块顶层。

local M = {}

function M.install()
  ygg.on("probe.mod", function()
    ygg.narrate("mod module ok")
  end)
end

return M
