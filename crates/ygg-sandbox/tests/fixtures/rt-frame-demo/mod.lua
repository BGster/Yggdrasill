-- rt-frame-demo（S31-R2）：320×200 一帧 cast 同形投影——天/墙/地列 span +
-- 精灵列 blit + present。与 tests/frame_face.rs::FRAME_THROUGHPUT_MOD 同形
-- （吞吐口径见 rt-face.md §6），供 examples/rt --ppm/--preview 的 demo 证据：
--   cargo run -p ygg-sandbox --example rt -- tests/fixtures/rt-frame-demo \
--     --frames 60 --record /tmp/rt-demo.jsonl --ppm /tmp/rt-demo-ppm --preview 8734
local h = ygg.rt.frame.open({w = 320, h = 200})
ygg.on("rt.tick", function()
  ygg.rt.frame.clear(h, 0x182838FF)
  for col = 0, 319 do
    local wall = 40 + (col * 37) % 90
    local top = (200 - wall) // 2
    ygg.rt.frame.span(h, col, 0, 1, top, 0x0A1428FF)
    ygg.rt.frame.span(h, col, top, 1, wall, 0xB04A28FF)
    ygg.rt.frame.span(h, col, top + wall, 1, 200 - top - wall, 0x204030FF)
  end
  ygg.rt.frame.blit(h, 100, 90, 6, "\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255\222\184\135\255")
  ygg.rt.frame.present(h)
end)
