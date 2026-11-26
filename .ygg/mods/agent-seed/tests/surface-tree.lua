-- surface-tree.lua —— S32 纯搬移自 surface.lua L302-564（契约树索引 + texts/src 指针账 + goal 授权门；自包含分组，test() 载入即跑与拆分前同序）。

local CONTEXT_DIR = __REPO_ROOT .. "/openwiki/operating"
local DESIGN_DIR = __REPO_ROOT .. "/openwiki/contracts"
local DESIGN_INDEX = __REPO_ROOT .. "/openwiki/architecture/README.md"
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

test("契约树索引：四模块 + 30 份文件 + 索引双向相等 + id 双射", function()
  local dir = DESIGN_DIR

  -- ④ 一级子目录恰好四模块；平铺零文件（索引已搬到 architecture/README.md）。
  local dirs, flat = list_entries(dir)
  table.sort(dirs)
  table.sort(flat)
  assert_eq(#dirs, 4, "openwiki/contracts 一级子目录必须是四模块，实得：" .. table.concat(dirs, ","))
  assert_eq(dirs[1], "host", "一级子目录 1")
  assert_eq(dirs[2], "kernel", "一级子目录 2")
  assert_eq(dirs[3], "mod", "一级子目录 3")
  assert_eq(dirs[4], "packages", "一级子目录 4")
  assert_eq(#flat, 0, "openwiki/contracts/*.md 不允许平铺文件，实得：" .. table.concat(flat, ","))

  -- ① 实文件集合（相对 openwiki/contracts，排除生成的 index.md）。
  local actual = {}
  local prefix = dir .. "/"
  for _, path in ipairs(list_files(dir, "md")) do
    local rel = path:sub(#prefix + 1)
    if rel ~= "index.md" then
      actual[#actual + 1] = rel
    end
  end
  table.sort(actual)
  assert_eq(#actual, 30, "契约面应为 30 份文件：" .. table.concat(actual, ","))

  -- ① 索引里的链接集合必须与实文件集合双向相等。
  local readme = read_file(DESIGN_INDEX)
  local indexed, seen_idx = {}, {}
  for target in readme:gmatch("%]%(([^%)]+)%)") do
    local bare = target:match("^([^#]*)")
    if bare ~= nil and bare ~= "" and not bare:match("^https?://")
      and not bare:match("^mailto:") and bare:sub(-3) == ".md" then
      local resolved = normalize_repo_path("openwiki/architecture/" .. bare)
      local rel = resolved:match("^openwiki/contracts/(.+)$")
      if rel ~= nil and not seen_idx[rel] then
        seen_idx[rel] = true
        indexed[#indexed + 1] = rel
      end
    end
  end
  table.sort(indexed)
  assert_eq(#indexed, #actual,
    "openwiki/architecture/README.md 的索引与实文件集合必须双向相等，索引实得："
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
    assert_true(exists(path), "reference id 解析不到文件：openwiki/operating/" .. id .. ".md")

    local text = read_file(path)
    local declared = nil
    for line in text:gmatch("[^\n]+") do
      if line:sub(1, 9) == "> **id** " then
        declared = line:sub(10):match("^%S+")
        break
      end
    end
    assert_true(declared ~= nil, "openwiki/operating/" .. id .. ".md 缺 `> **id**` 固定头")
    assert_eq(declared, id, "openwiki/operating/" .. id .. ".md 的 id 与 scenes 表不一致")

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

test("texts 与 src 的 openwiki/operating 指针：逐来源账 34 守恒 + 锚点全部存在", function()
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
    for _, file in ipairs(pointer_paths_trim(item[3], "openwiki/operating/")) do
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
        violations[#violations + 1] = ("%s.%s -> openwiki/operating/%s (illegal)"):format(item[1], item[2], file)
      elseif not exists(CONTEXT_DIR .. "/" .. file) then
        violations[#violations + 1] = ("%s.%s -> openwiki/operating/%s (missing)"):format(item[1], item[2], file)
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
  assert_eq(#keys, #ek, "openwiki/operating 指针账漂移（来源数）：实得 " .. table.concat(keys, ","))
  for i = 1, #ek do
    assert_eq(keys[i], ek[i], "来源名漂移")
    assert_eq(ledger[keys[i]], expected[ek[i]], ("来源 `%s` 的指针数漂移"):format(ek[i]))
  end
  assert_eq(total, 34, ("openwiki/operating 指针总数漂移：期望 34，实得 %d"):format(total))
  assert_eq(#violations, 0, "openwiki/operating 指针锚点不存在：" .. table.concat(violations, ", "))
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
