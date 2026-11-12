# Yggdrasill · 同步工具桥契约

> **定位**：ygg.tool.call 双层白名单（pack.json tools / allowedTools）的指针与反应器边界指引。
> **replaces**：DESIGN §2.11（L352–355）（搬运基线 `3a2536f`）
> **上游**：[docs/README.md](../README.md) · **相关**：[mod-package.md](mod-package.md) · [action-faces.md](action-faces.md)
> **搬运**：以下正文自 DESIGN.md L352–L355 逐字迁移，不重写语义。

### 2.11 工具桥契约

同步工具桥（`ygg.tool.call`）受**双层白名单**约束：`pack.json` `tools`（Layer 1 能力面，装载期宿主强制）与 `data/config.json` `allowedTools`（Layer 2 发布面，请求期 Mod 强制，必须为前者超集）——契约正文与诚实声明见 §3.2 / §3.4，本节不重复。反应器边界（只写 KV + 叙事、探针失败是警告不是门禁、机证与人证同级）见 mod README 的三条硬边界。

