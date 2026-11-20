-- `packages/libs/assetgen/bytes.lua` —— 字节装箱原语（整数 → 字节串）。
--
-- 契约：`docs/design/packages/assetgen.md` §2（三类产物的 `bytes`）/ §8。
--
-- # 沙箱方言
--
-- `string` 表**不可达**（`crates/ygg-sandbox/src/sandbox.rs` 的逐名断言），所以
-- 数字 → 字节只能走字符串元表的 `("%c"):format(b)`——与 `codec/sha256.lua` 同一
-- 写法（那里是语法先例，不是「包能在沙箱里跑」的证据）。
--
-- # 一个必须写死的陷阱：`>>` 是逻辑右移
--
-- Lua 5.4 的 `>>` **不做符号扩展**：`s = -1` 时 `s >> 8` 得 `0x00FF…FF`，`& 0xFF`
-- 得 `0xFF`——这正好是二补码低位字节想要的值。但**不能**用 `s // 256` 取高字节：
-- 那是算术除，`-1 // 256` 得 `-1`，字节全错。s16 的用例用 `-1 / -32768 / 32767`
-- 三个锚点逐字节钉死这条（`sound-wav-header`）。

local M = {}

local join = require("pkg.assetgen.join")

local BYTE = "%c"
local HEX2 = "%02x"
local FIRM3 = "%d.%03d"

--- 低 8 位（`v & 0xFF` 对负数给二补码低位字节）。
function M.u8(v)
  return (BYTE):format(v & 0xFF)
end

--- 小端 u16（低字节在前）。
function M.u16le(v)
  return (BYTE):format(v & 0xFF) .. (BYTE):format((v >> 8) & 0xFF)
end

--- 小端 u32。
function M.u32le(v)
  return (BYTE):format(v & 0xFF)
    .. (BYTE):format((v >> 8) & 0xFF)
    .. (BYTE):format((v >> 16) & 0xFF)
    .. (BYTE):format((v >> 24) & 0xFF)
end

--- 小端 s16（与 `u16le` 同字节，单独命名是为了让调用点自证「这是有符号样本」）。
function M.s16le(v)
  return (BYTE):format(v & 0xFF) .. (BYTE):format((v >> 8) & 0xFF)
end

--- 小写十六进制（证据面：宿主 `xxd -r -p` 能逐字节还原）。
function M.hex(text)
  local out = {}
  for i = 1, #text do
    out[i] = (HEX2):format(text:byte(i))
  end
  return join.concat(out)
end

--- 毫米整数 → 固定三位小数的十进制（OBJ 顶点文本：`-1234` → `"-1.234"`）。
---
--- `-0.000` 归一成 `0.000`：符号在整数部分之外，`v = 0` 时不会凭空写出负号。
function M.mils(v)
  local sign = ""
  if v < 0 then
    sign = "-"
    v = -v
  end
  return sign .. (FIRM3):format(v // 1000, v % 1000)
end

return M
