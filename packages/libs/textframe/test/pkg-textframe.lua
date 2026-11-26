-- `pkg.textframe.*` 自测（一）：网格与行装配 + 错误面 + 导出面。
-- 契约 `openwiki/contracts/packages/textframe.md` §2 / §3 / §7 的用例 1–7、14–15。
--
-- # 为什么自带计数垫片
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）：这里就地取 `__SELFTEST` 计数面，
-- 没有就直接建一个——包自测因此既能被 `--mod packages/mods/pkg-selftest` 收集，也能被
-- 任何声明了本包的 Mod 收集，两处计数口径同源。每个测试文件都自带一份（文件是独立
-- 执行的单元，共享模块会把「一个文件跑不动」变成整套跑不动）。
--
-- # 命名 ⇒ 计数可复算
--
-- 文件名以 `pkg-textframe` 开头，`--filter pkg-textframe` 同时命中本文件的三个兄弟
-- （`-compose` / `-perf`）；三者 `cases passed=` 之和 = 契约 §7 的 18。

local S = rawget(_G, "__SELFTEST")
if type(S) ~= "table" then
  S = { pass = 0, fail = 0 }
  _G.__SELFTEST = S
end

local function run(name, body)
  local ok, err = pcall(body)
  if ok then
    S.pass = S.pass + 1
  else
    S.fail = S.fail + 1
    print("FAIL " .. name .. ": " .. tostring(err))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error(("%s: expected %s, got %s"):format(tostring(label or "eq"), tostring(expected), tostring(actual)), 2)
  end
end

local function eq_seq(actual, expected, label)
  eq(#actual, #expected, (label or "eq_seq") .. " 长度")
  for i = 1, #expected do
    eq(actual[i], expected[i], (label or "eq_seq") .. "[" .. i .. "]")
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy", 2)
  end
end

--- 断言某次调用抛出指定错误码（子串匹配：码后面总跟着 `: 细节`）。
local function err_code(code, fn, label)
  local ok, err = pcall(fn)
  if ok then
    error((label or "err_code") .. ": 应当报 " .. code .. "，但没有抛错", 2)
  end
  if tostring(err):find(code, 1, true) == nil then
    error((label or "err_code") .. ": 期望码 " .. code .. "，实得 " .. tostring(err), 2)
  end
end

local function fresh()
  for name in pairs(package.loaded) do
    if type(name) == "string" and (name == "pkg.textframe" or name:find("^pkg%.textframe%.")) then
      package.loaded[name] = nil
    end
  end
  local module = require("pkg.textframe")
  module.install()
  return module
end

local function sorted_keys(map)
  local keys = {}
  for key in pairs(map) do
    keys[#keys + 1] = tostring(key)
  end
  table.sort(keys)
  return table.concat(keys, ",")
end

local tf = fresh()
local E = tf.ERRORS

-- ── 形状与单格写 ──────────────────────────────────────────────────────────

run("pkg.textframe.frame-blank", function()
  local blank = tf.frame(4, 3)
  eq(blank.w, 4, "w")
  eq(blank.h, 3, "h")
  is_true(type(blank.cells) == "table", "cells 必须是表")
  eq(#blank.cells, 0, "新帧不该预填任何格子（nil 就是空白）")
  eq_seq(tf.to_lines(blank), { "    ", "    ", "    " }, "空白帧 3 行 × 4 空格")
end)

run("pkg.textframe.put-index-1based", function()
  local f = tf.frame(4, 3)
  tf.put(f, 1, 1, "a")
  eq(f.cells[1], "a", "(1,1) → cells[1]")
  tf.put(f, 4, 3, "z")
  eq(f.cells[12], "z", "(4,3) → cells[(3-1)*4+4] = cells[12]")
  eq(f.cells[2], nil, "同格之外仍是 nil")
  eq(tf.put(f, 2, 2, "m"), f, "put 返回 dst 本身（链式）")
end)

run("pkg.textframe.put-clear", function()
  local f = tf.frame(2, 1)
  tf.put(f, 1, 1, "a")
  eq(tf.to_lines(f)[1], "a ", "写后有字")
  tf.put(f, 1, 1, nil)
  eq(f.cells[1], nil, "nil = 清格（回到空白态）")
  eq(tf.to_lines(f)[1], "  ", "清格后回到空格")
end)

run("pkg.textframe.put-range", function()
  local f = tf.frame(3, 2)
  err_code(E.RANGE, function() tf.put(f, 0, 1, "a") end, "x=0")
  err_code(E.RANGE, function() tf.put(f, 4, 1, "a") end, "x=w+1")
  err_code(E.RANGE, function() tf.put(f, 1, 0, "a") end, "y=0")
  err_code(E.RANGE, function() tf.put(f, 1, 3, "a") end, "y=h+1")
  tf.put(f, 3, 2, "a")
  eq(f.cells[6], "a", "边界内的最后格可写")
end)

run("pkg.textframe.put-cell", function()
  local f = tf.frame(2, 1)
  err_code(E.CELL, function() tf.put(f, 1, 1, "") end, "空串")
  err_code(E.CELL, function() tf.put(f, 1, 1, "ab") end, "两字符")
  err_code(E.CELL, function() tf.put(f, 1, 1, "\228\189\160") end, "多字节（你）")
  err_code(E.CELL, function() tf.put(f, 1, 1, 5) end, "非字符串")
  tf.put(f, 1, 1, " ")
  eq(f.cells[1], " ", "空格是合法单元格（实字，不是空白）")
end)

-- ── 行装配 ────────────────────────────────────────────────────────────────

run("pkg.textframe.to-lines-width", function()
  local f = tf.frame(8, 3)
  for y = 1, 3 do
    for x = 1, 8 do
      tf.put(f, x, y, string.char(64 + (y - 1) * 8 + x))
    end
  end
  local lines = tf.to_lines(f)
  eq_seq(lines, { "ABCDEFGH", "IJKLMNOP", "QRSTUVWX" }, "行序 = 行主序")
  for i = 1, #lines do
    eq(#lines[i], 8, "第 " .. i .. " 行宽度")
  end
end)

run("pkg.textframe.to-lines-blank", function()
  local f = tf.frame(3, 1)
  f.cells[2] = ""
  eq(tf.to_lines(f)[1], "   ", "nil 与 \"\" 都渲染成空格")
  -- 多字节格必然让 #line ≠ w：I6 的读侧防线（断言在行上，不在格上）
  local bad = tf.frame(2, 1)
  bad.cells[1] = "\228\189\160"
  err_code(E.CELL, function() tf.to_lines(bad) end, "多字节格破坏行宽")
end)

-- ── 错误面与导出面 ────────────────────────────────────────────────────────

run("pkg.textframe.shape-errors", function()
  err_code(E.PARAM, function() tf.frame(0, 1) end, "w=0")
  err_code(E.PARAM, function() tf.frame(1.5, 1) end, "w=1.5")
  err_code(E.PARAM, function() tf.frame(1, "3") end, "h 是字符串")
  err_code(E.SHAPE, function() tf.to_lines(42) end, "to_lines 收非表")
  err_code(E.SHAPE, function() tf.to_lines({ w = 2, h = 1, cells = "x" }) end, "cells 非表")
  err_code(E.SHAPE, function() tf.stack(42) end, "stack 收非表")
  err_code(E.SHAPE, function() tf.stack({ { "a" }, { 1 } }) end, "块里有非字符串行")
  eq(#tf.to_lines({ w = 2, h = 1 }), 1, "cells 缺省 = 空表（§2 缺省列）")
end)

run("pkg.textframe.exports-shape", function()
  eq(sorted_keys(tf), "ERRORS,VERSION,blit,frame,install,perf_keys,put,stack,to_lines", "导出面")
  eq(sorted_keys(E), "CELL,PARAM,RANGE,SHAPE", "错误码面")
  eq(#tf.perf_keys(), 9, "perf_keys 九键")
end)

return S
