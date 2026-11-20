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

-- ── 跨语言向量：与 Tier 1（crates/ygg-sim）共用同一份夹具 ─────────────────────
--
-- 一个向量、两个消费者：Rust 侧 `tests/rng_golden.rs` 与这里各断言一次，任一侧漂移即红。
-- 夹具由**第三实现**（Python）生成，不是「Rust 算出来再让 Lua 抄一遍」——后者只能证明
-- 两边都错得一样。
--
-- 路径解析 fail-closed：读不到就 error，不静默跳过（跳过会让「夹具改名」变成永久静默）。
-- 仓根从宿主注入的 `PKG_ROOT`（包根绝对路径）向上找第一个带 `.git` 的目录——与
-- `examples/selftest.rs` 的 `ancestor_with_git` 同一个锚点，不自己发明相对深度公式。
--
-- 只用 base 函数（`io` / `tonumber` / `error`），不用 `string` / `math`：本文件在
-- 普通 Lua 与沙箱两种宿主下都跑得动，多用一个库就多一条「哪个宿主下能跑」的分支。
-- 十六进制向量用 `tonumber(s)` 转成整数比对（Lua 5.4 的整数字面量与 FNV 的 i64
-- 按位回绕同一个语义，故 ≥2^63 的值两侧同样变负，比对仍然成立）。

local function exists(path)
  local probe = io.open(path, "r")
  if probe == nil then
    return false
  end
  probe:close()
  return true
end

local function repo_root()
  local start = rawget(_G, "PKG_ROOT")
  if type(start) ~= "string" then
    error("宿主未注入 PKG_ROOT（本用例必须在 examples/selftest 下跑）", 0)
  end
  -- 向上找第一个同时有 `Cargo.toml` 与 `.git` 的目录（仓根的两个独立锚点：前者是构建
  -- 身份，后者是版本控制身份；只用一个会把「子目录里恰好有个 Cargo.toml」判成仓根）。
  -- 路径可能是相对的（宿主默认传 `packages/libs`），走到最后一段时用 `.` 表示仓根——
  -- 与 assetgen 的 `PKG_ROOT .. "/" .. rel` 同一条假设：宿主在仓根下运行。
  local dir = start
  for _ = 1, 16 do
    if exists(dir .. "/Cargo.toml") and exists(dir .. "/.git") then
      return dir
    end
    if dir == "." or dir == "/" then
      break
    end
    local parent = dir:match("^(.*)/[^/]+$")
    if parent == nil then
      -- 相对路径走到「无斜杠」这一段（如 `packages`）：上一级就是宿主的 cwd，用 `.` 表示。
      dir = "."
    elseif parent == "" then
      dir = "/"
    else
      dir = parent
    end
  end
  error("从 PKG_ROOT 向上找不到含 Cargo.toml 与 .git 的仓根：" .. start, 0)
end

local function read_fixture()
  local path = repo_root() .. "/crates/ygg-sim/fixtures/fnv64-vectors.json"
  local file = io.open(path, "rb")
  if file == nil then
    error("读不到跨语言向量夹具：" .. path, 0)
  end
  local text = file:read("a")
  file:close()
  local ok, doc = pcall(codec.decode, text)
  if not ok then
    error("向量夹具不是合法 JSON：" .. tostring(doc), 0)
  end
  return doc
end

local rng_mod = require("pkg.procgen.rng")

run("pkg.procgen.rng.fnv64-cross-language-vectors", function()
  local doc = read_fixture()
  if type(doc.fnv64) ~= "table" or #doc.fnv64 == 0 then
    error("夹具缺 fnv64 数组", 0)
  end
  for _, entry in ipairs(doc.fnv64) do
    local got = rng_mod.fnv64(entry.text)
    local want = tonumber(entry.fnv64)
    if want == nil then
      error("向量里的 fnv64 不是可解析的十六进制：" .. tostring(entry.fnv64), 0)
    end
    if got ~= want then
      error("fnv64(" .. entry.text .. ") 与共享向量不符（Lua=" .. got .. " 向量=" .. want .. "）", 0)
    end
  end
end)

run("pkg.procgen.rng.derive-state-cross-language-vectors", function()
  local doc = read_fixture()
  if type(doc.streams) ~= "table" or #doc.streams == 0 then
    error("夹具缺 streams 数组", 0)
  end
  for _, entry in ipairs(doc.streams) do
    local label = entry.seed .. "/" .. entry.stream
    -- 状态必须与向量一致（derive_state 的直接证据）
    local state = procgen.state(procgen.rng(entry.seed, entry.stream))
    for i = 1, 4 do
      if state[i] ~= entry.state[i] then
        error(label .. " state[" .. i .. "] = " .. tostring(state[i]) .. "，期望 " .. tostring(entry.state[i]), 0)
      end
    end
    -- 前 8 抽也必须一致（next32 的位级证据）
    local fresh = procgen.rng(entry.seed, entry.stream)
    for i = 1, 8 do
      local got = procgen.next32(fresh)
      if got ~= entry.first8[i] then
        error(label .. " first8[" .. i .. "] = " .. tostring(got) .. "，期望 " .. tostring(entry.first8[i]), 0)
      end
    end
  end
end)
