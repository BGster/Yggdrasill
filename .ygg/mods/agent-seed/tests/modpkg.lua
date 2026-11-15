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

test("seed 布局单源：无旧名残留、软链相对、manifest 字节数一致", function()
  local repo = __REPO_ROOT

  local legacy = io.open(repo .. "/.ygg/mods/pi-agent-seed/pack.json", "r")
  assert_nil(legacy, "`.ygg/mods/pi-agent-seed` must not exist after the rename")

  local pipe = assert(io.popen("readlink " .. repo .. "/packages/mods/agent-seed"))
  local target = pipe:read("*l")
  pipe:close()
  assert_eq(target, "../../.ygg/mods/agent-seed", "the seed symlink must stay relative")

  for _, section in ipairs({ "code", "entries" }) do
    local items = __MANIFEST[section]
    assert_true(type(items) == "table", ("manifest.%s must be an array"):format(section))
    for _, item in ipairs(items) do
      local path = item.path
      local recorded = item.byteLength
      local handle = assert(io.open(__MOD_DIR .. "/" .. path, "rb"))
      local actual = handle:seek("end")
      handle:close()
      assert_eq(recorded, actual, ("manifest byteLength drift for %s"):format(path))
    end
  end
end)

test("五层架构：模块只住在五层内、entry 不在模块表、声明文件都在盘上", function()
  local code = __PACK.code
  local modules = code.modules
  assert_true(type(modules) == "table", "code.modules must be an array")
  assert_eq(#modules, 30, "the mod declares 30 modules")

  local entry = code.entry
  local layers = {}
  for _, path in ipairs(modules) do
    assert_true(path ~= entry, "the entry must not appear in code.modules")
    local rest = path:match("^src/(.*)$")
    assert_true(rest ~= nil, ("%s: every module must live under src/"):format(path))
    local layer = rest:match("^([^/]+)/")
    assert_true(
      layer == "core" or layer == "domain" or layer == "view" or layer == "runtime" or layer == "handlers",
      ("%s: modules may only live in the five layers"):format(path)
    )
    layers[layer] = true
    local handle = io.open(__MOD_DIR .. "/" .. path, "r")
    assert_true(handle ~= nil, ("declared module %s must exist on disk"):format(path))
    if handle then
      handle:close()
    end
  end
  local count = 0
  for _ in pairs(layers) do
    count = count + 1
  end
  assert_eq(count, 5, "all five layers must be populated")
end)

test("AGENTS §3 的 design 路由都能解析到文件", function()
  local agents = read_file(__REPO_ROOT .. "/AGENTS.md")
  local start = agents:find("## 3. Prompt%-surface index")
  assert_true(start ~= nil, "AGENTS.md must carry `## 3. Prompt-surface index`")

  local tail = agents:sub(start)
  local section = tail:match("^(.-)\n## ") or tail

  -- 只看表格行：§3 的散文（退役说明）里也会提到 design 路径，但它不是路由。
  -- 与 Rust 版一致：**不去重**（同一路径在两条路由行里出现算两条）。
  local routes = {}
  for line in section:gmatch("[^\n]+") do
    if line:match("^%s*|") then
      for path in line:gmatch("docs/design/([%w%-_/%.]+%.md)") do
        routes[#routes + 1] = path
      end
    end
  end

  assert_eq(#routes, 13, "AGENTS §3 的 design 路由应为 13 条")
  for _, route in ipairs(routes) do
    assert_true(
      exists(__REPO_ROOT .. "/docs/design/" .. route),
      ("AGENTS §3 路由解析不到文件：docs/design/%s"):format(route)
    )
    local module = route:match("^([^/]+)/")
    assert_true(
      route == "README.md" or module == "kernel" or module == "host" or module == "mod",
      ("%s: design 一级目录只允许 kernel/host/mod"):format(route)
    )
  end
end)

test("结算文案不含 harness 私有字面量", function()
  local path = __MOD_DIR .. "/data/texts/beats-settlement.json"
  local text = read_file(path)
  local forbidden = { "pi-subagents", ".pi/agent", ".claude", ".codex", "archive_sessions" }
  local violations = {}
  local line_no = 0
  for line in text:gmatch("[^\n]+") do
    line_no = line_no + 1
    for _, literal in ipairs(forbidden) do
      if line:find(literal, 1, true) then
        violations[#violations + 1] = ("%s:%d (%s)"):format(path, line_no, literal)
      end
    end
  end
  assert_eq(#violations, 0, "harness literals: " .. table.concat(violations, ", "))
end)
