-- `packages/mods/superagent/tests/harness.lua` —— 本 mod 自检的 ygg 替身。
--
-- 与 pkg-selftest 的夹具同族（`check()` 判定 + `__SELFTEST` 同一张计数面），
-- 但本 mod 的自测要穿过**真包面**（pkg.entity / pkg.quest / pkg.dialogue /
-- pkg.role_skill / pkg.codec / pkg.procgen 经 searcher 装载），所以替身要补齐
-- 它们消费的宿主面：kv / narrate / emit / meta / choice / on / sim / fs。
--
-- 三条刻意严格（沿 pkg-selftest 先例）：
--   1. `emit` 只收标量（真内核 `unsupported payload type: table` 的同口径前哨）；
--   2. `kv.set` 只收标量（ygg-core KV 的四标量型纪律）；
--   3. `sim.spawn_batch` **真解码**帧字节（魔数/版本/flags/rows/组件数/尾 digest
--      逐段校验）——编帧 bug 在这一面当场红，而不是被「什么都收」的 mock 吞掉。
--
-- 已知缺口（selftest.rs 头注同款）：driver 层事务回滚 / 存档 receipt /
-- D1 choice 语义不在此覆盖。`ygg.choice` 在替身里记 pending 后**正常返回**
-- （真内核是 sentinel 终止 + 丢弃先续 emit——两处差异由真实跑局暴露）。

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

--- 断言相等（自检方言里最常用的 Sugar）。
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

-- ── 状态缓冲 ────────────────────────────────────────────────────────────────

local __kv = {}
local __handlers = {}
local __narrated = {}
local __emitted = {}
local __meta = {}
local __pending = nil
local __sim_ticks = 0
local __spawned_batches = {}
local __vfs = {} -- 内存 VFS（artifacts/ 与 mounts/ 同一张表，键即路径）
local __ents = { next = 0, recs = {} } -- 宿主实体 store 替身（pkg.entity 的底座）

-- ── sim 帧解码（canonical.rs FrameReader 的最小同构）────────────────────────

local FNV_OFFSET = 0xCBF29CE484222325
local FNV_PRIME = 0x100000001B3

local function fnv1a64(bytes)
  local hash = FNV_OFFSET
  for i = 1, #bytes do
    hash = hash ~ bytes:byte(i)
    hash = hash * FNV_PRIME
  end
  return hash
end

--- 从字节串取 [pos, pos+len) 的 8 字节小端（返回 Lua 整数位型）。
local function peek_u64(bytes, pos)
  local value = 0
  for k = 7, 0, -1 do
    value = (value << 8) | bytes:byte(pos + k)
  end
  return value
end

--- 校验一个裸实体批帧：返回 rows（任何一段不对 = error）。
local function decode_spawn_frame(frame)
  local magic = frame:sub(1, 8)
  if magic ~= "YGGSIM\0\1" then
    error("帧魔数不符：" .. string.format("%q", magic))
  end
  local version = frame:byte(9) | (frame:byte(10) << 8)
  assert_eq(version, 2, "帧版本")
  local flags = frame:byte(11) | (frame:byte(12) << 8)
  assert_eq(flags, 0, "帧 flags")
  local rows = frame:byte(13) | (frame:byte(14) << 8) | (frame:byte(15) << 16) | (frame:byte(16) << 24)
  local comp_n = frame:byte(17) | (frame:byte(18) << 8)
  assert_eq(comp_n, 0, "裸实体批组件数")
  local digest_stored = peek_u64(frame, 19)
  local digest_calc = fnv1a64(frame:sub(1, 18))
  assert_eq(digest_stored, digest_calc, "帧尾 digest")
  assert_eq(#frame, 18 + 8, "帧总长（18 头 + 8 尾）")
  return rows
end

-- ── 最小 `ygg` 替身 ─────────────────────────────────────────────────────────

ygg = {
  engine_version = "selftest",

  -- 资源面：runner 先把 pack.json entries 注入全局 __ASSETS（id → 值）。
  -- 语义同 resources.rs：id 缺失/非字符串 = 可读错误，不返 nil。
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
      "ygg.emit 的载荷只收标量（与 value_from_lua 同口径），得到 " .. kind
    )
    __emitted[#__emitted + 1] = { name = name, payload = payload }
  end,

  meta = function(key, value)
    __meta[tostring(key)] = value
  end,

  choice = function(prompt, options)
    __pending = { prompt = prompt, options = options }
  end,

  -- 宿主实体面替身（真内核 14 键；pkg.entity 在其上叠 schema/属性袋）。
  -- query / destroy 未实现（本 mod 自测不触；触到 = 响亮失败，不静默）。
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
      local rec = __ents.recs[id]
      return rec ~= nil and rec.tags[tag] == true
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
    query = function(_)
      error("selftest 替身未实现 ygg.entity.query（本 mod 自测不触）", 0)
    end,
    destroy = function(_)
      error("selftest 替身未实现 ygg.entity.destroy（本 mod 自测不触）", 0)
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

  fs = {
    write = function(path, content)
      assert(type(path) == "string" and type(content) == "string", "ygg.fs.write 收 (String, String)")
      if #content > 256 * 1024 then
        error("ygg.fs.write 拒绝：内容超 256 KiB 上限")
      end
      __vfs[path] = content
      return { path = path, bytes = #content, sha256 = "mock-sha" }
    end,
    read = function(path)
      return __vfs[path]
    end,
    stat = function(path)
      local blob = __vfs[path]
      if blob == nil then
        return { exists = false, bytes = 0, sha256 = "", kind = "file" }
      end
      return { exists = true, bytes = #blob, sha256 = "mock-sha", kind = "file" }
    end,
    list = function(path)
      local out = {}
      local prefix = path .. "/"
      for key in pairs(__vfs) do
        if key:sub(1, #prefix) == prefix then
          local blob = __vfs[key]
          out[#out + 1] = {
            name = key:sub(#prefix + 1),
            path = key,
            bytes = #blob,
            sha256 = "",
            kind = "file",
          }
        end
      end
      -- 字典序（宿主 BTreeMap 序）。
      for i = 1, #out do
        local min = i
        for j = i + 1, #out do
          if out[j].path < out[min].path then
            min = j
          end
        end
        if min ~= i then
          out[i], out[min] = out[min], out[i]
        end
      end
      return out
    end,
  },

  sim = {
    step = function(dt)
      assert(type(dt) == "number" and dt == (dt // 1), "sim.step 收定点整数 dt")
      __sim_ticks = __sim_ticks + dt
      return __sim_ticks, "mockhash"
    end,
    tick = function()
      return __sim_ticks
    end,
    spawn_batch = function(frame)
      assert(type(frame) == "string", "sim.spawn_batch 收字节串")
      local rows = decode_spawn_frame(frame)
      __spawned_batches[#__spawned_batches + 1] = rows
      return { spawned = rows, next_id = rows }
    end,
    hash = function()
      return "mockhash" .. tostring(__sim_ticks)
    end,
    status = function()
      return { tick = __sim_ticks, hash = "mockhash" .. tostring(__sim_ticks) }
    end,
  },
}

-- ── 读回面（用例断言用）────────────────────────────────────────────────────

--- 已登记的事件名（升序）。
function ygg_event_names()
  local names = {}
  for name in pairs(__handlers) do
    names[#names + 1] = name
  end
  for i = 1, #names do
    local min = i
    for j = i + 1, #names do
      if names[j] < names[min] then
        min = j
      end
    end
    if min ~= i then
      names[i], names[min] = names[min], names[i]
    end
  end
  return names
end

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

--- 最近一次 pending choice（nil = 无）。
function ygg_last_choice()
  return __pending
end

--- sim 批次记录（decode 后的 rows 序列）。
function ygg_spawn_batches()
  return __spawned_batches
end

--- meta 收集面读回（C5 信封落账断言用）。
function ygg_meta_all()
  return __meta
end

--- VFS 快照读（挂载写穿断言用）。
function ygg_vfs()
  return __vfs
end

--- KV 快照读（断言面）。
function ygg_kv_all()
  return __kv
end
