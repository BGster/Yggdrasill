-- perf-compat：用例段自 pkg-role-skill.lua 原样搬移（S-25 批十三测试件连拆）。
return function(C)
  local S, run, eq, eq_seq, is_true, sorted_keys, expect_error, codec, new_world, fresh, events_named, record_of, pkg_sources, source_text =
    C.S, C.run, C.eq, C.eq_seq, C.is_true, C.sorted_keys, C.expect_error, C.codec, C.new_world, C.fresh, C.events_named, C.record_of, C.pkg_sources, C.source_text
run("pkg.role-skill.compat-export-face", function()
  local world = new_world()
  local role = fresh(world)
  local exports = {
    "install",
    "define",
    "skill_define",
    "grant",
    "revoke",
    "has",
    "grant_skill",
    "revoke_skill",
    "charge",
    "recharge",
    "effective",
    "compose",
    "count",
    "reset_counters",
    "skills_of",
    "skill_state",
    "set_capacity",
    "window_push",
    "window",
  }
  for i = 1, #exports do
    eq(type(role[exports[i]]), "function", "导出 " .. exports[i] .. " 必须是函数")
  end
  eq(#exports, 19, "导出面 = 既有 14 + 新增 5（I9：只增不减）")

  eq_seq(
    sorted_keys(role.ERRORS),
    { "ATTR", "COMPOSE", "DEF", "DEPS", "INSTALL", "KV", "UNKNOWN" },
    "错误码仍是那 7 个（不新增码族）"
  )
  eq_seq(
    role.EVENTS,
    { "role.granted", "role.revoked", "role.skill_gained", "role.skill_lost" },
    "事件面仍是那 4 个名字"
  )

  local keys = sorted_keys(role.KEY)
  eq_seq(keys, { "DEFS", "INDEX", "SKILLS", "WINDOW" }, "KV 键名单源 4 条")
  for i = 1, #keys do
    is_true(role.KEY[keys[i]]:find("^role%.") ~= nil, "KEY." .. keys[i] .. " 以 role. 开头")
  end
  eq(type(role.WINDOW_CAP), "number", "WINDOW_CAP 是数字")
end)

-- ── 用例 27：一次 effective 的读次数与实体总数无关（O(1) 的结构证据）───────────

run("pkg.role-skill.perf-o1-structural", function()
  local world = new_world()
  local role = fresh(world)
  role.define("warrior", { attrs_spec = { attack = 3 } })
  for i = 1, 6 do
    role.skill_define("skill" .. i, { attrs_spec = { attack = 1 } })
  end

  --- 直接铺记录：这里测的是 `effective` 的复杂度（与实体**总数**无关）与载荷常驻
  --- 内存。`grant` 走 KV 时每建一个记录要重写整张 `role.index`（「一个键装整张索引」
  --- 的固有成本），拿它当 O(1) 的探针会把两件事混在一起——那属于契约 §5 已声明的
  --- 边界，不是 `effective` 的形状。
  local function seed(count, first)
    for id = first or 1, count do
      local skills = {}
      for i = 1, 6 do
        skills["skill" .. i] = { rank = 1 }
      end
      world.kv["role.actor." .. id] = codec.encode({
        counters = {},
        role = "warrior",
        skills = skills,
      })
      world.set_attr(id, "attack", 10)
    end
  end

  -- ① 1k actor（各 1 角色 + 6 技能）的载荷常驻内存
  -- 解耦钉子（S22）：先预载 3000 条合成记录（≈3× 被测载荷）再取基线——宿主环境与
  -- 注入资产（PokeRogue 面 952 技能定义 + 11 份资源）连同预载全部计入基线，判据
  -- 只量被测 1000 条新记录的增量。若绝对口径回潮或注入漏进判据，读数跳到
  -- ~4-5 MiB 即红：解耦是被钉住的显式语义，不是仪器巧合。（seed 的两参是
  -- count,last：被测段 = 1000 条新记录，id 3001..4000，与预载不重叠。）
  seed(3000, 1)
  collectgarbage("collect")
  collectgarbage("collect")
  local before_mem = collectgarbage("count")
  seed(4000, 3001)
  -- 读之前显式回收：`collectgarbage("count")` 含**未回收**的垃圾，而 seed 里每次
  -- `codec.encode` 都产一张临时表 —— 不回收时这个数随进程 GC 步进漂移（S8 实测同一
  -- 命令三次得 505.7 / 714.4 / 1098.4 KiB；普通 Lua 对照 live 面：老包 333.3 / 新包
  -- 332.6 KiB，差 0.2%）。判据量的是「载荷常驻」，不是「这次恰好留了多少垃圾」。
  collectgarbage("collect")
  collectgarbage("collect")
  local mem_kib = collectgarbage("count") - before_mem
  print(string.format("perf.role-skill.mem_1k_actors_kib=%.1f", mem_kib))
  is_true(mem_kib <= 1024, "1k actor 载荷增量 ≤ 1 MiB（差值口径：基线含宿主与注入资产），实得 " .. string.format("%.1f", mem_kib) .. " KiB")

  -- ② 三档读次数与墙钟
  local function measure(count)
    seed(count)
    local reads_before = world.reads_now()
    local sample = role.effective(1, "attack")
    local ops = world.reads_now() - reads_before
    local began = os.clock()
    local total = 0
    for _ = 1, 200 do
      total = total + role.effective(1, "attack")
    end
    local per_call = (os.clock() - began) / 200
    return ops, per_call, sample, total
  end

  seed(1)
  local ops_one, time_one, sample, total = measure(1)
  local ops_hundred = measure(100)
  local ops_tenk, time_tenk = measure(10000)

  eq(ops_one, 3, "一次 effective 恰好三次 kv.get")
  eq(ops_hundred, ops_one, "N=100 同")
  eq(ops_tenk, ops_one, "N=10k 同")
  eq(sample, 19, "10 + 3 + 6×1（角色 + 六个技能的加成）")
  is_true(total > 0, "累加参与（不让 200 次调用被当成不可观测）")
  print(string.format("perf.role-skill.kv_ops_per_effective=%d", ops_one))

  local ratio = time_tenk / time_one
  print(string.format("perf.role-skill.time_ratio_10k_over_1=%.3f", ratio))
  -- 1.55 而不是 1.5：P-K r1 D-8 在同一台机器上两连同命令实测 1.46 / 1.50，判据压着
  -- 观测红值（S13）；余量按红值 +3.3% 取 1.55。O(N) 回归在本仪器上是 ~10×，判据仍挡得住。
  -- 1.2 更不行：GC 脉冲会被打成假红（r1 在 quest 的同类指标上实证，见 quest.md §5）。
  is_true(ratio <= 1.55, "墙钟比 ≤ 1.55×，实得 " .. string.format("%.3f", ratio))

  -- ③ 读面 / 实例容量 / 窗口 与既有 `charge` 同带对照（契约 §5；三次取中位、print 即口径）
  --
  -- 余量给到 1e9 是为了让 `charge` 恒走「有得扣 ⇒ 必写」的那条路：余量耗尽后它只剩
  -- 一次 kv.get，会把基准压成读面速度，同带比例随之失真。
  local cap = role.WINDOW_CAP
  role.skill_define("pp", { attrs_spec = {}, charges = 1000000000 })
  role.grant_skill(1, "pp")
  role.set_capacity(1, "pp", 1000000000)
  role.recharge(1, "pp")
  for i = 1, cap do
    role.window_push(1, "skill" .. tostring(i % 6 + 1))
  end

  local function bench(fn, iters, rounds)
    local best = math.huge
    local acc = 0
    for _ = 1, rounds or 3 do
      local began = os.clock()
      for _ = 1, iters do
        acc = acc + (fn() or 0)
      end
      local dt = (os.clock() - began) / iters
      if dt < best then
        best = dt
      end
    end
    return best, acc
  end

  local function ops_of(fn)
    local reads_before = world.reads_now()
    local writes_before = world.writes_now()
    fn()
    return world.reads_now() - reads_before, world.writes_now() - writes_before
  end

  local flip = false
  local charge_ms = bench(function()
    return role.charge(1, "pp")
  end, 500)
  local ratios = {
    { "recharge", bench(function()
      return role.recharge(1, "pp", 1)
    end, 500) },
    { "skills_of", bench(function()
      role.skills_of(1)
      return 1
    end, 500) },
    { "skill_state", bench(function()
      role.skill_state(1, "pp")
      return 1
    end, 500) },
    { "set_capacity", bench(function()
      flip = not flip
      return role.set_capacity(1, "pp", flip and 1000000000 or 1000000001)
    end, 500) },
    { "window", bench(function()
      return role.window(1, "skill1", cap)
    end, 500) },
    { "window_push", bench(function()
      return role.window_push(1, "skill2")
    end, 500) },
  }
  for i = 1, #ratios do
    print(string.format("perf.role-skill.ratio_%s=%.3f", ratios[i][1], ratios[i][2] / charge_ms))
  end

  -- 结构证据（与实体总数无关）：窗口与读面的 KV 次数是形状，不是墙钟
  eq(select(1, ops_of(function()
    role.skills_of(1)
  end)), 1, "skills_of = 1 次 kv.get")
  eq(select(1, ops_of(function()
    role.skill_state(1, "pp")
  end)), 1, "skill_state = 1 次 kv.get")
  eq(select(1, ops_of(function()
    role.window(1, "skill1", cap)
  end)), 1, "window = 1 次 kv.get")
  local ring_reads, ring_writes = ops_of(function()
    role.window_push(1, "skill3")
  end)
  eq(ring_reads, 1, "window_push = 1 次 kv.get")
  eq(ring_writes, 1, "window_push = 1 次 kv.set（不碰 actor 记录）")
  local cap_reads, cap_writes = ops_of(function()
    flip = not flip
    role.set_capacity(1, "pp", flip and 1000000000 or 1000000001)
  end)
  eq(cap_reads, 2, "set_capacity = 2 次 kv.get（记录 + 索引回读，与 charge 同形）")
  eq(cap_writes, 1, "set_capacity = 1 次 kv.set")
  print(string.format("perf.role-skill.window_payload_bytes=%d", #world.kv["role.window.1"]))
  print(string.format("perf.role-skill.window_cap=%d", cap))

  -- 判据按计划稿 §5 的原型值 + 余量（selftest 面实测为准，O(N) 回归在本仪器上是 ~10×）
  local function ratio_of(name)
    for i = 1, #ratios do
      if ratios[i][1] == name then
        return ratios[i][2] / charge_ms
      end
    end
    error("unknown ratio " .. name)
  end
  is_true(ratio_of("skills_of") <= 1.0, "skills_of ≤ 1.0×charge，实得 " .. string.format("%.2f", ratio_of("skills_of")))
  is_true(ratio_of("skill_state") <= 1.0, "skill_state ≤ 1.0×charge，实得 " .. string.format("%.2f", ratio_of("skill_state")))
  is_true(ratio_of("set_capacity") <= 1.5, "set_capacity ≤ 1.5×charge，实得 " .. string.format("%.2f", ratio_of("set_capacity")))
  is_true(ratio_of("window") <= 2.5, "window ≤ 2.5×charge，实得 " .. string.format("%.2f", ratio_of("window")))
  is_true(ratio_of("window_push") <= 4.0, "window_push ≤ 4.0×charge，实得 " .. string.format("%.2f", ratio_of("window_push")))
end)

-- ── 用例 28：包加载（清缓存 = 定义表与订阅面一起归零）────────────────────────

run("pkg.role-skill.perf-load", function()
  local samples = {}
  for round = 1, 5 do
    package.loaded["pkg.role_skill"] = nil
    for name in pairs(package.loaded) do
      if type(name) == "string" and name:find("^pkg%.role_skill%.") then
        package.loaded[name] = nil
      end
    end
    local began = os.clock()
    local module = require("pkg.role_skill")
    module.install({ codec = codec })
    samples[round] = (os.clock() - began) * 1000
  end
  table.sort(samples)
  local median = samples[3]
  print(string.format("perf.role-skill.load_ms=%.3f", median))
  is_true(median < 10, "包加载 < 10ms，实得 " .. string.format("%.3f", median) .. "ms")
end)

end
