-- `pkg.assetgen.*` 自测。契约 `docs/design/packages/assetgen.md` §7。
--
-- # 为什么自带计数垫片
--
-- 包不依赖消费它的 mod（`packages/README.md` §5）：计数面直接挂 `__SELFTEST`。
-- 包**不碰宿主面**（不写 KV、不发事件、不注册 handler），所以不需要宿主 mock。
--
-- # 隔离口径
--
-- 每条用例 `fresh()`：清掉 `pkg.assetgen*` 全部 `package.loaded` 项再 require +
-- install，算法表与注入状态一起归零。用例之间不共享任何状态。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-assetgen.lua`，`--filter pkg-assetgen` 跑到它。

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
    error(
      ("%s: expected %s, got %s"):format(tostring(label or "eq"), tostring(expected), tostring(actual)),
      2
    )
  end
end

local function is_true(value, label)
  if not value then
    error((label or "is_true") .. ": expected truthy", 2)
  end
end

local function expect_error(code, body, label)
  local ok, err = pcall(body)
  if ok then
    error((label or "expect_error") .. ": expected " .. code .. ", but the call succeeded", 2)
  end
  if not tostring(err):find(code, 1, true) then
    error(("%s: expected %s, got %s"):format(label or "expect_error", code, tostring(err)), 2)
  end
end

--- 取一份**全新**的包实例（清缓存 = 算法表与 install 状态归零），并装配。
local function fresh()
  for name in pairs(package.loaded) do
    if type(name) == "string" and (name == "pkg.assetgen" or name:find("^pkg%.assetgen%.")) then
      package.loaded[name] = nil
    end
  end
  local assetgen = require("pkg.assetgen")
  assetgen.install()
  return assetgen
end

-- ── 静态面工具 ──────────────────────────────────────────────────────────────

--- 包的源文件清单：来自 manifest 的 packages 段（内容寻址那截，天然排除 `test/**`）。
local function pkg_sources(id)
  local manifest = rawget(_G, "__MANIFEST")
  if type(manifest) ~= "table" or type(manifest.packages) ~= "table" then
    error("manifest 缺 packages 段（载体 pack.json 未声明包或未重算 manifest）", 0)
  end
  for i = 1, #manifest.packages do
    local package = manifest.packages[i]
    if package.id == id then
      local out = {}
      for j = 1, #package.files do
        out[#out + 1] = package.files[j].path
      end
      return out
    end
  end
  error("manifest 的 packages 段没有包 " .. id, 0)
end

local function source_text(rel)
  local full = PKG_ROOT .. "/" .. rel
  local handle = assert(io.open(full, "rb"), "读不到 " .. full)
  local text = handle:read("a")
  handle:close()
  return text
end

--- 去掉行注释与**双引号字符串**后的源码。
---
--- 方法的局限一并声明（包内纪律）：不支持长括号注释 `--[[ ]]` 与单引号字符串——
--- 包源码禁用这两种写法，所以它们缺席不会让扫描漏判；真要写进来，本函数会把它们
--- 当普通字符留下（偏保守：宁可多报，不可漏报）。
local function strip(text)
  local out = {}
  local i = 1
  local n = #text
  while i <= n do
    local c = text:sub(i, i)
    if c == '"' then
      i = i + 1
      while i <= n do
        local ch = text:sub(i, i)
        if ch == "\\" then
          i = i + 2
        elseif ch == '"' then
          i = i + 1
          break
        else
          i = i + 1
        end
      end
    elseif c == "-" and text:sub(i + 1, i + 1) == "-" then
      while i <= n and text:sub(i, i) ~= "\n" do
        i = i + 1
      end
    else
      out[#out + 1] = c
      i = i + 1
    end
  end
  return table.concat(out)
end

-- ── 用例：静态面（零浮点 / 无标准库表 / 不复制 PRNG）──────────────────────────

run("pkg.assetgen.no-float-static", function()
  local paths = pkg_sources("assetgen")
  eq(#paths, 9, "manifest 里 assetgen 源文件数（加模块要同步抬这个数）")
  local forbidden = { "math.", "string.", "table.", "print(", "collectgarbage", "ygg.", ":float(", ".float(" }
  local problems = {}
  for i = 1, #paths do
    local text = strip(source_text(paths[i]))
    for j = 1, #forbidden do
      if text:find(forbidden[j], 1, true) ~= nil then
        problems[#problems + 1] = paths[i] .. " 含 " .. forbidden[j]
      end
    end
    if text:find("[^%w_]os%.") ~= nil then
      problems[#problems + 1] = paths[i] .. " 含 os."
    end
    if text:find("%d%.%d") ~= nil then
      problems[#problems + 1] = paths[i] .. " 含浮点字面量"
    end
    -- 单斜杠除法：`//` 合法，其余 `/` 都是浮点除法。
    for at = 1, #text do
      if text:sub(at, at) == "/" and text:sub(at - 1, at - 1) ~= "/" and text:sub(at + 1, at + 1) ~= "/" then
        problems[#problems + 1] = paths[i] .. " 含单斜杠除法（偏移 " .. at .. "）"
        break
      end
    end
  end
  is_true(#problems == 0, table.concat(problems, "; "))
end)

run("pkg.assetgen.rng-not-copied", function()
  local paths = pkg_sources("assetgen")
  local forbidden = {
    "xoshiro",
    "CBF29CE484222325",
    "100000001B3",
    "BF58476D1CE4E5B9",
    "94D049BB133111EB",
    "9E3779B9",
    "fnv64",
  }
  local problems = {}
  local injected = false
  for i = 1, #paths do
    local text = source_text(paths[i])
    for j = 1, #forbidden do
      if text:find(forbidden[j], 1, true) ~= nil then
        problems[#problems + 1] = paths[i] .. " 复制了 procgen 的常量 " .. forbidden[j]
      end
    end
    if text:find('require("pkg.procgen")', 1, true) ~= nil then
      injected = true
    end
  end
  is_true(injected, "init.lua 必须 require(\"pkg.procgen\")（共享 rng 契约单份）")
  is_true(#problems == 0, table.concat(problems, "; "))
end)

-- ── 用例：纹理（I1 同 seed 同字节 / 异 seed 不同 / 形状）──────────────────────

local FBM = { algo = "texture.fbm", params = { w = 24, h = 16, ch = 3, cell = 4, octaves = 3 } }

run("pkg.assetgen.texture-same-seed-same-bytes", function()
  local assetgen = fresh()
  local first = assetgen.texture(FBM, "tex-seed")
  local second = assetgen.texture(FBM, "tex-seed")
  eq(#first.bytes, #second.bytes, "同 seed 两次的长度")
  is_true(first.bytes == second.bytes, "同 seed 两次的字节必须逐字节相等")
  eq(first.canon, second.canon, "同 seed 两次的 canon")
  eq(first.canon, assetgen.canon(first), "canon(product) 与 product.canon")
end)

run("pkg.assetgen.texture-diff-seed-diff-bytes", function()
  local assetgen = fresh()
  local a = assetgen.texture(FBM, "tex-seed-a")
  local b = assetgen.texture(FBM, "tex-seed-b")
  is_true(a.bytes ~= b.bytes, "异 seed 的字节必须不同")
  eq(#a.bytes, #b.bytes, "异 seed 的长度必须相同（防「长度不同也算通过」的假阳性）")
end)

run("pkg.assetgen.texture-size-and-shape", function()
  local assetgen = fresh()
  local product = assetgen.texture(FBM, "tex-shape")
  eq(product.w, 24, "w")
  eq(product.h, 16, "h")
  eq(product.ch, 3, "ch")
  eq(#product.bytes, 24 * 16 * 3, "#bytes = w*h*ch")
  eq(math.type(product.w), "integer", "w 必须是 integer")
  eq(math.type(product.h), "integer", "h 必须是 integer")
  eq(math.type(product.ch), "integer", "ch 必须是 integer")
  local low, high = 255, 0
  for i = 1, #product.bytes do
    local v = product.bytes:byte(i)
    if v < low then
      low = v
    end
    if v > high then
      high = v
    end
  end
  -- fbm 必须是非退化场：全黑或全白说明格点/插值写错了。
  is_true(low < 64, "fbm 的最小值应 < 64，实得 " .. low)
  is_true(high > 192, "fbm 的最大值应 > 192，实得 " .. high)
end)
