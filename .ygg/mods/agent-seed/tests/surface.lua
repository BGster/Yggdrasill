-- surface：叙述面与 docs 树的指针账（迁移自 Rust 对照套件）。

local CONTEXT_DIR = __REPO_ROOT .. "/docs/context"

--- 从一个文案值里取出所有 `docs/context/<相对路径>.md` 指针。
--- 允许 `/`（子目录形），仍要求 `.md` 结尾：目录指针 `acts/`、通配 `acts/*.md` 与空串都不算。
local function context_pointer_paths(value)
  local out = {}
  local at = 1
  local prefix = "docs/context/"
  while true do
    local s = value:find(prefix, at, true)
    if s == nil then
      break
    end
    local tail_start = s + #prefix
    local tail = value:sub(tail_start)
    local file = tail:match("^[A-Za-z0-9_%./%-]*")
    at = tail_start + #file
    if file:sub(-3) == ".md" then
      out[#out + 1] = file
    end
  end
  return out
end

--- 核一个键值的全部指针：累加 count 并记违规（非法路径段 / 锚点缺失）。
local function check_pointer(acc, ns, key, value)
  for _, file in ipairs(context_pointer_paths(value)) do
    acc.count = acc.count + 1
    local illegal = false
    for seg in (file .. "/"):gmatch("([^/]*)/") do
      if seg == "" or seg == ".." then
        illegal = true
        break
      end
    end
    if illegal then
      acc.violations[#acc.violations + 1] =
        ("%s.%s -> docs/context/%s (illegal path segment)"):format(ns, key, file)
    elseif not exists(CONTEXT_DIR .. "/" .. file) then
      acc.violations[#acc.violations + 1] =
        ("%s.%s -> docs/context/%s (missing)"):format(ns, key, file)
    end
  end
end

test("叙述面的 docs/context 指针：锚点全部存在，且指针账守恒在 33", function()
  local acc = { count = 0, violations = {} }

  for _, asset in ipairs({
    "ygg:scene/beats/flow", "ygg:scene/beats/harness", "ygg:scene/beats/lanes",
    "ygg:scene/beats/map-guide", "ygg:scene/beats/requirements", "ygg:scene/beats/settlement",
  }) do
    local name = asset:match("([^/]+)$")
    for key, value in pairs(__ASSETS[asset].beats) do
      if type(value) == "string" then
        check_pointer(acc, name, key, value)
      end
    end
  end
  local core = __ASSETS["ygg:scene/texts"]
  for _, ns in ipairs({ "scenes", "prompts" }) do
    for key, value in pairs(core[ns]) do
      if type(value) == "string" then
        check_pointer(acc, ns, key, value)
      end
    end
  end

  -- 六个 <node>_common 必须携带指针（防收缩被意外回退）。
  local flow = __ASSETS["ygg:scene/beats/flow"].beats
  for _, key in ipairs({
    "prologue_common", "charter_common", "coordinate_common",
    "settle_common", "release_common", "retired_common",
  }) do
    local value = flow[key]
    assert_eq(type(value), "string", ("收缩键 `%s` 必须是字符串"):format(key))
    assert_true(
      value:find("docs/context/", 1, true) ~= nil,
      ("收缩键 `%s` 必须携带 docs/context 指针"):format(key)
    )
  end

  assert_eq(
    acc.count,
    33,
    ("docs/context 指针账漂移：期望 33，实得 %d"):format(acc.count)
  )
  assert_eq(
    #acc.violations,
    0,
    "docs/context 下的叙述指针锚点缺失：" .. table.concat(acc.violations, ", ")
  )
end)

--- 公开包装：核一个值并返回 { count, violations }（漏检防线用例用）。
function pointer_violations(ns, key, value)
  local acc = { count = 0, violations = {} }
  check_pointer(acc, ns, key, value)
  return acc
end

test("解析器漏检防线：子目录被识别、缺失锚点转红、`..` 穿越违规", function()
  -- ① 子目录指针必须被识别为 1 条（旧实现：0 条）。
  local one = context_pointer_paths("正文见 docs/context/acts/charter.md（立局幕）。")
  assert_eq(#one, 1, "子目录指针必须被识别")
  assert_eq(one[1], "acts/charter.md", "路径原样")

  -- ② 两层子目录 + 紧邻中文标点 / `§` / `；` 不得吞掉路径。
  local two = context_pointer_paths(
    "政策见 docs/context/rituals/cleanup.md §2；叙事 docs/context/story/run-story.md。"
  )
  assert_eq(#two, 2, "两个子目录指针")
  assert_eq(two[1], "rituals/cleanup.md", "第一个")
  assert_eq(two[2], "story/run-story.md", "第二个")

  -- ③ 目录指针 / 通配不算文件指针。
  local none = context_pointer_paths("见 docs/context/acts/ 与 docs/context/acts/*.md")
  assert_eq(#none, 0, "目录指针与通配都不算文件指针")

  -- ④ 存在的子目录锚点：计数 1、零违规。
  local ok = pointer_violations("t", "ok", "docs/context/acts/charter.md")
  assert_eq(ok.count, 1, "子目录指针必须计入 pointer_count")
  assert_eq(#ok.violations, 0, "存在的锚点零违规")

  -- ⑤ 不存在的子目录锚点：必须计数 1 且产生 1 条 violation（旧实现：0 条 = 漏检）。
  local missing = pointer_violations("t", "missing", "docs/context/acts/definitely-no-such-doc.md")
  assert_eq(missing.count, 1, "缺失锚点也必须计数")
  assert_eq(#missing.violations, 1, "指向不存在子目录文件的指针必须转红")
  assert_true(
    missing.violations[1]:find("missing", 1, true) ~= nil,
    "违规行必须点名键：" .. tostring(missing.violations[1])
  )

  -- ⑥ `..` 穿越必须判违规。
  local escape = pointer_violations("t", "escape", "docs/context/acts/../README.md")
  assert_eq(#escape.violations, 1, "`..` 穿越必须违规：" .. table.concat(escape.violations, ", "))
end)

local DESIGN_DIR = __REPO_ROOT .. "/docs/design"

--- 指向 `docs/design/` 的文件指针（允许多级子目录；尾部多余的点与斜杠被 trim 掉）。
local function design_pointer_paths(value)
  local out = {}
  local at = 1
  local prefix = "docs/design/"
  while true do
    local s = value:find(prefix, at, true)
    if s == nil then
      break
    end
    local tail_start = s + #prefix
    local tail = value:sub(tail_start)
    local run = tail:match("^[A-Za-z0-9_%./%-]*")
    at = tail_start + #run
    local file = run:gsub("[%./]+$", "")
    if file:sub(-3) == ".md" then
      out[#out + 1] = file
    end
  end
  return out
end

local function design_pointer_violations(acc, ns, key, value)
  for _, file in ipairs(design_pointer_paths(value)) do
    acc.count = acc.count + 1
    local illegal = false
    for seg in (file .. "/"):gmatch("([^/]*)/") do
      if seg == "" or seg == ".." then
        illegal = true
        break
      end
    end
    if illegal then
      acc.violations[#acc.violations + 1] =
        ("%s.%s -> docs/design/%s (illegal path segment)"):format(ns, key, file)
    elseif not exists(DESIGN_DIR .. "/" .. file) then
      acc.violations[#acc.violations + 1] =
        ("%s.%s -> docs/design/%s (missing)"):format(ns, key, file)
    end
  end
end

test("叙述面与源码的 docs/design 指针：逐来源账守恒 + 锚点全部存在", function()
  local items = {}

  -- ① 文案（texts/*.json）：beats + scenes/prompts 两命名空间。
  for _, asset in ipairs({
    "ygg:scene/texts", "ygg:scene/beats/flow", "ygg:scene/beats/harness",
    "ygg:scene/beats/lanes", "ygg:scene/beats/map-guide",
    "ygg:scene/beats/requirements", "ygg:scene/beats/settlement",
  }) do
    local frag = __ASSETS[asset]
    local name = asset:match("([^/]+)$")
    local fname = ({ ["texts"] = "core.json" })[name] or ("beats-" .. name .. ".json")
    if frag.beats ~= nil then
      for key, value in pairs(frag.beats) do
        if type(value) == "string" then
          items[#items + 1] = { fname, key, value }
        end
      end
    end
    for _, ns in ipairs({ "scenes", "prompts" }) do
      if frag[ns] ~= nil then
        for key, value in pairs(frag[ns]) do
          if type(value) == "string" then
            items[#items + 1] = { fname, ns .. "." .. key, value }
          end
        end
      end
    end
  end

  -- ② Lua 源码（src/**/*.lua）整文件当值扫。
  for _, path in ipairs(list_files(__MOD_DIR .. "/src", "lua")) do
    local name = path:match("([^/]+)$")
    items[#items + 1] = { name, "source", read_file(path) }
  end

  local ledger, total, violations = {}, 0, {}
  for _, item in ipairs(items) do
    local acc = { count = 0, violations = {} }
    design_pointer_violations(acc, item[1], item[2], item[3])
    for _, v in ipairs(acc.violations) do
      violations[#violations + 1] = v
    end
    if acc.count > 0 then
      total = total + acc.count
      ledger[item[1]] = (ledger[item[1]] or 0) + acc.count
    end
  end

  local expected = { ["beats-settlement.json"] = 3, ["tools.lua"] = 2 }
  local keys = {}
  for k in pairs(ledger) do
    keys[#keys + 1] = k
  end
  table.sort(keys)
  local ek = {}
  for k in pairs(expected) do
    ek[#ek + 1] = k
  end
  table.sort(ek)
  assert_eq(#keys, #ek, "docs/design 指针账漂移（来源数）：实得 " .. table.concat(keys, ","))
  for i = 1, #ek do
    assert_eq(keys[i], ek[i], "docs/design 指针账漂移（来源名）")
    assert_eq(ledger[keys[i]], expected[ek[i]],
      ("来源 `%s` 的指针数漂移"):format(ek[i]))
  end
  assert_eq(total, 5, ("docs/design 指针总数漂移：期望 5，实得 %d"):format(total))
  assert_eq(#violations, 0, "docs/design 指针锚点不存在：" .. table.concat(violations, ", "))
end)

--- 公开包装：design 指针解析与核销（漏检防线用例用）。
function design_ptr_paths(value)
  return design_pointer_paths(value)
end

function design_ptr_violations(ns, key, value)
  local acc = { count = 0, violations = {} }
  design_pointer_violations(acc, ns, key, value)
  return acc
end

test("design 解析器漏检防线：子目录 / 句末点 / 通配 / `..` 六段", function()
  -- ① 一层子目录。
  local one = design_ptr_paths("见 docs/design/kernel/state-machine.md（状态机）。")
  assert_eq(#one, 1, "子目录指针必须识别")
  assert_eq(one[1], "kernel/state-machine.md", "路径原样")

  -- ② 句末多点一个也不吞；紧邻中文标点 / `；` 不吞路径。
  local two = design_ptr_paths(
    "改 harness-tools 前读 docs/design/host/harness-tools.md. 再看 docs/design/mod/tool-bridge.md；"
  )
  assert_eq(#two, 2, "两个指针")
  assert_eq(two[1], "host/harness-tools.md", "句末多余的点被 trim")
  assert_eq(two[2], "mod/tool-bridge.md", "紧邻中文标点不吞")

  -- ③ 目录指针 / 通配 / 占位都不算文件指针。
  local none = design_ptr_paths(
    "目录 docs/design/kernel/ 与通配 docs/design/*.md 与占位 docs/design/<module>/<topic>.md"
  )
  assert_eq(#none, 0, "目录、通配、占位都不算文件指针")

  -- ④ 存在的子目录锚点：计数 1、零违规。
  local ok = design_ptr_violations("t", "ok", "docs/design/kernel/state-machine.md")
  assert_eq(ok.count, 1, "子目录指针必须计入 pointer_count")
  assert_eq(#ok.violations, 0, "存在的锚点零违规")

  -- ⑤ 不存在的子目录锚点：计数 1 且 1 条违规。
  local missing = design_ptr_violations("t", "missing", "docs/design/kernel/definitely-no-such-doc.md")
  assert_eq(missing.count, 1, "缺失锚点也计数")
  assert_eq(#missing.violations, 1, "指向不存在子目录文件的指针必须转红")

  -- ⑥ `..` 穿越必须违规。
  local escape = design_ptr_violations("t", "escape", "docs/design/kernel/../README.md")
  assert_eq(#escape.violations, 1, "`..` 穿越必须违规：" .. table.concat(escape.violations, ", "))
end)

--- 列出目录下的一级条目，分成 { dirs, files } 两个数组。
local function list_entries(path)
  local dirs, files = {}, {}
  local pipe = io.popen(('ls -1a "%s"'):format(path))
  if pipe == nil then
    return dirs, files
  end
  for name in pipe:lines() do
    if name ~= "." and name ~= ".." then
      local probe = io.popen(('test -d "%s/%s" && echo yes'):format(path, name))
      local is_dir = probe:read("*l") == "yes"
      probe:close()
      if is_dir then
        dirs[#dirs + 1] = name
      else
        files[#files + 1] = name
      end
    end
  end
  pipe:close()
  return dirs, files
end

--- 规范化仓库内相对路径（吃掉 `.`，`..` 回退一级）。
local function normalize_repo_path(p)
  local parts = {}
  for seg in p:gmatch("[^/]+") do
    if seg == ".." then
      if #parts > 0 then
        table.remove(parts)
      end
    elseif seg ~= "." then
      parts[#parts + 1] = seg
    end
  end
  return table.concat(parts, "/")
end

test("docs/design 树索引：三模块 + 16 份文件 + README 双向相等 + id 双射", function()
  local dir = DESIGN_DIR

  -- ④ 一级子目录恰好三模块；平铺只允许树入口 README.md。
  local dirs, flat = list_entries(dir)
  table.sort(dirs)
  table.sort(flat)
  assert_eq(#dirs, 3, "docs/design 一级子目录必须是三模块，实得：" .. table.concat(dirs, ","))
  assert_eq(dirs[1], "host", "一级子目录 1")
  assert_eq(dirs[2], "kernel", "一级子目录 2")
  assert_eq(dirs[3], "mod", "一级子目录 3")
  assert_eq(#flat, 1, "docs/design/*.md 只允许 README.md，实得：" .. table.concat(flat, ","))
  assert_eq(flat[1], "README.md", "平铺文件或软链 = 留了兼容壳")

  -- ① 实文件集合（相对 docs/design，排除 README.md）。
  local actual = {}
  local prefix = dir .. "/"
  for _, path in ipairs(list_files(dir, "md")) do
    local rel = path:sub(#prefix + 1)
    if rel ~= "README.md" then
      actual[#actual + 1] = rel
    end
  end
  table.sort(actual)
  assert_eq(#actual, 16, "契约面应为 16 份文件：" .. table.concat(actual, ","))

  -- ① 索引里的链接集合必须与实文件集合双向相等。
  local readme = read_file(dir .. "/README.md")
  local indexed, seen_idx = {}, {}
  for target in readme:gmatch("%]%(([^%)]+)%)") do
    local bare = target:match("^([^#]*)")
    if bare ~= nil and bare ~= "" and not bare:match("^https?://")
      and not bare:match("^mailto:") and bare:sub(-3) == ".md" then
      local resolved = normalize_repo_path("docs/design/" .. bare)
      local rel = resolved:match("^docs/design/(.+)$")
      if rel ~= nil and rel ~= "README.md" and not seen_idx[rel] then
        seen_idx[rel] = true
        indexed[#indexed + 1] = rel
      end
    end
  end
  table.sort(indexed)
  assert_eq(#indexed, #actual,
    "docs/design/README.md 的索引与实文件集合必须双向相等，索引实得："
      .. table.concat(indexed, ","))
  for i = 1, #actual do
    assert_eq(indexed[i], actual[i], "索引与实文件集合逐项相等")
  end

  -- ② id 与路径双射（每份文件的 `> **id** ` 固定头必须等于去扩展名的相对路径）。
  for _, rel in ipairs(actual) do
    local text = read_file(dir .. "/" .. rel)
    local declared = nil
    for line in text:gmatch("[^\n]+") do
      if line:sub(1, 9) == "> **id** " then
        declared = line:sub(10):match("^%S+")
        break
      end
    end
    assert_true(declared ~= nil, rel .. " 缺 `> **id**` 固定头")
    assert_eq(declared, (rel:gsub("%.md$", "")), rel .. " 的 id 与路径不一致")
  end
end)

test("scenes 表 reference 段：4 个理解性文档可解析，且不进 scene/reqStage/readPlan", function()
  local scenes = __SCENES

  local reference = {}
  for key, group in pairs(scenes.reference) do
    if key ~= "note" then
      for _, id in ipairs(group) do
        reference[#reference + 1] = id
      end
    end
  end
  assert_eq(#reference, 4,
    "reference 段应登记 4 个（含不进 readPlan 的规范文档）：" .. table.concat(reference, ","))

  local scene_ids, stage_ids = {}, {}
  for _, group in pairs(scenes.scene) do
    for _, id in ipairs(group) do
      scene_ids[id] = true
    end
  end
  for _, id in pairs(scenes.reqStage) do
    stage_ids[id] = true
  end
  local read_plan = read_file(__MOD_DIR .. "/data/config.json")

  for _, id in ipairs(reference) do
    local path = CONTEXT_DIR .. "/" .. id .. ".md"
    assert_true(exists(path), "reference id 解析不到文件：docs/context/" .. id .. ".md")

    local text = read_file(path)
    local declared = nil
    for line in text:gmatch("[^\n]+") do
      if line:sub(1, 9) == "> **id** " then
        declared = line:sub(10):match("^%S+")
        break
      end
    end
    assert_true(declared ~= nil, "docs/context/" .. id .. ".md 缺 `> **id**` 固定头")
    assert_eq(declared, id, "docs/context/" .. id .. ".md 的 id 与 scenes 表不一致")

    assert_true(not scene_ids[id],
      id .. " 是理解性文档，不得进 scenes.scene（那会进 [READ FIRST] 并产生签名义务）")
    assert_true(not stage_ids[id], id .. " 是理解性文档，不得进 scenes.reqStage")
    assert_true(read_plan:find(id, 1, true) == nil,
      id .. " 不得出现在 data/config.json 的 readPlan 里（默认不拦门禁）")
  end
end)

--- 通用指针解析（带 `trim_end_matches(['.', '/'])`——与 design 侧同语义）。
local function pointer_paths_trim(value, prefix)
  local out = {}
  local at = 1
  while true do
    local s = value:find(prefix, at, true)
    if s == nil then
      break
    end
    local tail_start = s + #prefix
    local tail = value:sub(tail_start)
    local run = tail:match("^[A-Za-z0-9_%./%-]*")
    at = tail_start + #run
    local file = run:gsub("[%./]+$", "")
    if file:sub(-3) == ".md" then
      out[#out + 1] = file
    end
  end
  return out
end

test("texts 与 src 的 docs/context 指针：逐来源账 34 守恒 + 锚点全部存在", function()
  local items = {}

  for _, asset in ipairs({
    "ygg:scene/texts", "ygg:scene/beats/flow", "ygg:scene/beats/harness",
    "ygg:scene/beats/lanes", "ygg:scene/beats/map-guide",
    "ygg:scene/beats/requirements", "ygg:scene/beats/settlement",
  }) do
    local frag = __ASSETS[asset]
    local name = asset:match("([^/]+)$")
    local fname = ({ ["texts"] = "core.json" })[name] or ("beats-" .. name .. ".json")
    if frag.beats ~= nil then
      for key, value in pairs(frag.beats) do
        if type(value) == "string" then
          items[#items + 1] = { fname, key, value }
        end
      end
    end
    for _, ns in ipairs({ "scenes", "prompts" }) do
      if frag[ns] ~= nil then
        for key, value in pairs(frag[ns]) do
          if type(value) == "string" then
            items[#items + 1] = { fname, ns .. "." .. key, value }
          end
        end
      end
    end
  end

  for _, path in ipairs(list_files(__MOD_DIR .. "/src", "lua")) do
    items[#items + 1] = { path:match("([^/]+)$"), "source", read_file(path) }
  end

  local ledger, total, violations = {}, 0, {}
  for _, item in ipairs(items) do
    for _, file in ipairs(pointer_paths_trim(item[3], "docs/context/")) do
      total = total + 1
      ledger[item[1]] = (ledger[item[1]] or 0) + 1
      local illegal = false
      for seg in (file .. "/"):gmatch("([^/]*)/") do
        if seg == "" or seg == ".." then
          illegal = true
          break
        end
      end
      if illegal then
        violations[#violations + 1] = ("%s.%s -> docs/context/%s (illegal)"):format(item[1], item[2], file)
      elseif not exists(CONTEXT_DIR .. "/" .. file) then
        violations[#violations + 1] = ("%s.%s -> docs/context/%s (missing)"):format(item[1], item[2], file)
      end
    end
  end

  local expected = {
    ["beats-flow.json"] = 7,
    ["beats-harness.json"] = 5,
    ["beats-settlement.json"] = 3,
    ["core.json"] = 18,
    ["tools.lua"] = 1,
  }
  local keys = {}
  for k in pairs(ledger) do
    keys[#keys + 1] = k
  end
  table.sort(keys)
  local ek = {}
  for k in pairs(expected) do
    ek[#ek + 1] = k
  end
  table.sort(ek)
  assert_eq(#keys, #ek, "docs/context 指针账漂移（来源数）：实得 " .. table.concat(keys, ","))
  for i = 1, #ek do
    assert_eq(keys[i], ek[i], "来源名漂移")
    assert_eq(ledger[keys[i]], expected[ek[i]], ("来源 `%s` 的指针数漂移"):format(ek[i]))
  end
  assert_eq(total, 34, ("docs/context 指针总数漂移：期望 34，实得 %d"):format(total))
  assert_eq(#violations, 0, "docs/context 指针锚点不存在：" .. table.concat(violations, ", "))
end)

test("goal 授权门：无授权给可读警告，登记后放行", function()
  fresh()
  reset()
  fire("config.set", "play.mode=goal")
  assert_eq(count_marker("[GOAL AUTH PENDING]"), 1, "无授权时切 goal 必须给可读警告")

  fire("config.set", "goal.authorization=authorized for this run")
  reset()
  fire("config.set", "play.mode=goal")
  assert_eq(count_marker("[GOAL AUTH PENDING]"), 0, "登记授权后不再警告")
end)
