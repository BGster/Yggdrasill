-- 夹具冒烟 · 2 条：harness 底座在位 + 声明面已清空。
--
-- 包层退役后本夹具的被测面就是自己的 `pack.json`：`packages` 为空是期望态
-- （`PackagesSpec::parse` 对「缺键」与「空数组」等价，见
-- `crates/ygg-sandbox/src/packages/declare.rs`），所以这里用 `__PACK` 钉住
-- 「声明面确实是空的」，而不是钉「装载器拒了什么」——后者归
-- `crates/ygg-sandbox/tests/pack_packages.rs`。
check("harness_surface", function()
  if type(ygg) ~= "table" then
    return "harness 没装出 ygg 替身"
  end
  if type(ygg.on) ~= "function" then
    return "ygg.on 缺失"
  end
  local names = ygg_event_names()
  if #names ~= 0 then
    return "mod.lua 不应登记任何事件，实际：" .. table.concat(names, ",")
  end
end)
check("packages_declaration_is_empty", function()
  if type(__PACK) ~= "table" then
    return "__PACK 未注入（examples/selftest 的夹具契约）"
  end
  local packages = __PACK.packages
  if packages ~= nil and #packages ~= 0 then
    return "pack.json 的 packages 应为空，实际 " .. tostring(#packages) .. " 条"
  end
  if type(__PACK.entries) ~= "table" or #__PACK.entries ~= 1 then
    return "pack.json 应恰有 1 条资源条目"
  end
end)
