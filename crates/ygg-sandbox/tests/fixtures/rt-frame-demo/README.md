# rt-frame-demo（S31-R2 fixture）

320×200 一帧 cast 同形投影（天/墙/地列 span + 精灵列 blit + present）。
与 `tests/frame_face.rs::FRAME_THROUGHPUT_MOD` 同形——吞吐真数见
`docs/design/host/rt-face.md` §6。

用途：`examples/rt --ppm/--preview` 的可复现 demo 证据通道：

```bash
cargo run -p ygg-sandbox --example rt -- \
  crates/ygg-sandbox/tests/fixtures/rt-frame-demo \
  --frames 60 --record /tmp/rt-demo.jsonl --ppm /tmp/rt-demo-ppm --preview 8734
```

manifest.json 只需 `schemaVersion`（零资源零代码段 → 字节不被解析，
与 `mod.lua` 无内容寻址耦合——改 mod.lua 无需重生成 manifest）。
