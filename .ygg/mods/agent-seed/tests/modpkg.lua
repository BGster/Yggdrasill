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
  assert_eq(#modules, 45, "the mod declares 45 modules（S32 拆分后）")

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

-- ── 配置面分区表（r8 / R1）──
-- 新增 data/config.json 顶层键或 gates 子键必须在此登记，否则下面的用例直接红——
-- 这正是 R1 的咬合力：加开关的人必须先回答「这是固定项还是 armed」。

local GENERATED, HANDWRITTEN = "generated", "handwritten"

local function ckey(path, origin, slug, probe)
  return { path = path, origin = origin, slug = slug, probe = probe }
end

local CONFIG_SURFACE = {
  -- 生成区：生成器拥有；两份都是 armed（缺席 ⇒ 必读集为空 ⇒ 读签门禁整体 inert）。
  ckey("readPlan", GENERATED, "read_gate", "nonempty"),
  ckey("readGate", GENERATED, "read_gate", "nonempty"),
  -- 手写区：缺席走 lib 缺省，且不削弱任何保护。
  ckey("play", HANDWRITTEN),
  ckey("drive", HANDWRITTEN),
  ckey("evolution", HANDWRITTEN),
  ckey("autosave", HANDWRITTEN),
  ckey("hot_reload", HANDWRITTEN),
  ckey("harness", HANDWRITTEN),
  ckey("settlement", HANDWRITTEN),
  -- allowedTools 缺席 = 一个工具都不放行（fail-closed）：缺席**收紧**而非削弱，故非 armed。
  ckey("allowedTools", HANDWRITTEN),
  ckey("gates", HANDWRITTEN),
  ckey("gates.require_all_green", HANDWRITTEN, "gates_strict", "istrue"),
}

--- 取点分路径的值。
local function config_value(cfg, path)
  local node = cfg
  for seg in path:gmatch("[^.]+") do
    if type(node) ~= "table" then
      return nil
    end
    node = node[seg]
  end
  return node
end

--- 某一层深度的声明键集合（1 = 顶层，2 = gates 子键）。
local function declared_at(depth)
  local out = {}
  for _, e in ipairs(CONFIG_SURFACE) do
    local n = 0
    for _ in e.path:gmatch("[^.]+") do
      n = n + 1
    end
    if n == depth then
      out[e.path] = true
    end
  end
  return out
end

local function assert_cover(label, actual, declared)
  for _, k in ipairs(actual) do
    assert_true(
      declared[k] == true,
      ("%s：未分类的配置键 `%s` —— 必须登记进 CONFIG_SURFACE"):format(label, k)
    )
  end
  for k in pairs(declared) do
    local found = false
    for _, a in ipairs(actual) do
      if a == k then
        found = true
        break
      end
    end
    assert_true(found, ("%s：CONFIG_SURFACE 声明的键 `%s` 已不存在"):format(label, k))
  end
end

--- 生成器的自报键表：解析脚本而不是抄一份常量（两处一旦分叉，R1 立刻红）。
local function generator_owned_keys()
  local src = read_file(__REPO_ROOT .. "/scripts/gen-scene-readfirst.py")
  local line = src:match("READSET_KEYS%s*=%s*%(([^)]*)%)")
  assert_true(line ~= nil, "gen-scene-readfirst.py 必须声明 READSET_KEYS（生成区归属的判据）")
  local out = {}
  for k in line:gmatch('"([^"]+)"') do
    out[#out + 1] = k
  end
  return out
end

test("配置面：每个键都被分类（生成区 == 生成器自报，且至少两枚 armed）", function()
  local cfg = __ASSETS["ygg:agent/config"]

  local top = {}
  for k in pairs(cfg) do
    top[#top + 1] = k
  end
  assert_cover("top-level", top, declared_at(1))

  local gates = {}
  if type(cfg.gates) == "table" then
    for k in pairs(cfg.gates) do
      gates[#gates + 1] = "gates." .. k
    end
  end
  assert_cover("gates", gates, declared_at(2))

  -- 生成区不是「我们在表里这么写」，而是「生成器这么报」——两处必须逐项相等。
  local declared_generated = {}
  for _, e in ipairs(CONFIG_SURFACE) do
    if e.origin == GENERATED then
      declared_generated[#declared_generated + 1] = e.path
    end
  end
  table.sort(declared_generated)
  local actual_generated = generator_owned_keys()
  table.sort(actual_generated)
  assert_eq(
    #actual_generated,
    #declared_generated,
    "生成区必须等于 scripts/gen-scene-readfirst.py 的 READSET_KEYS：生成器拥有而表说是手写的"
      .. "（或反之），等于这条分区是对一个不复存在的事实的声明"
  )
  for i = 1, #declared_generated do
    assert_eq(actual_generated[i], declared_generated[i], "生成区逐项相等")
  end

  -- 分区必须真的分出了 armed：一张全是 fixed 的表会让 R2 空转。
  local armed = 0
  for _, e in ipairs(CONFIG_SURFACE) do
    if e.slug ~= nil then
      armed = armed + 1
    end
  end
  assert_true(armed >= 2, "必须至少分类两个 armed 开关，实得 " .. armed .. "——空的 armed 集让 R2 变成空转")
end)

test("armed 开关入库即生效，且每条都有 ships_armed 锚点", function()
  local cfg = __ASSETS["ygg:agent/config"]
  -- 扫描全部 Lua 测试文件的 `-- [armed-case] <slug>_ships_armed` 锚点。
  local source = ""
  for _, path in ipairs(list_files(__MOD_DIR .. "/tests", "lua")) do
    source = source .. "\n" .. read_file(path)
  end

  for _, e in ipairs(CONFIG_SURFACE) do
    if e.slug ~= nil then
      local value = config_value(cfg, e.path)
      if e.probe == "nonempty" then
        assert_true(
          type(value) == "table",
          e.path .. " 必须入库为对象——库里没有这个键，正是「代码完整、闸没开」那种失效"
        )
        assert_true(next(value) ~= nil, e.path .. " 必须**已上膛**：空表就是惰性态")
      else
        assert_eq(
          value,
          true,
          e.path .. " 必须入库为 true——缺席或 false 会静默关掉它守卫的东西"
        )
      end

      local needle = "\n-- [armed-case] " .. e.slug .. "_ships_armed"
      assert_true(
        source:find(needle, 1, true) ~= nil,
        ("armed 开关 `%s`（slug %s）缺少 `-- [armed-case] %s_ships_armed` 锚点："
          .. "一条只断言「生效态」或只断言「退让态」的用例，是这条规则要消除的那种沉默"):format(
          e.path,
          e.slug,
          e.slug
        )
      )
    end
  end
end)
