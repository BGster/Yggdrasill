-- rt-audio-demo（S31-R3）：rt 面四件全走通的实弹一局——cast 列 span +
-- 精灵 blit + 鼠标 dx 转向列偏移 + 开火键→ygg.rt.audio.play（波形 = Saltline
-- bundle sfx_rifle_shot 参数子集，整数参数只读引用：sr=22050 ms=120 ch=1
-- amp=900 lp=620，src/assets/bundle.lua）。demo 证据跑法（rt-face.md §6）：
--   cargo run -p ygg-sandbox --example rt -- tests/fixtures/rt-audio-demo \
--     --frames 1800 --record /tmp/r3-rec.jsonl --audio /tmp/r3-rec.wav \
--     --input tests/fixtures/rt-audio-demo/input-30s.jsonl
-- 回放（两次重放 WAV 字节全等）：
--   ... --replay /tmp/r3-rec.jsonl --audio /tmp/r3-rep.wav
local h = ygg.rt.frame.open({w = 320, h = 200})
local a = ygg.rt.audio.open({rate = 22050})

-- Saltline bundle SOUNDS[1] 参数（只读引用）：sfx_rifle_shot 噪声爆
-- 整数 LCG 噪声 + 一阶低通（lp/22050 定点比），s16 小端单声道。
local SR, MS, AMP, LP = 22050, 120, 900, 620
local seed = 4711
local function shot_pcm()
  local n = (SR * MS) // 1000
  local acc, parts = 0, {}
  for _ = 1, n do
    seed = (seed * 1103515245 + 12345) % 2147483648
    local white = (seed % 65536) - 32768
    acc = acc + (white - acc) * LP // SR
    local v = acc * AMP // 32768
    if v > 32767 then v = 32767 elseif v < -32768 then v = -32768 end
    local u = v % 65536
    parts[#parts + 1] = string.char(u % 256, u // 256)
  end
  return table.concat(parts)
end
local SHOT = shot_pcm()

local yaw = 0
ygg.on("rt.tick", function()
  local keys = ygg.rt.input.keys()
  local mouse = ygg.rt.input.mouse()
  yaw = (yaw + mouse.dx) % 320
  for col = 0, 319 do
    local src = (col + yaw) % 320
    local wall = 40 + (src * 37) % 90
    local top = (200 - wall) // 2
    ygg.rt.frame.span(h, col, 0, 1, top, 0x0A1428FF)
    ygg.rt.frame.span(h, col, top, 1, wall, 0xB04A28FF)
    ygg.rt.frame.span(h, col, top + wall, 1, 200 - top - wall, 0x204030FF)
  end
  ygg.rt.frame.blit(h, 154, 90, 6,
    "\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255")
  ygg.rt.frame.present(h)
  for _, k in ipairs(keys.pressed) do
    if k == 32 then ygg.rt.audio.play(a, SHOT) end
  end
end)
