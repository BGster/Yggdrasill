-- `packages/libs/assetgen/encode.lua` —— 容器装箱（WAV16 / PCM16 / BMP24 / OBJ / hex / raw）。
--
-- 契约：`openwiki/contracts/packages/assetgen.md` §2 / §4（`assetgen.encode` 的容器表）。
--
-- # 为什么手写而不是复用 `codec`
--
-- JSON 文本不是素材容器：`codec` 的编码口径是为**可读存档**设计的（键序、转义），
-- 二进制进它会走控制字符转义（每字节最多六个字符），高字节直通又依赖解码侧容忍度。
-- 44 字节 WAV 头与 54 字节 BMP 头写几十行纯 Lua 就够，还顺带拿到「字节即证据」。
--
-- # 三条会被静默弄错的细节
--
-- 1. BMP 是**自底向上**存储（高度字段为正数），并且每行补到 4 的倍数。
-- 2. WAV 的 `byteRate` 是 `sr*ch*2` 而不是 `sr*ch`（少乘 2 只是时长听着不对）。
-- 3. 拼装一律走 `join.concat`：逐段 `..` 是 O(n²)，1.6 MB 会跑几十秒。

local bytes = require("pkg.assetgen.bytes")
local join = require("pkg.assetgen.join")
local spec = require("pkg.assetgen.spec")

local M = {}

local fail = spec.fail
local E = spec.E

--- 缺省容器：三类产物各有一个人眼 / 播放器可查的默认。
local DEFAULT = { texture = "raw", mesh = "obj", sound = "wav16" }

--- 每类产物允许的容器（交叉组合是 `-PARAM`：`encode(纹理, "wav16")` 没有意义）。
local ALLOWED = {
  texture = { raw = true, bmp24 = true, hex = true },
  mesh = { obj = true, hex = true },
  sound = { wav16 = true, pcm16 = true, hex = true },
}

--- WAV（44 字节 RIFF 头 + s16 小端交织）。字段表见契约 §2（c）。
function M.wav16(sound)
  local ch = sound.ch
  local sr = sound.sr
  local pcm = sound.pcm
  local data = #pcm
  local head = {
    "RIFF",
    bytes.u32le(36 + data),
    "WAVE",
    "fmt ",
    bytes.u32le(16),
    bytes.u16le(1), -- PCM
    bytes.u16le(ch),
    bytes.u32le(sr),
    bytes.u32le(sr * ch * 2), -- byteRate
    bytes.u16le(ch * 2), -- blockAlign
    bytes.u16le(16), -- bitsPerSample
    "data",
    bytes.u32le(data),
  }
  return join.concat(head) .. pcm
end

--- BMP24（54 字节头 + 自底向上 BGR 行，每行补到 4 的倍数）。
function M.bmp24(texture)
  local w, h, ch = texture.w, texture.h, texture.ch
  if ch < 3 then
    fail(E.PARAM, "bmp24 需要 ch ≥ 3（灰度纹理没有颜色通道），得到 ch=" .. ch)
  end
  local stride = ((w * 3 + 3) // 4) * 4
  local pad = stride - w * 3
  local image = stride * h
  local head = {
    "BM",
    bytes.u32le(54 + image),
    bytes.u16le(0),
    bytes.u16le(0),
    bytes.u32le(54),
    bytes.u32le(40), -- BITMAPINFOHEADER 大小
    bytes.u32le(w),
    bytes.u32le(h), -- 正数 = 自底向上
    bytes.u16le(1), -- planes
    bytes.u16le(24), -- bpp
    bytes.u32le(0), -- BI_RGB
    bytes.u32le(image),
    bytes.u32le(2835), -- 72 DPI（像素/米）
    bytes.u32le(2835),
    bytes.u32le(0),
    bytes.u32le(0),
  }
  local src = texture.bytes
  local rows = { join.concat(head) }
  local zeros = ("\0"):rep(pad)
  for y = h - 1, 0, -1 do
    local base = y * w * ch
    local parts = {}
    for x = 0, w - 1 do
      local at = base + x * ch + 1
      local r, g, b = src:byte(at, at + 2)
      parts[x + 1] = bytes.u8(b) .. bytes.u8(g) .. bytes.u8(r)
    end
    if pad > 0 then
      parts[w + 1] = zeros
    end
    rows[y + 2] = join.concat(parts) -- +2：rows[1] 是 BMP 头，像素行从 rows[2] 起（修复 r4-F1：y=0 行曾覆盖头部 54 字节）
  end
  return join.concat(rows)
end

--- OBJ 文本（`# assetgen mesh v<v> <algo>` + `v` 行 + `f` 行，顶点三位小数）。
function M.obj(mesh)
  local verts, faces = mesh.verts, mesh.faces
  local rows = { "# assetgen mesh v" .. mesh.v .. " " .. mesh.algo .. "\n" }
  local count = #verts // 3
  local chunks = {}
  for i = 1, count do
    local at = (i - 1) * 3
    chunks[i] = "v "
      .. bytes.mils(verts[at + 1])
      .. " "
      .. bytes.mils(verts[at + 2])
      .. " "
      .. bytes.mils(verts[at + 3])
      .. "\n"
  end
  rows[2] = join.concat(chunks)
  local tris = {}
  local face_count = #faces // 3
  for i = 1, face_count do
    local at = (i - 1) * 3
    tris[i] = "f " .. faces[at + 1] .. " " .. faces[at + 2] .. " " .. faces[at + 3] .. "\n"
  end
  rows[3] = join.concat(tris)
  return join.concat(rows)
end

--- `(product, container?)` → 字节串。缺省容器见 `DEFAULT`。
function M.encode(product, container)
  local kind = product.kind
  local allowed = ALLOWED[kind]
  if allowed == nil then
    fail(E.KIND, "encode 的 product.kind 必须是 texture / mesh / sound，得到 " .. tostring(kind))
  end
  local name = container
  if name == nil then
    name = DEFAULT[kind]
  end
  if allowed[name] == nil then
    fail(E.PARAM, "容器 " .. tostring(name) .. " 不适用于 " .. kind)
  end
  if name == "hex" then
    return bytes.hex(product.bytes)
  end
  if name == "bmp24" then
    return M.bmp24(product)
  end
  if name == "pcm16" then
    return product.pcm
  end
  -- raw / obj / wav16 都是产物的 canonical 字节（恒等）。
  return product.bytes
end

--- `encode.containers(kind)` → 允许的容器名（字节序升序；自省面，测试与 mod 共用）。
function M.containers(kind)
  local allowed = ALLOWED[kind]
  if allowed == nil then
    fail(E.KIND, "kind 必须是 texture / mesh / sound，得到 " .. tostring(kind))
  end
  local names = {}
  for name in pairs(allowed) do
    names[#names + 1] = name
  end
  for i = 2, #names do
    local value = names[i]
    local j = i - 1
    while j >= 1 and names[j] > value do
      names[j + 1] = names[j]
      j = j - 1
    end
    names[j + 1] = value
  end
  return names
end

return M
