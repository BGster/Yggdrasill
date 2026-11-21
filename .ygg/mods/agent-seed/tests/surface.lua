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

test("docs/design 树索引：四模块 + 26 份文件 + README 双向相等 + id 双射", function()
  local dir = DESIGN_DIR

  -- ④ 一级子目录恰好四模块；平铺只允许树入口 README.md。
  local dirs, flat = list_entries(dir)
  table.sort(dirs)
  table.sort(flat)
  assert_eq(#dirs, 4, "docs/design 一级子目录必须是四模块，实得：" .. table.concat(dirs, ","))
  assert_eq(dirs[1], "host", "一级子目录 1")
  assert_eq(dirs[2], "kernel", "一级子目录 2")
  assert_eq(dirs[3], "mod", "一级子目录 3")
  assert_eq(dirs[4], "packages", "一级子目录 4")
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
  assert_eq(#actual, 26, "契约面应为 26 份文件：" .. table.concat(actual, ","))

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

--- 从叙述里抽需求行：第 2 个 token 形如 `r<digits>`、第 3 个以 `[` 开头。
local function req_rows(lines)
  local rows = {}
  for _, line in ipairs(lines) do
    local tokens = {}
    for tok in line:gmatch("%S+") do
      tokens[#tokens + 1] = tok
    end
    local id, status = tokens[2], tokens[3]
    if id ~= nil and status ~= nil then
      if id:match("^r%d+$") ~= nil and status:sub(1, 1) == "[" then
        rows[#rows + 1] = { id = id, status = status:sub(2) }
      end
    end
  end
  return rows
end

test("requirement.list 分页：默认只翻开放行，full 才含已关闭", function()
  fresh()
  charter_and_begin()

  -- r1 走完整链路到 done（唯一关闭的行）。
  fire("requirement.add", "synthetic requirement|synthetic description|page-r1")
  fire("requirement.dispatch", "0")
  submit(1, "plan", "page-p1")
  click("gate.approve — ")
  submit(1, "development", "page-p2")
  click("proposal.accept — ")
  click("gate.approve — ")
  fire("integration.confirm", "r1|develop@abc1234")
  assert_eq(kv("req.1.status"), "done", "r1 已关闭")

  -- r2..r13：十二条开放草稿（共 13 条需求）。
  for i = 2, 13 do
    fire("requirement.add", ("page requirement %d|synthetic description|page-r%d"):format(i, i))
  end

  -- 默认视图：恰好 12 条**开放**行，关闭的 r1 被折出。
  reset()
  fire("requirement.list", "0")
  local default_lines = narrated()
  local rows = req_rows(default_lines)
  assert_eq(#rows, 12, "默认视图必须翻 12 行")
  for _, row in ipairs(rows) do
    assert_true(row.id ~= "r1", "关闭的 r1 不得占开放页：" .. row.id)
  end
  assert_true(
    default_lines[1] ~= nil and default_lines[1]:sub(1, 11) == "[REQ GRAPH]",
    "翻页必须保留图头"
  )

  -- full 选项渲染全部：13 行，关闭的 r1 在末尾。
  reset()
  fire("requirement.list", "full")
  local full_lines = narrated()
  local full_rows = req_rows(full_lines)
  assert_eq(#full_rows, 13, "full 必须渲染每一条需求")
  assert_eq(
    full_rows[#full_rows].id,
    "r1",
    "唯一关闭的行收在 full 页末（开放在前、关闭最新到最旧）"
  )
  assert_true(
    full_lines[1] ~= nil and full_lines[1]:sub(1, 11) == "[REQ GRAPH]",
    "full 视图同样保留图头"
  )
end)

test("summary 投影：play/生命周期三件套 + 批结算后 drain + 维护义务", function()
  fresh()

  -- 新局：boot 落在立局幕，结算前置未满足。
  reset()
  fire("agent.summary", "0")
  local s = __SELFTEST.meta.summary
  assert_eq(s.play, "guide", "play.mode 默认 guide 必须投影")
  assert_eq(s.lifecycle, "resident", "resident 生命周期")
  assert_eq(s.batch, 1, "批次号")
  assert_eq(s.batchDrained, false, "未 drain")

  -- 立局 + 一条开放需求 → `gate_problem()` 非 nil，仍未 drain。
  charter_and_begin()
  fire("requirement.add", "watchdog req|synthetic description|watchdog-r1")
  reset()
  fire("agent.summary", "0")
  assert_eq(
    __SELFTEST.meta.summary.batchDrained,
    false,
    "有开放需求时不得 drain"
  )

  -- 完整链路 + 批结算 → drain。
  fire("requirement.dispatch", "0")
  submit(1, "plan", "watchdog-p1")
  click("gate.approve — ")
  submit(1, "development", "watchdog-p2")
  click("proposal.accept — ")
  click("gate.approve — ")
  fire("integration.confirm", "r1|develop@abc1234")
  click("coordinate.submit — ")
  click("verify.submit all_green=true — ")
  fire("settlement.evolution", "|screened|c=0/0|sessions=0/0/none|todo=0/cleared")
  fire("release.skip", "纯测试批次")
  click("review.note — ")

  reset()
  fire("agent.summary", "0")
  local s2 = __SELFTEST.meta.summary
  assert_eq(s2.batch, 2, "批次号 +1")
  -- 文档维护义务不再派生需求行（用户裁定：CONTEXT / DESIGN 维护归主会话自办）。
  assert_eq(kv("req.count"), 1, "注册表没有新增行")
  assert_eq(kv("docs.maintenance.due"), "1", "义务登记在 due 上")
  assert_eq(s2.batchDrained, true, "批结算且无开放需求 → drain")

  -- 主会话做完复核后解除义务；此后新需求正常入册。
  reset()
  fire("config.set", "docs.maintenance.done=1")
  assert_true(narrated_contains("[DOCS MAINTENANCE CLEARED]"), "必须叙述已解除")
  assert_eq(kv("docs.maintenance.due"), "", "due 清空")
  fire("requirement.add", "watchdog b2|desc|watchdog-2")
  assert_eq(kv("req.count"), 2, "解除后新需求正常入册")
end)

--- 地图窗口：从 `[CURRENT NODE]` 头到 `(operation view` 脚（含两端）。
local function map_window(lines)
  local head, foot
  for i, line in ipairs(lines) do
    if head == nil and line:sub(1, #"[CURRENT NODE]") == "[CURRENT NODE]" then
      head = i
    end
    if line:sub(1, #"(operation view") == "(operation view" then
      foot = i
      break
    end
  end
  assert_true(head ~= nil, "缺地图头 `[CURRENT NODE]`")
  assert_true(foot ~= nil, "缺操作视图脚 `(operation view`")
  assert_true(head < foot, "地图头必须在脚之前")
  local out = {}
  for i = head, foot do
    out[#out + 1] = lines[i]
  end
  return out
end

test("map.show 单视图且对载荷不敏感（含措辞红线）", function()
  fresh()
  local function run(payload)
    reset()
    fire("map.show", payload)
    return narrated()
  end

  local empty = run("0")
  local zero = run("")
  local full = run("full")
  local junk = run("nonsense-payload")

  -- ① 四种载荷得到**同一个**窗口（handler 完全忽略载荷）。
  local window = map_window(empty)
  local baseline = table.concat(window, "\n")
  assert_eq(table.concat(map_window(zero), "\n"), baseline, "空串载荷窗口不变")
  assert_eq(table.concat(map_window(full), "\n"), baseline, "full 载荷窗口不变")
  assert_eq(table.concat(map_window(junk), "\n"), baseline, "无意义载荷窗口不变")

  -- ② 单视图：头带当前微状态，脚否定载荷面；设计视图（`[MAP]` 头）不可达。
  assert_true(
    window[1]:sub(1, #"[CURRENT NODE] [") == "[CURRENT NODE] [",
    "地图头必须带当前微状态：" .. window[1]
  )
  assert_true(
    baseline:find("map.show takes no payload", 1, true) ~= nil,
    "脚必须否定载荷面：" .. baseline
  )
  for _, line in ipairs(empty) do
    assert_true(line:sub(1, 5) ~= "[MAP]", "设计视图已注销，不得渲染 [MAP] 头：" .. line)
  end

  -- ③ 措辞红线（两视图时代留下的三条）。
  assert_true(baseline:find("selectable", 1, true) ~= nil, "每条边必须说 selectable：" .. baseline)
  assert_true(
    baseline:find("condition not shown", 1, true) ~= nil,
    "缺 when 必须读作 condition not shown：" .. baseline
  )
  assert_true(
    baseline:find("executable", 1, true) == nil,
    "操作视图不得承诺 executable（选择侧没有前置条件）：" .. baseline
  )
  assert_true(
    baseline:find("no extra condition", 1, true) == nil,
    "操作视图不得担保 no extra condition：" .. baseline
  )
end)

test("泳道分支命名：合法形状即记账 / 八条拒绝矩阵 / 空名 claim 层合法", function()
  fresh()
  start_requirement()

  -- 合法形状：逐字记账，带自己的叙述。
  reset()
  fire("lane.claim", "1|r1|feature/aa11-semantic-name|abc1234|1")
  assert_eq(count_marker("[LANE CLAIMED]"), 1, "合法分支必须叙述")

  -- 拒绝矩阵（每条规则一例）。每条拒绝都要移动 reject 账，且一件都不绑。
  local refusals = {
    "feature/r1",                           -- uuid 槽里是裸需求号
    "FEATURE/aa11-semantic-name",           -- 前缀大小写敏感
    "feature/aa11-x",                       -- uuid 槽必须恰好 4 位
    "feature/aa1-semantic-name",            -- 3 位 uuid
    "feature/aa11-deadbeef",                -- 8 位纯十六进制语义段非法
    "feature/aa11-12345",                   -- 纯数字语义段
    "feature/aa11-",                        -- 空语义段
    "feature/aa11-" .. string.rep("a", 37), -- 总长超过 48
  }
  local rejects_before = kv("reject.count")
  for _, branch in ipairs(refusals) do
    reset()
    fire("lane.claim", ("2|r1|%s|abc1234|1"):format(branch))
    assert_true(
      narrated_contains("[REJECTED") or narrated_contains("[BLOCKED"),
      ("分支 `%s` 必须被拒："):format(branch) .. table.concat(narrated(), " | ")
    )
    assert_nil(kv("lane.2.branch"), "被拒的分支不得绑定泳道：" .. branch)
  end
  assert_true(kv("reject.count") ~= rejects_before, "拒绝必须移动 reject 账")

  -- 原认领不受这些拒绝影响。
  assert_eq(kv("lane.1.branch"), "feature/aa11-semantic-name", "台账保留原认领")
  assert_eq(kv("lane.1.req"), "r1", "需求绑定保留")

  -- 空分支在 claim 层**仍然合法**：形状由自动认领层拒，不是这一层。
  fire("lane.release", "1")
  reset()
  fire("lane.claim", "1|r1|||1")
  assert_eq(count_marker("[LANE CLAIMED]"), 1, "空分支在 claim 层合法")
  assert_eq(kv("lane.1.branch"), "", "分支记为空串")
end)
