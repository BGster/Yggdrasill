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
