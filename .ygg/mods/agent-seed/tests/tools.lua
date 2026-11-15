-- tools：工具桥的两层白名单与合并声明面（迁移自 Rust 对照套件）。

local SHELL_BASENAMES = {
  sh = true, bash = true, zsh = true, dash = true, fish = true, ksh = true,
  csh = true, tcsh = true, cmd = true, ["cmd.exe"] = true, powershell = true, pwsh = true,
}

local function segment_ok(part)
  if part == nil or part == "" then
    return false
  end
  if not part:match("^[a-z]") then
    return false
  end
  return part:match("^[a-z0-9_-]+$") ~= nil
end

local function tool_id_shape_ok(id)
  local parts = {}
  for part in id:gmatch("[^.]+") do
    parts[#parts + 1] = part
  end
  if #parts ~= 2 then
    return false
  end
  return segment_ok(parts[1]) and segment_ok(parts[2])
end

local function count_of(haystack, needle)
  local n = 0
  local at = 1
  while true do
    local found = haystack:find(needle, at, true)
    if found == nil then
      return n
    end
    n = n + 1
    at = found + #needle
  end
end

test("工具声明面：pack.tools 形状 + 两层白名单同集合 + 内建工具归属 egg", function()
  local pack = __PACK
  local pack_raw = read_file(__MOD_DIR .. "/pack.json")
  assert_true(pack_raw:find("ygg-agent", 1, true) == nil, "pack.json 不得引用退役的 ygg-agent 二进制名")

  -- ① 逐个工具的声明形状。
  local declared = {}
  local max_output_cap = 0
  for id, spec in pairs(pack.tools) do
    assert_true(tool_id_shape_ok(id), ("工具 id `%s` 必须是 <namespace>.<name>"):format(id))
    assert_true(#id <= 64, ("工具 id `%s` 超过 64 字节"):format(id))
    declared[#declared + 1] = id
    assert_eq(spec.kind, "cli", id .. ": 本轮只声明 cli 工具")
    assert_true(type(spec.readOnly) == "boolean", id .. ": readOnly 必须是布尔")
    assert_eq(spec.resultEvent, "tool.result", id .. ": 结果必须回到 Mod 处理的那个事件")
    assert_true(type(spec.timeoutMs) == "number", id .. ": timeoutMs 必须是整数")
    assert_true(type(spec.inlineWaitMs) == "number", id .. ": inlineWaitMs 必须是整数")
    assert_true(spec.readOnly or spec.inlineWaitMs > 0,
      id .. ": 写工具（readOnly=false）不得脱离，inlineWaitMs 必须为正")
    assert_true(spec.inlineWaitMs <= spec.timeoutMs, id .. ": inlineWaitMs 不得超过 timeoutMs")
    local cap = spec.maxOutputBytes or 0
    assert_true(cap >= 1, id .. ": maxOutputBytes 至少为 1")
    if cap > max_output_cap then
      max_output_cap = cap
    end
    local program = spec.program
    assert_true(type(program) == "string", id .. ": program 必须是字符串")
    assert_true(program:sub(1, 1) ~= "/", id .. ": program 不得是绝对路径")
    assert_true(program:find("..", 1, true) == nil, id .. ": program 不得用 .. 逃逸")
    local basename = program:match("([^/]+)$") or program
    assert_true(SHELL_BASENAMES[basename] == nil, ("%s: `%s` 在 shell 黑名单上"):format(id, basename))
    for _, arg in ipairs(spec.args) do
      assert_true(arg:find("$ENV{", 1, true) == nil, ("%s: `%s` 不得内联读取进程环境"):format(id, arg))
      local at = 1
      while true do
        local s = arg:find("{{", at, true)
        if s == nil then
          break
        end
        local e = arg:find("}}", s + 2, true)
        assert_true(e ~= nil, ("%s: `%s` 的 `{{` 未闭合"):format(id, arg))
        local name = arg:sub(s + 2, e - 1)
        local legal = name ~= ""
          and name:match("^[a-z0-9_]+$") ~= nil
          and name:match("^%d") == nil
        assert_true(legal, ("%s: `{{%s}}` 必须是小写标识符"):format(id, name))
        at = e + 2
      end
      assert_eq(count_of(arg, "{{"), count_of(arg, "}}"), ("%s: `%s` 的占位符不平衡"):format(id, arg))
    end
  end
  assert_true(#declared > 0, "工具桥至少需要一个已声明工具")

  -- ② 内建子系统工具由 `egg` 服务；两个朴素的 git 读者仍走 git。
  for _, id in ipairs({
    "git.worktree_claim", "git.worktree_release", "git.lane_probe",
    "fs.artifact_probe", "gate.verify_suite",
  }) do
    assert_eq(pack.tools[id].program, "egg", id .. ": 内建工具由改名后的宿主二进制服务")
  end
  for _, id in ipairs({ "git.head_short", "git.status" }) do
    assert_eq(pack.tools[id].program, "git", id .. ": 朴素 git 读者由 git 本身服务")
  end

  -- ③ `allowedTools` 是请求期闸门：超集——此处恰好同集合。
  local allowed = __ASSETS["ygg:agent/config"].allowedTools
  assert_true(type(allowed) == "table", "allowedTools 必须是数组")
  local seen = {}
  for _, id in ipairs(allowed) do
    assert_true(tool_id_shape_ok(id), ("放行的工具 id `%s` 必须是 <namespace>.<name>"):format(id))
    assert_true(seen[id] == nil, ("放行的工具 `%s` 列了两次"):format(id))
    seen[id] = true
  end
  table.sort(declared)
  table.sort(allowed)
  assert_eq(#allowed, #declared, "allowedTools 与 pack.json tools 必须同集合")
  for i = 1, #declared do
    assert_eq(allowed[i], declared[i], "allowedTools 与 pack.json tools 必须同集合")
  end

  -- ④ 五层模块路径取代了扁平布局。
  local modules = pack.code.modules
  local has_tools, has_flat = false, false
  for _, item in ipairs(modules) do
    if item == "src/runtime/tools.lua" then
      has_tools = true
    end
    if item == "src/tools.lua" then
      has_flat = true
    end
  end
  assert_true(has_tools, "src/runtime/tools.lua 必须在 code.modules 里")
  assert_true(not has_flat, "扁平的 src/tools.lua 布局已退役")

  -- ⑤ inbox 路由与 handler 产出的 wire 一致。
  local route = pack.inboxRoutes.tool_result
  assert_eq(route.action, "tool.result", "路由动作")
  local fields = {
    "call_id", "tool_id", "status", "code", "output", "external_id",
  }
  assert_eq(#route.fields, #fields, "路由字段必须是 wire 段，且有序")
  for i = 1, #fields do
    assert_eq(route.fields[i], fields[i], "路由字段顺序")
  end

  -- ⑥ 片段携带每一个工具 beat（缺键会渲染出裸占位符）。
  local harness_beats = __ASSETS["ygg:scene/beats/harness"].beats
  for _, key in ipairs({
    "tool_requested", "tool_result", "tool_settled", "tool_replayed", "tool_after_done",
    "tool_not_allowed", "tool_allowed_none", "tool_usage", "tool_result_usage",
    "tool_bad_tool_id", "tool_bad_args", "tool_bad_call_id", "tool_call_dup",
    "tool_capacity", "tool_unknown_call", "tool_result_mismatch", "tool_bad_status",
    "tool_bad_code", "tool_output_truncated",
  }) do
    assert_true(harness_beats[key] ~= nil, ("文案片段缺少 `%s`（共享 src/ 会渲染裸占位符）"):format(key))
  end

  -- ⑦ Mod 自己的输出上限不得严于宿主的单工具上限。
  local core_raw = read_file(__MOD_DIR .. "/data/texts/core.json")
  local truncate_at = tonumber(core_raw:match('"tool_output_max"%s*:%s*(%d+)'))
  assert_true(truncate_at ~= nil, "tuning.tool_output_max 必须是整数")
  assert_true(truncate_at >= max_output_cap,
    ("tuning.tool_output_max（%d）低于最大的 maxOutputBytes（%d）"):format(truncate_at, max_output_cap))
end)
