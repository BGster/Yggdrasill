-- modpkg：Mod 包结构与文本卫生（迁移自 Rust 对照套件 54–66 例，分批搬）。

test("mod 文本片段不含退休路径字面量", function()
  local fragments = {}
  for _, entry in ipairs(__PACK.entries) do
    local path = entry.path
    if path:sub(1, 11) == "data/texts/" then
      fragments[#fragments + 1] = path
    end
  end
  assert_true(#fragments > 0, "scope guard: pack.json must declare texts fragments")

  local violations = {}
  for _, path in ipairs(fragments) do
    local handle = assert(io.open(__MOD_DIR .. "/" .. path, "r"))
    local line_no = 0
    for line in handle:lines() do
      line_no = line_no + 1
      local retired = line:find("ygg-agent", 1, true) or line:find("pi-agent-seed", 1, true)
      if retired and not line:find("[deprecated-ref]", 1, true) then
        violations[#violations + 1] = path .. ":" .. line_no
      end
    end
    handle:close()
  end
  assert_eq(#violations, 0, "retired path literals: " .. table.concat(violations, ", "))
end)

test("pack.json 不引用退休的二进制名", function()
  local raw = assert(io.open(__MOD_DIR .. "/pack.json", "r")):read("*a")
  assert_nil(raw:find("ygg-agent", 1, true), "pack.json must not mention the retired `ygg-agent`")
end)
