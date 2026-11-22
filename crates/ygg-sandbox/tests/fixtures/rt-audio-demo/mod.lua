-- rt-audio-demo（S31-R3）：rt 面四件全走通的实弹一局——cast 列 span +
-- 精灵 blit + 鼠标 dx 转向列偏移 + 开火键→ygg.rt.audio.play。
-- 波形参数抄 Saltline bundle sfx_rifle_shot（sr=22050 ms=120 ch=1 amp=900
-- lp=620，src/assets/bundle.lua 整数参数只读引用）：mod 沙箱白名单无
-- string/table 库，故波形以 64 字节种子 × 整周期 `..` 拼装（32 样本 × 80
-- 周期 = 2560 样本 ≈ 116ms，形状 = 衰减爆裂，非逐样本复刻）。
-- demo 证据跑法（rt-face.md §6）：
--   cargo run -p ygg-sandbox --example rt -- crates/ygg-sandbox/tests/fixtures/rt-audio-demo \
--     --frames 1800 --record /tmp/r3-rec.jsonl --audio /tmp/r3-rec.wav \
--     --input crates/ygg-sandbox/tests/fixtures/rt-audio-demo/input-30s.jsonl
-- 回放两次 WAV 字节全等：
--   ... --replay /tmp/r3-rec.jsonl --audio /tmp/r3-rep1.wav   （再跑 rep2）
local h = ygg.rt.frame.open({w = 320, h = 200})
local a = ygg.rt.audio.open({rate = 22050})

-- 32 样本衰减爆裂（s16 小端手写字面量，amp=900 域内整数）。
local SEED = "\x84\x03\x10\xfc\xae\x02\x5a\xfd\x02\x02\xe2\xfd\x8c\x01\x38\xfe" ..
             "\xd4\x00\x9a\xfe\x62\x00\x34\xff\x10\x00\xf4\xff\xe0\xff\xd0\xff" ..
             "\x9a\x01\x30\xfc\x7c\x01\xaa\xfd\x06\x01\x52\xfe\xce\x00\x1e\xff" ..
             "\x8a\x00\xe2\xff\x56\x00\x9a\xff\x2a\x00\x52\xff\x04\x00\x12\x00"
local SHOT = SEED
for _ = 2, 80 do SHOT = SHOT .. SEED end  -- 32×80 = 2560 样本 ≈ 116ms@22050

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
