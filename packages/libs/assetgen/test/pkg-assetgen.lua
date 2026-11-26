-- `pkg.assetgen.*` 自测。契约 `openwiki/contracts/packages/assetgen.md` §7。
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

local function samples_of(product)
  local out = {}
  local pcm = product.pcm
  local stride = product.ch * 2
  for j = 0, product.frames - 1 do
    local lo = pcm:byte(j * stride + 1)
    local hi = pcm:byte(j * stride + 2)
    local v = lo | (hi << 8)
    if v >= 32768 then
      v = v - 65536
    end
    out[j + 1] = v
  end
  return out
end

local OSC = { algo = "sound.osc", params = { wave = "sine", freq = 100, ms = 50, sr = 8000, amp = 800 } }

local BUNDLE_ENTRIES = {
  { key = "grass", kind = "texture", algo = "texture.checker", params = { w = 8, h = 8, ch = 1, size = 2 } },
  { key = "crate", kind = "mesh", algo = "mesh.box", params = { sx = 600, sy = 600, sz = 600 } },
  { key = "blip", kind = "sound", algo = "sound.osc", params = { wave = "square", freq = 300, ms = 20, sr = 8000 } },
}


-- === S-25 批十三测试件连拆装配：用例段搬入兄弟件（test/assetgen/<theme>.lua），执行次序 = 装配次序 ===
local C = {
  S = S,
  run = run,
  eq = eq,
  is_true = is_true,
  expect_error = expect_error,
  fresh = fresh,
  pkg_sources = pkg_sources,
  source_text = source_text,
  strip = strip,
  samples_of = samples_of,
  OSC = OSC,
  BUNDLE_ENTRIES = BUNDLE_ENTRIES
}

dofile(PKG_ROOT .. "/assetgen/test/pkg-assetgen/texture.lua")(C)
dofile(PKG_ROOT .. "/assetgen/test/pkg-assetgen/mesh-raster.lua")(C)
dofile(PKG_ROOT .. "/assetgen/test/pkg-assetgen/sound.lua")(C)
dofile(PKG_ROOT .. "/assetgen/test/pkg-assetgen/bundle-perf.lua")(C)
