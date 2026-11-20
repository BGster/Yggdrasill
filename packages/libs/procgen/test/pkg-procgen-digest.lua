-- `pkg.procgen` 跨进程字节一致载体（契约 §7 的第三层证据）。
--
-- # 这个文件只做一件事
--
-- 打印**唯一一行** `pkg.procgen.canon=<codec.encode 产物>`。门禁把它跑两次（两个
-- 独立进程），把两行各写文件后由**宿主**做 `cmp` 与 `shasum -a 256`——Lua 侧不实现
-- sha256，那会变成仓里第二份摘要实现（契约 §1）。
--
-- # 为什么 canon 是 encode 的字符串而不是 hash
--
-- `hash` 是便捷面（FNV-1a 64，用于变更检测）；「同 seed 同产物**字节**一致」的
-- 判据必须是产物本身的字节。所以 canon 用 `codec.encode`（唯一那份确定性 JSON
-- 口径）的完整字符串：宿主的 sha256 算的是 16 进制哈希的**输入**，不是它的输出。
--
-- 命名 ⇒ 计数可复算：文件名 `pkg-procgen-digest.lua`，`--filter pkg-procgen-digest`
-- 只跑本文件（夹具侧同名冒烟接住 mod 侧过滤）。

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

--- canon 的生成器：参数固定、产物 32×32（小到能一眼看完，大到跨过所有分支）。
local NAME = "canon"
local SEED = "r3-canon-seed"

local function canon_of(procgen, codec)
  procgen.define(NAME, {
    algo = "terrain.noise",
    params = { w = 32, h = 32, cell = 8, octaves = 3 },
  })
  return codec.encode(procgen.run(NAME, SEED))
end

local procgen = require("pkg.procgen")
local codec = require("pkg.codec")
procgen.install()

local first = canon_of(procgen, codec)
local second = canon_of(procgen, codec)

print("pkg.procgen.canon=" .. first)

run("pkg.procgen.same-seed-same-canon", function()
  if first ~= second then
    error("同 seed 两次 encode 字符串不等（长度 " .. #first .. " vs " .. #second .. "）", 0)
  end
  if first:find("\n", 1, true) ~= nil then
    error("canon 必须是单行（宿主按行取证据）", 0)
  end
  if procgen.hash(procgen.run(NAME, SEED)) ~= procgen.hash(procgen.run(NAME, SEED)) then
    error("同 seed 两次 hash 不等", 0)
  end
end)
