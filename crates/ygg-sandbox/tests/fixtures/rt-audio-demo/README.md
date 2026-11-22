# rt-audio-demo（S31-R3 fixture）

rt 面四件全走通的实弹一局（Leg B）：cast 列 span + 精灵 blit + 鼠标 dx
转向列偏移 + 开火键（空格 32）→ `ygg.rt.audio.play`。波形参数抄 Saltline
bundle `sfx_rifle_shot`（sr=22050 ms=120 ch=1 amp=900 lp=620，整数只读引用）。

用途：Saltline 接入预演的可复现 demo 证据通道（rt-face.md §6）：

```bash
cargo run -p ygg-sandbox --example rt -- \
  crates/ygg-sandbox/tests/fixtures/rt-audio-demo \
  --frames 1800 --record /tmp/r3-rec.jsonl --audio /tmp/r3-rec.wav \
  --input crates/ygg-sandbox/tests/fixtures/rt-audio-demo/input-30s.jsonl
# 回放两次，WAV 字节全等 + 链全等 + audio 字段全等：
cargo run -p ygg-sandbox --example rt -- \
  crates/ygg-sandbox/tests/fixtures/rt-audio-demo \
  --replay /tmp/r3-rec.jsonl --audio /tmp/r3-rep1.wav
cargo run -p ygg-sandbox --example rt -- \
  crates/ygg-sandbox/tests/fixtures/rt-audio-demo \
  --replay /tmp/r3-rec.jsonl --audio /tmp/r3-rep2.wav
cmp /tmp/r3-rep1.wav /tmp/r3-rep2.wav && echo "WAV 字节全等"
```

真 Saltline mod 的八条实改属 S30 轮（`mod.lua:601` 解析链不可选动作，
unmodified 下射击结构性不可能——如实声明，rt-face.md 音频面节）。
manifest.json 只需 `schemaVersion`（同 rt-frame-demo：零资源零代码段，
改 mod.lua 无需重生成 manifest）。
