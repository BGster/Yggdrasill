-- `src/simframe.lua` —— Tier 1 批量帧的 Lua 侧编码器（模块顶层纯导出）。
--
-- # 为什么存在
--
-- 开局铺场走宿主的批式铁律面（`ygg.sim.spawn_batch`，P-K A1 同纪律：批式进出、
-- 没有逐实体 sim getter）。宿主入口只收**字节帧**（`crates/ygg-sim/src/canonical.rs`
-- 的 FrameWriter 线格式：魔数 + 版本 + flags + u32 rows + u16 组件数 + 列载荷 +
-- 尾 digest），而沙箱方言里没有 `string.char` / `string.pack` 可用。本文件按仓内
-- 已有的两个先例拼出同一份字节：
--   * 字节构造：`("%c"):format(n)`（`packages/libs/codec/sha256.lua` 的既有方言）；
--   * 尾 digest：FNV-1a 64（`crates/ygg-sim/src/hash.rs` 与
--     `packages/libs/procgen/rng.lua` 的跨语言同源实现，乘法按 2^64 回绕）。
--
-- # 本局用到的帧形状（只有一种）
--
-- 裸实体批（组件数 = 0）：`n` 个无组件实体进空 archetype。世界规模因此进 sim
-- 状态哈希——「重放后 sim.hash 逐字节相等」（契约 §五 P4）断的就不是常数了。
-- 带列的帧（Number/Str/Bool/Ref/Tag）本局不用：人物/道具/任务的三系统数据形状
-- 全落 KV/entity/派生事件面（计划 §4「热核面 (b) 预计零项」裁决）。
--
-- # 纪律
--
-- 模块顶层只有 local / 常量 / 函数定义；`ygg` 访问一律在被调用的函数体里
-- （与 packages 同一条纯度线，`ygg-lint --pack` 的 YGG-MOD-EFFECT 静态盯防）。

local M = { VERSION = "1.0.0" }

-- 帧魔数（与 canonical::MAGIC 一致：b"YGGSIM\0\x01"）。
local MAGIC = "YGGSIM\0\1"
-- 帧格式版本（canonical::FMT_VERSION = 2；读取器收 {1, 2}，写入器产 2）。
local FMT_VERSION = 2
-- FNV-1a 64 常量（hash.rs 的 OFFSET / PRIME，照抄不改——跨语言同源）。
local FNV_OFFSET = 0xCBF29CE484222325
local FNV_PRIME = 0x100000001B3

-- ── 字节面（沙箱方言：("%c"):format 单字节；位运算拼小端）───────────────────

--- 1 字节。
local function u8(value)
  return ("%c"):format(value & 0xFF)
end

--- 2 字节小端。
local function u16(value)
  return u8(value) .. u8(value >> 8)
end

--- 4 字节小端。
local function u32(value)
  return u8(value) .. u8(value >> 8) .. u8(value >> 16) .. u8(value >> 24)
end

--- 8 字节小端（接受任意 64 位位型；负数 = 位型直出，与 Rust 的 to_le_bytes 同形）。
local function u64(value)
  return u8(value)
    .. u8(value >> 8)
    .. u8(value >> 16)
    .. u8(value >> 24)
    .. u8(value >> 32)
    .. u8(value >> 40)
    .. u8(value >> 48)
    .. u8(value >> 56)
end

--- FNV-1a 64：`h ^= byte; h *= prime`，乘法按 2^64 回绕（Lua 5.4 整数语义）。
---
--- 输入是字节串（Lua 串 8-bit 干净，`\0` 与正文同权）：`#s` 与 `:byte(i)` 对
--- 含 NUL 串的语义与字节数一致。
local function fnv1a64(bytes)
  local hash = FNV_OFFSET
  for i = 1, #bytes do
    hash = hash ~ bytes:byte(i)
    hash = hash * FNV_PRIME
  end
  return hash
end

-- ── 导出面 ──────────────────────────────────────────────────────────────────

--- `spawn_entities(n)` → 裸实体批帧字节串（n 个无组件实体）。
---
--- 帧布局（canonical.rs）：MAGIC(8) | u16 版本 | u16 flags(=0) | u32 rows |
--- u16 组件数(=0) | 尾 digest(fnv1a64 over 前缀, 8 字节小端)。
function M.spawn_entities(count)
  if type(count) ~= "number" or count ~= (count // 1) or count < 1 or count > 0xFFFFFFFF then
    error("YGG-MOD-SIMFRAME-ROWS: 行数必须是 1..=0xFFFFFFFF 的整数，得到 " .. tostring(count), 0)
  end
  local head = MAGIC .. u16(FMT_VERSION) .. u16(0) .. u32(count) .. u16(0)
  return head .. u64(fnv1a64(head))
end

--- `despawn_one(id)` → 注销单实体帧（`u32 n=1 | u64 id | 尾 digest`；despawn
--- 帧无魔数头——sim.despawn_batch 的读取从行数直入，与 spawn 头格式不同）。
function M.despawn_one(id)
  if type(id) ~= "number" or id ~= (id // 1) or id < 1 then
    error("YGG-MOD-SIMFRAME-ID: 实体 id 必须是正整数，得到 " .. tostring(id), 0)
  end
  local head = u32(1) .. u64(id)
  return head .. u64(fnv1a64(head))
end

--- `fnv1a64(bytes)` 的导出副本（自测直接对向量；不重复造第二份算法）。
function M.digest(bytes)
  return fnv1a64(bytes)
end

return M
