-- 微 harness（~30 行）：**只**给 `test` / `assert_eq` / `assert_true` /
-- `__SELFTEST` 与一个最小 `ygg` mock。
--
-- 刻意不复制 agent-seed 那份 400 行 harness：本夹具的被测面是**装载契约**
-- （包文件能不能解析、能不能 require、导出能不能调），不是 agent-seed 的
-- 读签台账 / 场景路由。抄一份会把两套无关的依赖绑在一起。

__SELFTEST = { pass = 0, fail = 0 }

local function record(ok, name, detail)
  if ok then
    __SELFTEST.pass = __SELFTEST.pass + 1
  else
    __SELFTEST.fail = __SELFTEST.fail + 1
    print("FAIL " .. tostring(name) .. ": " .. tostring(detail))
  end
end

function test(name, fn)
  local ok, err = pcall(fn)
  record(ok, name, err)
end

function assert_true(cond, name)
  record(cond and true or false, name, "期望为真")
end

function assert_eq(got, want, name)
  record(got == want, name, "got=" .. tostring(got) .. " want=" .. tostring(want))
end

-- `ygg` mock：只给本夹具用到的三个面。
local registered = {}
ygg = {
  on = function(name, fn)
    registered[name] = fn
  end,
  narrate = function(line)
    print("narrate " .. tostring(line))
  end,
  data = function()
    return {}
  end,
}
__HANDLERS = registered
