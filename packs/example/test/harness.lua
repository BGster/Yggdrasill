-- 参考包的自测脚手架：一个**普通 Lua** 里的世界面替身 + 断言。
--
-- 为什么要有替身：`selftest` 跑的是普通解释器（不是内核沙箱），没有 `world`。
-- 这里的替身只实现包用到的那些口（component/spawn/get/set/ids/each/each_mut/
-- system/seal/tick/rng/log + 系统内的 ctx），语义照 `host_api/world.rs`：
-- 组件先声明、u8 有界、each 按 eid 升序、系统内不许调 `world.*`、结构操作延到
-- tick 边界。内核真身那一遍由 `crates/ygg-sandbox/tests/example_pack.rs` 覆盖。

local M = {}

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

-- ── 断言 ────────────────────────────────────────────────────────────────

function M.fail(fmt, ...)
  error(string.format(fmt, ...), 0)
end

--- 一条用例：`body` 正常返回 = 通过；返回字符串或抛错 = 失败。
function M.check(name, body)
  local ok, detail = pcall(body)
  if ok then
    if detail == nil or detail == "" then
      S.pass = S.pass + 1
      return
    end
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(detail))
    return
  end
  S.fail = S.fail + 1
  print("FAIL " .. name .. ": " .. tostring(detail))
end

function M.eq(got, want, what)
  if got ~= want then
    M.fail("%s：期望 %s，得到 %s", what or "值", tostring(want), tostring(got))
  end
end

--- 表按确定序逐项比（只用于扁平值表：id 序列、pos 序列）。
function M.eq_list(got, want, what)
  M.eq(#got, #want, (what or "列表") .. " 长度")
  for i = 1, #want do
    M.eq(got[i], want[i], string.format("%s[%d]", what or "列表", i))
  end
end

-- ── 世界替身 ────────────────────────────────────────────────────────────

local U8_MAX = 255
local U64_MAX = math.maxinteger

local function range_of(kind)
  if kind == "u8" then
    return 0, U8_MAX
  elseif kind == "u64" or kind == "i64" then
    return 0, U64_MAX
  end
  error("未知组件 kind：" .. tostring(kind))
end

local function check_value(kind, value, where)
  local lo, hi = range_of(kind)
  if type(value) ~= "number" or value % 1 ~= 0 then
    error(string.format("%s 必须是整数，得到 %s", where, tostring(value)))
  end
  if value < lo or value > hi then
    error(string.format("%s 超出 %s 范围 [%d, %d]：%d", where, kind, lo, hi, value))
  end
end

--- 造一个新的世界替身（每次调用互相独立）。
function M.new_world()
  local w = {
    tick_now = 0,
    comps = {}, -- name → { index, kind }
    comp_names = {}, -- 声明序
    ents = {}, -- id → { comp = value }
    order = {}, -- 活着的 id，升序
    next_id = 1,
    systems = {}, -- { name, stage, after, run }
    sealed = false,
    busy = nil,
    pending = {},
    seeds = {},
    lines = {},
  }

  local function idle(name)
    if w.busy ~= nil then
      error(string.format("系统 `%s` 里不能调 world.%s（只能摸 ctx）", w.busy, name))
    end
    if w.sealed and (name == "component" or name == "system") then
      error("世界已 seal，不能再声明 world." .. name)
    end
  end

  local function declared(name)
    local comp = w.comps[name]
    if comp == nil then
      error("组件 `" .. tostring(name) .. "` 未声明")
    end
    return comp
  end

  local function live(id)
    return w.ents[id] ~= nil
  end

  local function has_all(id, names)
    local row = w.ents[id]
    if row == nil then
      return false
    end
    for i = 1, #names do
      if row[names[i]] == nil then
        return false
      end
    end
    return true
  end

  local function set_one(id, name, value)
    local comp = declared(name)
    check_value(comp.kind, value, string.format("组件 `%s`", name))
    if w.ents[id] == nil then
      error("实体不存在：" .. tostring(id))
    end
    w.ents[id][name] = value
  end

  local function each(names, f)
    for i = 1, #w.order do
      local id = w.order[i]
      if has_all(id, names) then
        local args = { id }
        for k = 1, #names do
          args[k + 1] = w.ents[id][names[k]]
        end
        f(table.unpack(args, 1, #names + 1))
      end
    end
  end

  local function each_mut(names, f)
    for i = 1, #w.order do
      local id = w.order[i]
      if has_all(id, names) then
        local args = { id }
        for k = 1, #names do
          args[k + 1] = w.ents[id][names[k]]
        end
        local rets = table.pack(f(table.unpack(args, 1, #names + 1)))
        if rets.n ~= #names then
          error(string.format(
            "world.each_mut：回调必须返回 %d 个值，得到 %d 个",
            #names,
            rets.n
          ))
        end
        for k = 1, #names do
          set_one(id, names[k], rets[k])
        end
      end
    end
  end

  local function ids(names)
    local out = {}
    for i = 1, #w.order do
      if has_all(w.order[i], names) then
        out[#out + 1] = w.order[i]
      end
    end
    return out
  end

  local function spawn(payload)
    local row = {}
    for name, value in pairs(payload) do
      local comp = declared(name)
      check_value(comp.kind, value, string.format("组件 `%s`", name))
      row[name] = value
    end
    local id = w.next_id
    w.next_id = id + 1
    w.ents[id] = row
    w.order[#w.order + 1] = id
    return id
  end

  local function despawn(id)
    if not live(id) then
      return 0
    end
    w.ents[id] = nil
    for i = 1, #w.order do
      if w.order[i] == id then
        table.remove(w.order, i)
        break
      end
    end
    return 1
  end

  local function apply_pending()
    local queue = w.pending
    w.pending = {}
    for i = 1, #queue do
      local op = queue[i]
      if op.tag == "spawn" then
        spawn(op.payload)
      elseif op.tag == "despawn" then
        despawn(op.payload)
      else
        error("未知的延迟结构操作：" .. tostring(op.tag))
      end
    end
  end

  -- rng：每个 stream 一条 LCG（够确定就行——跨机一致靠内核的 rng32）。
  local function next_rng(stream)
    local seed = w.seeds[stream] or 1
    seed = (seed * 1103515245 + 12345) % 2147483648
    w.seeds[stream] = seed
    return seed
  end

  -- 系统内面：只有这几个口，且都没有 busy 检查（它就是允许的那一侧）。
  local function make_ctx(dt)
    return {
      tick = function()
        return w.tick_now
      end,
      dt = function()
        return dt
      end,
      ids = function(names)
        return ids(names)
      end,
      each = function(names, f)
        each(names, f)
      end,
      each_mut = function(names, f)
        each_mut(names, f)
      end,
      rng = function(stream)
        return next_rng(stream)
      end,
      spawn = function(payload)
        w.pending[#w.pending + 1] = { tag = "spawn", payload = payload }
      end,
      despawn = function(id)
        w.pending[#w.pending + 1] = { tag = "despawn", payload = id }
      end,
      insert = function(id, payload)
        for name, value in pairs(payload) do
          if w.ents[id] == nil then
            error("实体不存在：" .. tostring(id))
          end
          set_one(id, name, value)
        end
      end,
    }
  end

  -- 系统外面（= 宿主与包模块看到的 world）。
  w.component = function(spec)
    idle("component")
    if w.comps[spec.name] ~= nil then
      error("组件 `" .. tostring(spec.name) .. "` 已声明过")
    end
    range_of(spec.kind) -- 未知 kind 立刻报
    w.comps[spec.name] = { index = #w.comp_names, kind = spec.kind }
    w.comp_names[#w.comp_names + 1] = spec.name
    return w.comps[spec.name].index
  end
  w.spawn = function(payload)
    idle("spawn")
    return spawn(payload)
  end
  w.despawn = function(id)
    idle("despawn")
    return despawn(id)
  end
  w.insert = function(id, payload)
    idle("insert")
    for name, value in pairs(payload) do
      set_one(id, name, value)
    end
  end
  w.set = function(id, payload)
    idle("set")
    local n = 0
    for name, value in pairs(payload) do
      set_one(id, name, value)
      n = n + 1
    end
    return n
  end
  w.get = function(id, name)
    idle("get")
    local row = w.ents[id]
    if row == nil then
      return nil
    end
    return row[name]
  end
  w.ids = function(names)
    idle("ids")
    return ids(names)
  end
  w.each = function(names, f)
    idle("each")
    each(names, f)
  end
  w.each_mut = function(names, f)
    idle("each_mut")
    each_mut(names, f)
  end
  w.system = function(spec)
    idle("system")
    for i = 1, #(spec.writes or {}) do
      declared(spec.writes[i])
    end
    for i = 1, #(spec.after or {}) do
      local found = false
      for k = 1, #w.systems do
        local other = w.systems[k]
        if other.name == spec.after[i] then
          -- 内核只允许 after 指向**同阶段**的系统（跨阶段顺序由阶段本身保证）。
          if other.stage ~= spec.stage then
            error(string.format(
              "系统 `%s`（阶段 %s）的 after 指向别的阶段的系统 `%s`（阶段 %s）",
              spec.name,
              spec.stage,
              other.name,
              other.stage
            ))
          end
          found = true
          break
        end
      end
      if not found then
        error(string.format("系统 `%s` 的 after 引用了未注册的系统 `%s`", spec.name, spec.after[i]))
      end
    end
    w.systems[#w.systems + 1] = {
      name = spec.name,
      stage = spec.stage,
      writes = spec.writes or {},
      after = spec.after or {},
      run = spec.run,
    }
  end
  w.seal = function()
    idle("seal")
    -- 内核在 seal 时拒绝**同一阶段内写同一组件**的两个系统（YGG-SIM-SEALED）：
    -- 「同段谁先写」无法用 after 表达成两个写者共存，所以这是声明错误。
    -- 替身必须同样严格——否则用例会跑过一份装载期就挂的包（M3.4a 踩过）。
    for i = 1, #w.systems do
      local a = w.systems[i]
      for k = i + 1, #w.systems do
        local b = w.systems[k]
        if a.stage == b.stage then
          for x = 1, #a.writes do
            for y = 1, #b.writes do
              if a.writes[x] == b.writes[y] then
                error(string.format(
                  "阶段 %s 内写写冲突：系统 `%s` 与 `%s` 都写组件 `%s`",
                  a.stage,
                  a.name,
                  b.name,
                  a.writes[x]
                ))
              end
            end
          end
        end
      end
    end
    w.sealed = true
  end
  w.tick = function()
    idle("tick")
    return w.tick_now
  end
  w.rng = function(stream)
    idle("rng")
    return next_rng(stream)
  end
  w.log = function(fmt, ...)
    w.lines[#w.lines + 1] = string.format(fmt, ...)
  end

  --- 跑 n 个 tick（每个 tick：pre → rules → post，然后落地结构操作、tick+1）。
  function w.step(n, dt)
    for _ = 1, n do
      dt = dt or 1
      local async = make_ctx(dt)
      local stages = { "pre", "rules", "post" }
      for s = 1, #stages do
        for i = 1, #w.systems do
          local sys = w.systems[i]
          if sys.stage == stages[s] then
            w.busy = sys.name
            sys.run(dt, async)
            w.busy = nil
          end
        end
      end
      apply_pending()
      w.tick_now = w.tick_now + 1
    end
  end

  return w
end

--- 重置到「刚装载」的状态：新世界 + 清掉包模块缓存，下次 require 重新装配。
function M.fresh()
  local w = M.new_world()
  _G.world = w
  for name in pairs(package.loaded) do
    if string.sub(name, 1, 4) == "src." then
      package.loaded[name] = nil
    end
  end
  return w
end

--- 棋盘数据（`ygg:example/board`，由 selftest 把 data/board.json 转成 Lua 值注入）。
function M.board_data(id)
  local assets = rawget(_G, "__ASSETS")
  if type(assets) ~= "table" or assets[id] == nil then
    error("缺少资源：" .. tostring(id))
  end
  return assets[id]
end

--- 深拷贝（棋盘数据是按引用给的，用例之间不能互相污染）。
function M.copy(value)
  if type(value) ~= "table" then
    return value
  end
  local out = {}
  for k, v in pairs(value) do
    out[k] = M.copy(v)
  end
  return out
end

-- `ygg.data(id)` 的替身（内核会在装载期注入真身）。
_G.ygg = {
  data = function(id)
    return M.copy(M.board_data(id))
  end,
}

_G.world = M.new_world()
_G.check = M.check
_G.eq = M.eq
_G.eq_list = M.eq_list
_G.fail = M.fail
_G.fresh = M.fresh
_G.copy = M.copy
_G.board_data = M.board_data

return M
