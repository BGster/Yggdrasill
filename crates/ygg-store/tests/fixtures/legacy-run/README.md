# `legacy-run` fixture 的来历（合成，非真实局产物）

这两个文件由**合成探针 Mod**跑两步产生，用来冻结 `crates/ygg-store` 的字节兼容面：

- Mod：`packages` 之外的零业务语义探针（`event.start` → 一步 `ygg.choice_selected`），
  与 `crates/ygg-sandbox/examples/egg.rs` 里 `write_probe_mod` 生成的 Mod 同形。
- 生成者：**改动前的**宿主 CLI（master 版本；当时 example 目标尚名 `agent`，r7 更名后为 `egg`），
  命令 `--event event.start --payload 0` 后 `--resume --event ygg.choice_selected --payload 1`。
- 冻结内容：`autosave.json`（紧凑 JSON、键序字典序、**无尾随换行**）、
  `events.jsonl`（每行以 `\n` 结尾）。

**禁止**用 `.ygg/runs/**` 下的任何真实局产物替换本 fixture（那是本项目的工作内容）。

`tests/golden_bytes.rs` 的两条用法：
1. 回放：按 `events.jsonl` 的行重建 `StepDraft` 交给 `JsonFileStore`，产物必须逐字节相同；
2. 形状：`autosave.json` 末尾不得有换行、键序必须是字典序（`preserve_order` 未启用的证据）。
