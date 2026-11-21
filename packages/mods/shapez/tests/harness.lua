-- `packages/mods/shapez/tests/harness.lua` —— 本 mod 自检的 ygg 替身。
--
-- 与 superagent harness 同族（`check()` 判定 + `__SELFTEST` 同一张计数面），
-- 面向 v1 headless 模拟：kv / entity / sim / on / narrate / emit / meta / data。
-- 三条刻意严格（沿 pkg-selftest 先例）：
--   1. `emit` 只收标量（真内核 unsupported payload type 的同口径前哨）；
--   2. `kv.set` 只收标量（ygg-core KV 的四标量型纪律）；
--   3. `sim.step` 收定点整数 dt（真内核 YGG-SIM-FIXED 闸门同口径）。
--
-- `reset_all()`：清 KV / 实体 / 叙事 / 事件 / meta / sim 拍数（保留 handler），
-- 供重放对账（I3）在同一进程内开两局。模块内缓存的失效靠 belts 指纹
-- （带数+最大 id）——reset 后实体集合换血即强制重建。
--
-- 已知缺口（selftest.rs 头注同款）：driver 层事务回滚 / fuel / D1 choice 语义
-- 不在此覆盖。`ygg.choice` 记 pending 后正常返回。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

--- 记一条用例：`body()` 返回非空字符串（或 nil 以外的问题清单）即失败。
function check(name, body)
  local ok, verdict = pcall(body)
  local detail = nil
  if not ok then
    detail = verdict
  elseif type(verdict) == "string" and #verdict > 0 then
    detail = verdict
  end
  if detail == nil then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(detail))
  end
end

--- 断言相等。
function assert_eq(actual, expected, what)
  if actual ~= expected then
    error((what or "值") .. " 不相等：实得 " .. tostring(actual) .. "，期望 " .. tostring(expected), 2)
  end
  return true
end

--- 断言真。
function assert_true(value, what)
  if not value then
    error(what or "应为真", 2)
  end
  return true
end

--- 断言错误码子串（包面错误逐码断言的 Sugar）。
function assert_error(code, body, what)
  local ok, err = pcall(body)
  if ok then
    error((what or "应报错") .. " 却成功", 2)
  end
  if type(err) ~= "string" or err:find(code, 1, true) == nil then
    error((what or "应报错") .. " 码不符：期望含 " .. code .. "，实得 " .. tostring(err), 2)
  end
  return true
end

-- ── 状态缓冲 ────────────────────────────────────────────────────────────────

local __kv = {}
local __handlers = {}
local __narrated = {}
local __emitted = {}
local __meta = {}
local __pending = nil
local __sim_ticks = 0
local __ents = { next = 0, recs = {} }

-- ── 最小 `ygg` 替身 ─────────────────────────────────────────────────────────

ygg = {
  engine_version = "selftest",

  data = function(id)
    if type(id) ~= "string" or #id == 0 then
      error("ygg.data(id) 需要非空字符串 id，得到 " .. type(id), 0)
    end
    local assets = rawget(_G, "__ASSETS")
    if assets == nil then
      error("ygg.data: 资源表 __ASSETS 未注入（runner 装载序错误）", 0)
    end
    local value = assets[id]
    if value == nil then
      error("ygg.data: 未声明的资源 id：" .. id, 0)
    end
    return value
  end,

  on = function(name, handler)
    assert(type(name) == "string" and #name > 0, "ygg.on 需要非空事件名")
    assert(type(handler) == "function", "ygg.on 需要 handler 函数")
    local bucket = __handlers[name]
    if bucket == nil then
      bucket = {}
      __handlers[name] = bucket
    end
    bucket[#bucket + 1] = handler
  end,

  narrate = function(text)
    __narrated[#__narrated + 1] = tostring(text)
  end,

  emit = function(name, payload)
    local kind = type(payload)
    assert(
      kind ~= "table" and kind ~= "function",
      "ygg.emit 的载荷只收标量，得到 " .. kind
    )
    __emitted[#__emitted + 1] = { name = name, payload = payload }
  end,

  meta = function(key, value)
    __meta[tostring(key)] = value
  end,

  choice = function(prompt, options)
    __pending = { prompt = prompt, options = options }
  end,

  -- 宿主实体面替身（pkg.entity 的底座）。
  entity = {
    spawn = function()
      __ents.next = __ents.next + 1
      __ents.recs[__ents.next] = { attrs = {}, tags = {} }
      return __ents.next
    end,
    set_attribute = function(id, key, value)
      local rec = __ents.recs[id]
      if rec == nil then
        error("ygg.entity.set_attribute: 实体不存在 " .. tostring(id), 0)
      end
      local kind = type(value)
      assert(kind == "string" or kind == "number" or kind == "boolean", "实体属性只收标量")
      rec.attrs[key] = value
      return true
    end,
    get_attribute = function(id, key)
      local rec = __ents.recs[id]
      if rec == nil then
        return nil
      end
      return rec.attrs[key]
    end,
    add_tag = function(id, tag)
      local rec = __ents.recs[id]
      assert(rec ~= nil, "ygg.entity.add_tag: 实体不存在")
      if rec.tags[tag] then
        return false
      end
      rec.tags[tag] = true
      return true
    end,
    has_tag = function(id, tag)
      return __ents.recs[id] ~= nil and __ents.recs[id].tags[tag] == true
    end,
    remove_tag = function(id, tag)
      local rec = __ents.recs[id]
      if rec == nil or not rec.tags[tag] then
        return false
      end
      rec.tags[tag] = nil
      return true
    end,
    exists = function(id)
      return __ents.recs[id] ~= nil
    end,
    destroy = function(id)
      local rec = __ents.recs[id]
      if rec == nil then
        return false
      end
      __ents.recs[id] = nil
      return true
    end,
    ids = function(_schema)
      local out = {}
      for id in pairs(__ents.recs) do
        out[#out + 1] = id
      end
      for i = 1, #out do
        local min = i
        for j = i + 1, #out do
          if out[j] < out[min] then
            min = j
          end
        end
        if min ~= i then
          out[i], out[min] = out[min], out[i]
        end
      end
      return out
    end,
    count = function()
      local n = 0
      for _ in pairs(__ents.recs) do
        n = n + 1
      end
      return n
    end,
  },

  kv = {
    get = function(key)
      return __kv[key]
    end,
    set = function(key, value)
      local kind = type(value)
      assert(
        kind == "string" or kind == "number" or kind == "boolean",
        "ygg.kv.set 只收标量（四标量型纪律），得到 " .. kind
      )
      __kv[key] = value
    end,
  },

  sim = {
    step = function(dt)
      assert(type(dt) == "number" and dt == (dt // 1), "sim.step 收定点整数 dt")
      __sim_ticks = __sim_ticks + dt
      return { tick = __sim_ticks, hash = "mock" .. __sim_ticks }
    end,
    hash = function()
      return "mock" .. __sim_ticks
    end,
    status = function()
      return { tick = __sim_ticks, hash = "mock" .. __sim_ticks }
    end,
  },
}

-- ── 读回面 / 重置面 ─────────────────────────────────────────────────────────

--- 派发一个事件（按注册序跑全部 handler；返回最后非 nil 返回值）。
function ygg_dispatch(name, payload)
  local bucket = __handlers[name]
  assert(bucket ~= nil, "事件未注册：" .. tostring(name))
  local result = nil
  for i = 1, #bucket do
    result = bucket[i](payload)
  end
  return result
end

--- 已收集的叙事行副本。
function ygg_narrated_lines()
  local out = {}
  for i = 1, #__narrated do
    out[i] = __narrated[i]
  end
  return out
end

--- 已收集的派生事件副本。
function ygg_emitted_events()
  local out = {}
  for i = 1, #__emitted do
    out[i] = { name = __emitted[i].name, payload = __emitted[i].payload }
  end
  return out
end

--- KV 快照读。
function ygg_kv_all()
  return __kv
end

--- meta 读回。
function ygg_meta_all()
  return __meta
end

--- 开新一局：清状态、留 handler。belts 缓存靠指纹（实体换血）自动失效。
function reset_all()
  for key in pairs(__kv) do
    __kv[key] = nil
  end
  for id in pairs(__ents.recs) do
    __ents.recs[id] = nil
  end
  __ents.next = 0
  for i = 1, #__narrated do
    __narrated[i] = nil
  end
  for i = 1, #__emitted do
    __emitted[i] = nil
  end
  for key in pairs(__meta) do
    __meta[key] = nil
  end
  __pending = nil
  __sim_ticks = 0
end
