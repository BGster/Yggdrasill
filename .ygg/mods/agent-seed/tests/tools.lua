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

test("tool.call / tool.result 载荷形状与白名单：17 个拒绝分支 + 容量", function()
  fresh()
  fire("event.start", "tools gate|synthetic task description")

  local long_value = string.rep("v", 96)
  local long_args = "a1=" .. long_value .. ";a2=" .. long_value .. ";a3=" .. long_value
  local long_call_id = "t" .. string.rep("a", 32)

  -- 请求侧形状：11 个分支，全部零副作用。
  local refusals = {
    { "gh.issue", "gh.issue" },
    { "Git", "Git" },
    { "no-dot", "no-dot" },
    { "git.head_short|rev=HEAD;rev=x", "rev=" },
    { "git.head_short|abc", "not `k=v`" },
    { "git.head_short|a b=c", "must match" },
    { "git.head_short|a1=1;a2=2;a3=3;a4=4;a5=5;a6=6;a7=7;a8=8;a9=9", "8" },
    { "git.head_short|" .. long_args, "192" },
    { "git.head_short||T1", "T1" },
    { "git.head_short||count", "count" },
    { "git.head_short||" .. long_call_id, "not usable" },
  }
  for _, item in ipairs(refusals) do
    reset()
    fire("tool.call", item[1])
    assert_true(
      narrated_contains(item[2]),
      ("拒绝必须点名 `%s`，实际：%s"):format(item[2], table.concat(narrated(), " ⏎ "))
    )
    assert_nil(kv("tool.count"), "拒绝零副作用（无计数）")
    assert_nil(kv("tool.pending"), "拒绝零副作用（无待决）")
  end

  -- 真实请求分配 `t1`；再用同一 id 被拒（重放要靠 external_id）。
  fire("tool.call", "git.head_short")
  assert_eq(kv("tool.count"), 1, "真实请求分配一个槽位")
  reset()
  fire("tool.call", "git.head_short||t1")
  assert_true(narrated_contains("t1"), "复用同一 call id 被拒")
  assert_eq(kv("tool.count"), 1, "计数不变")

  -- 结果侧形状：6 个分支，全部保留 pending 状态。
  local result_refusals = {
    { "t99|git.head_short|ok|0|x", "t99" },
    { "t1|git.status|ok|0|x", "t1" },
    { "t1|git.head_short|maybe|0|x", "maybe" },
    { "t1|git.head_short|ok|1.5|x", "1.5" },
    { "t1|git.head_short|ok|0", "tool.result" },
    { "t1|git.head_short|ok|0|x|e|extra", "tool.result" },
  }
  for _, item in ipairs(result_refusals) do
    reset()
    fire("tool.result", item[1])
    assert_true(
      narrated_contains(item[2]),
      ("结果拒绝必须点名 `%s`，实际：%s"):format(item[2], table.concat(narrated(), " ⏎ "))
    )
    assert_eq(kv("tool.t1.status"), "pending", "拒绝后仍待决")
    assert_eq(kv("tool.pending"), 1, "待决计数不变")
  end

  -- 容量：计数器是生命周期计数器，占满即拒。
  local core_raw = read_file(__MOD_DIR .. "/data/texts/core.json")
  local capacity = tonumber(core_raw:match('"tool_capacity"%s*:%s*(%d+)'))
  assert_true(capacity ~= nil, "tuning.tool_capacity 必须是整数")
  fresh()
  fire("event.start", "tools gate|synthetic task description")
  for _ = 1, capacity do
    fire("tool.call", "git.head_short")
  end
  assert_eq(kv("tool.count"), capacity, "每个槽位都被占用")
  reset()
  fire("tool.call", "git.head_short")
  assert_true(narrated_contains("no free tool slot"), "占满后拒绝")
  assert_eq(kv("tool.count"), capacity, "计数不变")
end)

test("tool.call 发布密集待决快照：分配序、args 省略、meta 不拖 summary", function()
  fresh()
  fire("event.start", "tools gate|synthetic task description")

  reset()
  fire("tool.call", "git.head_short")
  local calls = __SELFTEST.meta.tool_calls
  assert_true(type(calls) == "table", "meta.tool_calls 必须是数组")
  assert_eq(#calls, 1, "一次请求入队一个 call")
  assert_eq(calls[1].id, "t1", "分配序从 t1 起")
  assert_eq(calls[1].tool, "git.head_short", "工具名")
  assert_true(calls[1].args == nil,
    "空参数集必须**省略** args 键（Lua 空表只能序列化成 []）")
  assert_true(__SELFTEST.meta.summary == nil,
    "一次派发只写它自己动作设置的键——tool_calls 不得把 summary 拖进来")

  assert_eq(kv("tool.count"), 1, "计数")
  assert_eq(kv("tool.pending"), 1, "待决计数")
  assert_eq(kv("tool.t1.status"), "pending", "行状态")
  assert_eq(kv("tool.t1.code"), -1, "未跑过 code=-1")
  assert_eq(kv("tool.t1.output"), "", "输出空")
  assert_true(narrated_contains("[TOOL REQUESTED]"), "必须叙述请求行")
  assert_true(
    narrated_contains("t1 -> git.head_short"),
    "请求行必须带分配序：" .. table.concat(narrated(), " | ")
  )

  reset()
  fire("tool.call", "git.status|path=.|t2")
  local calls2 = __SELFTEST.meta.tool_calls
  assert_eq(#calls2, 2, "两条都仍待决")
  assert_eq(calls2[1].id, "t1", "数组顺序 = 分配顺序")
  assert_eq(calls2[2].id, "t2", "数组顺序 = 分配顺序")
  assert_eq(calls2[2].args.path, ".", "参数进快照")
  assert_eq(kv("tool.pending"), 2, "待决计数")
  assert_eq(kv("tool.t2.status"), "pending", "第二行状态")
end)

test("tool.result：同回合回执收敛 + 重放幂等 + 终态冲突拒绝", function()
  -- ① 同回合收敛：结果落库，回执效应在唯一选择产出前就绪。
  drive_to_settle()
  fire("tool.call", "git.head_short")
  reset()
  fire("tool.result", "t1|git.head_short|ok|0|abc1234")
  assert_eq(count_marker("[TOOL RESULT]"), 1, "结果行恰好一次")
  assert_eq(count_marker("[TOOL SETTLED]"), 1, "回执效应必须在唯一选择产出前收敛")

  assert_eq(kv("tool.t1.status"), "ok", "行状态")
  assert_eq(kv("tool.t1.code"), 0, "code")
  assert_eq(kv("tool.t1.output"), "abc1234", "输出")
  assert_eq(kv("tool.pending"), 0, "队列清空")
  assert_eq(kv("tool.last"), "t1", "last 指向最后结清的 call")
  assert_eq(kv("tool.notified.t1"), true, "回执闩锁")
  assert_eq(#__SELFTEST.meta.tool_calls, 0, "已结清的 call 留下空队列（不是缺键）")

  -- ② 新 external_id 只填幂等键。
  reset()
  fire("tool.result", "t1|git.head_short|ok|0|abc1234|ext-1")
  assert_eq(kv("tool.t1.res_ext"), "ext-1", "外部 id 落幂等键")

  -- ③ 同 external_id = 重放：无 RESULT、无第二次 SETTLED。
  reset()
  fire("tool.result", "t1|git.head_short|ok|0|abc1234|ext-1")
  assert_eq(count_marker("[REPLAY:TOOL]"), 1, "重放行恰好一次")
  assert_eq(count_marker("[TOOL RESULT]"), 0, "重放不得再写结果行")
  assert_eq(count_marker("[TOOL SETTLED]"), 0, "重放不得再触发回执")
  assert_eq(kv("tool.pending"), 0, "待决计数不变")

  -- ④ 不同终态 = 冲突，不是覆盖。
  reset()
  fire("tool.result", "t1|git.head_short|err|1|boom|ext-2")
  assert_true(narrated_contains("t1"), "冲突必须点名 call id：" .. table.concat(narrated(), " | "))
  assert_eq(kv("tool.t1.status"), "ok", "终态不被覆盖")
  assert_eq(kv("tool.t1.output"), "abc1234", "输出不被覆盖")
end)
