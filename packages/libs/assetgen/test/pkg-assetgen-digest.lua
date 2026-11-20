-- `pkg.assetgen` 跨进程字节一致载体（契约 §7 的第三层证据）。
--
-- # 这个文件只做一件事
--
-- 打印**两行**：
--   `pkg.assetgen.lengths=<key=bytes,...>`   —— 每个产物的字节数
--   `pkg.assetgen.hex=<小写十六进制>`         —— 全部产物字节的拼接
--
-- 门禁把它跑两次（两个独立进程），两行各写文件后由**宿主**做 `cmp` 与
-- `xxd -r -p | shasum -a 256`。Lua 侧不实现 sha256：那会变成仓里第二份摘要（契约 §1）。
--
-- # 为什么是 hex 而不是 canon
--
-- canon 是**形状**的描述串（`k=v`），它证明不了字节；`hex` 就是产物字节本身，
-- 宿主的 sha256 算的是它的输入。两行一起给：lengths 让长度漂移一眼可见，
-- hex 让任何一位变化都现形。
--
-- 命名 ⇒ 计数可复算：`--filter pkg-assetgen` 同时命中本文件与 `pkg-assetgen.lua`。

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

local assetgen = require("pkg.assetgen")
assetgen.install()

--- 三类素材各一件：纹理 / 网格 / 音频，参数小到一眼能看完，大到跨过所有分支。
local ENTRIES = {
  {
    key = "tile",
    kind = "texture",
    algo = "texture.fbm",
    params = { w = 32, h = 32, ch = 4, cell = 8, octaves = 3 },
  },
  {
    key = "crate",
    kind = "mesh",
    algo = "mesh.box",
    params = { sx = 1000, sy = 600, sz = 400 },
  },
  {
    key = "shot",
    kind = "sound",
    algo = "sound.osc",
    params = { wave = "square", freq = 220, ms = 60, sr = 8000, amp = 900, env = "exp" },
  },
}
local SEED = "r1-assetgen-digest"

local function build()
  local bundle = assetgen.bundle(ENTRIES, SEED)
  local lengths = {}
  local hex = {}
  local total = 0
  for i = 1, #bundle.items do
    local item = bundle.items[i]
    local size = #item.product.bytes
    lengths[i] = item.key .. "=" .. size
    hex[i] = assetgen.encode(item.product, "hex")
    total = total + size
  end
  return {
    canon = bundle.canon,
    lengths = table.concat(lengths, ","),
    hex = table.concat(hex),
    total = total,
  }
end

local first = build()
local second = build()

print("pkg.assetgen.lengths=" .. first.lengths)
print("pkg.assetgen.hex=" .. first.hex)

run("pkg.assetgen.cross-process-canon", function()
  if first.canon ~= second.canon then
    error("同 seed 两次 bundle 的 canon 不等（长度 " .. #first.canon .. " vs " .. #second.canon .. "）", 0)
  end
  if first.hex ~= second.hex or first.lengths ~= second.lengths then
    error("同 seed 两次 bundle 的字节证据不等", 0)
  end
  if first.canon:find("\n", 1, true) ~= nil then
    error("canon 必须是单行（宿主按行取证据）", 0)
  end
  if first.lengths:find("%s") ~= nil then
    error("lengths 行不得含空白：" .. first.lengths, 0)
  end
  if first.lengths == "" or first.hex == "" then
    error("证据行不得为空（空值会让 cmp 两个空文件相等）", 0)
  end
  if #first.hex ~= first.total * 2 then
    error("hex 长度 " .. #first.hex .. " 必须等于 2×字节数之和 " .. (first.total * 2), 0)
  end
  if first.hex:find("^[0-9a-f]+$") == nil then
    error("hex 必须是小写十六进制", 0)
  end
  if #first.hex % 2 ~= 0 then
    error("hex 长度必须是偶数", 0)
  end
end)
