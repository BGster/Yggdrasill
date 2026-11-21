-- `pkg.textframe.*` 自测（二）：分层 / 块拼接 / 与 r4 适配器的对照 / 纯度。
-- 契约 `docs/design/packages/textframe.md` §3 I1–I4、§7 的用例 8–13、16–17。
--
-- # 参照实现 `oracle_compose` 是**逐字复刻**，不是从被测代码 import
--
-- 它照抄 Saltline `src/view/frame.lua` 的 `compose`（两两合并 + HUD 行追加）——
-- 那个适配器是 S6 之前的事实标准，本包的第一个验收就是「换上它之后 30 行一个字节
-- 都不变」。把 oracle 写成 `tf.to_lines` 的包装会让这条用例变成自证。

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

--- 两两合并（r4 适配器用的就是 `pkg.assetgen.join` 的这份算法，逐字复刻）。
local function merge_concat(parts)
  local n = #parts
  if n == 0 then
    return ""
  end
  local width = 1
  while width < n do
    local index = 1
    while index <= n do
      local right = parts[index + width]
      if right ~= nil then
        parts[index] = parts[index] .. right
      end
      index = index + width * 2
    end
    width = width * 2
  end
  return parts[1]
end

--- Saltline `src/view/frame.lua` 的 `compose`，逐字复刻。
local function oracle_compose(buf, hud_rows)
  local lines = {}
  local w = buf.w
  for row = 1, buf.h do
    local parts = {}
    local base = (row - 1) * w
    for col = 1, w do
      parts[col] = buf.cells[base + col] or " "
    end
    lines[row] = merge_concat(parts)
  end
  for i = 1, #hud_rows do
    lines[buf.h + i] = hud_rows[i]
  end
  return lines
end

local tf = fresh()

-- ── 分层 ──────────────────────────────────────────────────────────────────

run("pkg.textframe.blit-overwrite", function()
  local dst = tf.frame(4, 2)
  tf.put(dst, 1, 1, "A")
  local src = tf.frame(2, 1)
  tf.put(src, 1, 1, "X")
  tf.put(src, 2, 1, "Y")
  eq(tf.blit(dst, src, 1, 1), dst, "blit 返回 dst 本身")
  eq_seq(tf.to_lines(dst), { "XY  ", "    " }, "后写胜")
end)

run("pkg.textframe.blit-transparent", function()
  local dst = tf.frame(3, 1)
  for x = 1, 3 do
    tf.put(dst, x, 1, "a")
  end
  local src = tf.frame(3, 1)
  tf.put(src, 1, 1, nil) -- 显式 nil
  src.cells[2] = "" -- 外部数据里的空串（读侧宽容；put 会拒）
  tf.put(src, 3, 1, "Z")
  tf.blit(dst, src, 1, 1)
  eq(tf.to_lines(dst)[1], "aaZ", "nil 与 \"\" 透明，只有 Z 覆盖")

  -- 正向：`" "` 是实字空格，会覆盖（与上面两个透明态相反）
  local space = tf.frame(1, 1)
  tf.put(space, 1, 1, " ")
  tf.blit(dst, space, 1, 1)
  eq(tf.to_lines(dst)[1], " aZ", "\" \" 覆盖了第 1 格的 a")
end)

run("pkg.textframe.blit-clip", function()
  local dst = tf.frame(3, 3)
  for y = 1, 3 do
    for x = 1, 3 do
      tf.put(dst, x, y, "a")
    end
  end
  local src = tf.frame(3, 3)
  for y = 1, 3 do
    for x = 1, 3 do
      tf.put(src, x, y, "Z")
    end
  end
  tf.blit(dst, src, -1, -1)
  eq(tf.to_lines(dst)[1], "Zaa", "负偏移左上角：只有 (1,1) 落进来")
  tf.blit(dst, src, 3, 3)
  eq(tf.to_lines(dst)[3], "aaZ", "右下角越界：只有 (3,3) 落进来")
  local before = tf.to_lines(dst)
  tf.blit(dst, src, 100, 100)
  eq_seq(tf.to_lines(dst), before, "完全在外：一格不动，也不抛")
end)

run("pkg.textframe.blit-chain", function()
  local dst = tf.frame(3, 3)
  for y = 1, 3 do
    for x = 1, 3 do
      tf.put(dst, x, y, ".")
    end
  end
  local function stamp(ch)
    local layer = tf.frame(3, 3)
    tf.put(layer, 2, 2, ch)
    return layer
  end
  tf.blit(dst, stamp("i"), 1, 1) -- item
  tf.blit(dst, stamp("m"), 1, 1) -- monster
  tf.blit(dst, stamp("@"), 1, 1) -- player
  eq_seq(tf.to_lines(dst), { "...", ".@.", "..." }, "优先级链 = 后写胜（terrain < item < mon < player）")
end)

-- ── 块拼接 ────────────────────────────────────────────────────────────────

run("pkg.textframe.stack-order", function()
  local out = tf.stack({ { "a", "b" }, nil, { "c" }, { "d" } })
  eq_seq(out, { "a", "b", "c", "d" }, "块序 = 行序，nil 块跳过")
end)

run("pkg.textframe.stack-empty", function()
  eq(#tf.stack({}), 0, "空 blocks")
  eq(#tf.stack({ nil, nil }), 0, "全 nil 块")
  is_true(type(tf.stack({})) == "table", "永远返回表，不返回 nil")
end)

-- ── 对照基准：r4 适配器的事实形状 ──────────────────────────────────────────

run("pkg.textframe.oracle-saltline-frame", function()
  local w, h = 80, 24
  local glyphs = ".:-=+*#oO0@%"
  local buf = tf.frame(w, h)
  for i = 1, w * h do
    -- 每 7 格留一个空洞：逼出 nil → 空格那条路（也让 " " 与 nil 不可混同）
    if i % 7 ~= 0 then
      local ch = glyphs:sub((i % 11) + 1, (i % 11) + 1)
      tf.put(buf, (i - 1) % w + 1, (i - 1) // w + 1, ch)
    end
  end
  local hud = { "生命 88  弹匣 12/30", "目标 继电器 2/3", "雨 300", "风向 西北", "字幕：保持安静", "进度 60%" }
  local got = tf.stack({ tf.to_lines(buf), hud })
  local want = oracle_compose(buf, hud)
  eq(#got, 30, "24 行世界区 + 6 行 HUD")
  eq(#got[1], 80, "世界区行宽 = w")
  eq_seq(got, want, "与 r4 适配器逐字相同")
end)

-- ── 纯度 / 确定性 ─────────────────────────────────────────────────────────

run("pkg.textframe.purity-determinism", function()
  local function global_keys()
    local keys = {}
    for key in pairs(_G) do
      keys[#keys + 1] = tostring(key)
    end
    table.sort(keys)
    return table.concat(keys, ",")
  end
  local before = global_keys()
  local buf = tf.frame(12, 4)
  for i = 1, 40 do
    tf.put(buf, (i - 1) % 12 + 1, (i - 1) // 12 + 1, "x")
  end
  local first = table.concat(tf.stack({ tf.to_lines(buf), { "HUD" } }), "\n")
  for round = 1, 100 do
    local again = table.concat(tf.stack({ tf.to_lines(buf), { "HUD" } }), "\n")
    if again ~= first then
      error("第 " .. round .. " 轮输出与第 1 轮不同（I2 确定性）", 2)
    end
  end
  eq(global_keys(), before, "100 轮 compose 不新增全局（I1 纯）")
end)

return S
