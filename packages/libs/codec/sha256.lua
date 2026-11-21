-- `pkg.codec.sha256` —— 纯 Lua 5.4 的 SHA-256（字节串 → 64 位小写 hex）。
--
-- S19 起本文件是**规范执行体 + 缺面回退体**：宿主 `ygg.codec.sha256` 在场时
-- `runtime.M.sha256` 按键独立地委托原生（`face.sha256` 为 function 即用）；
-- 本文件仍是字节产出的规范与差分门禁的执行体，地位与 encode/decode 的 Lua
-- 体相同（契约 §4.1 下沉边界表）——旧宿主只有 encode/decode 两键或裸沙箱
-- 无面时，摘要仍由这里产出，逐字节不变。
--
-- 契约：`docs/design/packages/codec.md` §4「checksum 的 sha256 来源」（D11 已回写：
-- P0 的「FNV-1a 降级路径」退役——纯 Lua sha256 就位 + P1 挂载桥已落地，宿主
-- `fs.write` 的 sha256 由「来源」变「互校」）。
--
-- # 为什么自己写而不是调宿主
--
-- 宿主没有 `ygg.json.*`，也没有 `ygg.digest`。`ygg.fs.write` 会回一个 sha256，但
-- 那是**落盘之后**的事实：`checksum(bundle)` 必须在**不落盘**时就能算（导出侧要
-- 把校验和写进 bundle 本体），而且「导入前先校验」要求能在纯读路径上复核。
-- 依赖宿主回值就无法校验「文件在盘上被改过」——那正是 checksum 存在的理由。
--
-- # 方言约束（沙箱）
--
-- 沙箱没有 `string` / `table` 全局表（`crates/ygg-sandbox/src/sandbox.rs` 白名单
-- 测试钉死），但**字符串方法语法**可用（走字符串元表的 `__index`）。本文件因此
-- 只用 `s:byte(i, j)`，不用任何标准库表。

local M = {}

local MASK = 0xFFFFFFFF

-- SHA-256 的 64 个轮常量（FIPS 180-4 §4.2.2）。
local K = {
  0x428a2f98,
  0x71374491,
  0xb5c0fbcf,
  0xe9b5dba5,
  0x3956c25b,
  0x59f111f1,
  0x923f82a4,
  0xab1c5ed5,
  0xd807aa98,
  0x12835b01,
  0x243185be,
  0x550c7dc3,
  0x72be5d74,
  0x80deb1fe,
  0x9bdc06a7,
  0xc19bf174,
  0xe49b69c1,
  0xefbe4786,
  0x0fc19dc6,
  0x240ca1cc,
  0x2de92c6f,
  0x4a7484aa,
  0x5cb0a9dc,
  0x76f988da,
  0x983e5152,
  0xa831c66d,
  0xb00327c8,
  0xbf597fc7,
  0xc6e00bf3,
  0xd5a79147,
  0x06ca6351,
  0x14292967,
  0x27b70a85,
  0x2e1b2138,
  0x4d2c6dfc,
  0x53380d13,
  0x650a7354,
  0x766a0abb,
  0x81c2c92e,
  0x92722c85,
  0xa2bfe8a1,
  0xa81a664b,
  0xc24b8b70,
  0xc76c51a3,
  0xd192e819,
  0xd6990624,
  0xf40e3585,
  0x106aa070,
  0x19a4c116,
  0x1e376c08,
  0x2748774c,
  0x34b0bcb5,
  0x391c0cb3,
  0x4ed8aa4a,
  0x5b9cca4f,
  0x682e6ff3,
  0x748f82ee,
  0x78a5636f,
  0x84c87814,
  0x8cc70208,
  0x90befffa,
  0xa4506ceb,
  0xbef9a3f7,
  0xc67178f2,
}

-- 初始哈希（FIPS 180-4 §5.3.3）：8 个 32 位字，大端拼接即得摘要。
local INIT = {
  0x6a09e667,
  0xbb67ae85,
  0x3c6ef372,
  0xa54ff53a,
  0x510e527f,
  0x9b05688c,
  0x1f83d9ab,
  0x5be0cd19,
}

--- 32 位循环右移。
local function rrot(x, n)
  return ((x >> n) | (x << (32 - n))) & MASK
end

--- 压缩一个 64 字节块（`s` 的 `offset+1 .. offset+64` 字节）到 `h`。
local function compress(h, s, offset)
  local w = {}
  for i = 1, 16 do
    local at = offset + (i - 1) * 4 + 1
    local b1, b2, b3, b4 = s:byte(at, at + 3)
    w[i] = ((b1 << 24) | (b2 << 16) | (b3 << 8) | b4) & MASK
  end
  for i = 17, 64 do
    local x = w[i - 15]
    local y = w[i - 2]
    local s0 = rrot(x, 7) ~ rrot(x, 18) ~ (x >> 3)
    local s1 = rrot(y, 17) ~ rrot(y, 19) ~ (y >> 10)
    w[i] = (w[i - 16] + s0 + w[i - 7] + s1) & MASK
  end

  local a, b, c, d, e, f, g, hh = h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8]
  for i = 1, 64 do
    local S1 = rrot(e, 6) ~ rrot(e, 11) ~ rrot(e, 25)
    local ch = (e & f) ~ ((~e) & g)
    local temp1 = (hh + S1 + ch + K[i] + w[i]) & MASK
    local S0 = rrot(a, 2) ~ rrot(a, 13) ~ rrot(a, 22)
    local maj = (a & b) ~ (a & c) ~ (b & c)
    local temp2 = (S0 + maj) & MASK
    hh = g
    g = f
    f = e
    e = (d + temp1) & MASK
    d = c
    c = b
    b = a
    a = (temp1 + temp2) & MASK
  end

  h[1] = (h[1] + a) & MASK
  h[2] = (h[2] + b) & MASK
  h[3] = (h[3] + c) & MASK
  h[4] = (h[4] + d) & MASK
  h[5] = (h[5] + e) & MASK
  h[6] = (h[6] + f) & MASK
  h[7] = (h[7] + g) & MASK
  h[8] = (h[8] + hh) & MASK
end

--- 消息长度（bit）的大端 8 字节尾。
---
--- 字节用 `("%c"):format(n)` 构造（标准库的 char 面在沙箱不可达；`%c` 同属
--- 字符串方法面）。注意**不能**用 `"\\%03d"`：那产出的是「反斜杠 + 三位数字」
--- 四个字节，不是转义后的单字节（本实现的第一版就是这么错的，空串向量当场红）。
local function length_tail(bit_len)
  local bytes = {}
  local rest = bit_len
  for i = 8, 1, -1 do
    bytes[i] = rest & 0xFF
    rest = rest >> 8
  end
  local out = ""
  for i = 1, 8 do
    out = out .. ("%c"):format(bytes[i])
  end
  return out
end

--- `sha256(bytes)`：字节串 → 64 位小写十六进制。
---
--- 「字节串」是契约里唯一合法的输入：由 `encode` 产出的 ASCII/UTF-8 文本。非字符串
--- 输入报错而不是隐式转字符串——`tostring(table)` 得到 `"table: 0x..."`（含地址），
--- 那会把确定性判据变成一个陷阱。
function M.sha256(bytes)
  if type(bytes) ~= "string" then
    error("YGG-PKG-CODEC-TYPE: sha256 只收字符串（得到 " .. type(bytes) .. "）", 0)
  end
  local bit_len = #bytes * 8
  -- 填充：`0x80` + `0x00` × pad + 8 字节大端 bit 长度，使总长为 64 的倍数。
  local pad = (56 - (#bytes + 1) % 64) % 64
  local message = bytes .. "\128" .. ("\0"):rep(pad) .. length_tail(bit_len)

  local h = { INIT[1], INIT[2], INIT[3], INIT[4], INIT[5], INIT[6], INIT[7], INIT[8] }
  local blocks = #message / 64
  for index = 0, blocks - 1 do
    compress(h, message, index * 64)
  end

  local out = {}
  for i = 1, 8 do
    out[i] = ("%08x"):format(h[i])
  end
  local hex = ""
  for i = 1, 8 do
    hex = hex .. out[i]
  end
  return hex
end

return M
