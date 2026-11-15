-- .ygg/mods/agent-seed/tests/harness.lua
--
-- 纯 Lua 自检的宿主替身（mock `ygg`）与断言工具。
--
-- 边界：这里跑的是**普通 Lua**，不是内核沙箱。`ygg.*` 是可观测的替身，
-- 因此断言能覆盖 mod 的业务语义，但覆盖不到 driver 层（事务回滚 / fuel /
-- save receipt / 事件路由）。见 docs/design/mod/testing-strategy.md。

------------------------------------------------------------------------------
-- 1. 收集面
------------------------------------------------------------------------------

local KV = {}
local NARRATED = {}
local EMITTED = {}
local HANDLERS = {}

ygg = {
  engine_version = "selftest",
  ticks = function()
    return 0
  end,
  kv = {
    get = function(key)
      return KV[key]
    end,
    set = function(key, value)
      KV[key] = value
    end,
  },
  narrate = function(line)
    NARRATED[#NARRATED + 1] = line
  end,
  emit = function(name, payload)
    EMITTED[#EMITTED + 1] = { name = name, payload = payload }
  end,
  on = function(name, fn)
    HANDLERS[name] = fn
  end,
  data = function(id)
    return __ASSETS[id]
  end,
  choice = function(prompt, options)
    __SELFTEST.lastChoice = { prompt = prompt, options = options }
  end,
  meta = function(key, value)
    __SELFTEST.meta[key] = value
  end,
  narrative = {
    node = function() end,
    ["goto"] = function() end,
  },
  fsm = {
    define = function(def)
      __SELFTEST.fsm = def
    end,
  },
  fs = {},
  tool = {
    call = function() end,
  },
  choice_selected = function() end,
  choice_selector = function() end,
}

------------------------------------------------------------------------------
-- 2. 用例收集
------------------------------------------------------------------------------

__SELFTEST = {
  pass = 0,
  fail = 0,
  names = {},
  meta = {},
  lastChoice = nil,
  fsm = nil,
}

local function describe(err)
  if type(err) == "string" then
    return err
  end
  return tostring(err)
end

--- 注册并立即执行一条用例。用例失败不中断同文件内的其它用例。
function test(name, fn)
  local ok, err = pcall(fn)
  if ok then
    __SELFTEST.pass = __SELFTEST.pass + 1
    __SELFTEST.names[#__SELFTEST.names + 1] = name
  else
    __SELFTEST.fail = __SELFTEST.fail + 1
    print(("  FAIL  %s\n        %s"):format(name, describe(err)))
  end
end

------------------------------------------------------------------------------
-- 3. 断言
------------------------------------------------------------------------------

function assert_true(value, message)
  if not value then
    error(message or "expected truthy", 2)
  end
end

function assert_nil(value, message)
  if value ~= nil then
    error((message or "expected nil") .. (" (got %s)"):format(tostring(value)), 2)
  end
end

function assert_eq(actual, expected, message)
  if actual ~= expected then
    error(
      ("%s\n        expected: %s\n        actual:   %s"):format(
        message or "values differ",
        tostring(expected),
        tostring(actual)
      ),
      2
    )
  end
end

function assert_contains(haystack, needle, message)
  if type(haystack) ~= "string" or not haystack:find(needle, 1, true) then
    error(
      ("%s\n        expected to contain: %s\n        actual: %s"):format(
        message or "missing substring",
        tostring(needle),
        tostring(haystack)
      ),
      2
    )
  end
end

------------------------------------------------------------------------------
-- 4. 观测工具
------------------------------------------------------------------------------

--- 最近一次注册的 handler（按事件名直接调用 = 白盒驱动）。
function handler(name)
  return HANDLERS[name]
end

--- 调用一个 handler；未注册即失败（防止「测试悄悄空跑」）。
function fire(name, payload)
  local fn = HANDLERS[name]
  assert_true(fn ~= nil, ("handler not registered: %s"):format(name))
  return fn(payload)
end

function kv(key)
  return KV[key]
end

function set_kv(key, value)
  KV[key] = value
end

--- 清空收集面（每个用例开头调用，避免串味）。
function reset()
  NARRATED = {}
  EMITTED = {}
  __SELFTEST.lastChoice = nil
  return NARRATED
end

function narrated()
  return NARRATED
end

function emitted()
  return EMITTED
end

--- 叙述里是否出现过某个子串。
function narrated_contains(needle)
  for _, line in ipairs(NARRATED) do
    if line:find(needle, 1, true) then
      return true
    end
  end
  return false
end

--- 清空 KV 与收集面，然后重新开局（等价于 Rust 侧每个用例新建一个 Game）。
--- handlers 不动：它们是 mod 加载时注册的，跨用例复用。
function fresh()
  for key in pairs(KV) do
    KV[key] = nil
  end
  NARRATED = {}
  EMITTED = {}
  __SELFTEST.lastChoice = nil
  __SELFTEST.meta = {}
  fire("event.start", "0")
end

--- 读一个文件；失败即抛错（避免「读不到就静默通过」）。
function read_file(path)
  local handle = io.open(path, "r")
  assert_true(handle ~= nil, ("file must be readable: %s"):format(path))
  local text = handle:read("*a")
  handle:close()
  return text
end

--- 路径是否存在（文件或目录）。
function exists(path)
  local handle = io.open(path, "r")
  if handle then
    handle:close()
    return true
  end
  local probe = io.popen(('test -e "%s" && echo yes'):format(path))
  local out = probe:read("*l")
  probe:close()
  return out == "yes"
end
