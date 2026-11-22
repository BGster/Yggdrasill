-- surface-view.lua —— S32 纯搬移自 surface.lua L565-801（requirement.list 分页 + summary 投影 + map.show + 泳道命名；自包含分组）。
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
