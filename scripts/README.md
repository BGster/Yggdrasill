# scripts/

仓库自带的**开发/运维**辅助脚本（非内核、非 Mod、不入装载管线）。

| 脚本 | 作用 |
| --- | --- |
| `build-wasm.sh` | 构建 wasm 目标产物（WASI SDK 探测器；`crates/ygg-wasm/*.mjs` 冒烟与确定性校验的构建前置） |

---

## 决策命令（`--drive cmd:`）—— 决策者自己实现，仓库不提供默认策略

宿主在 `--drive cmd:<路径>` 下，会把**完整当前回执**写进 run 目录的 `decide-input.json`，再以该路径
为唯一参数调用决策命令；命令必须在 stdout 输出**恰好一行**：

- `N`（`1..=pendingChoice.options` 的序号），或
- `HUMAN[:理由]`（安全暂停：宿主不推进状态，退出 `0`）。

协议细节（5 秒超时、非零退出 / 多行输出 / 越界序号一律 fail closed、`YGG_DECIDE_CMD` 覆盖路径）
见 `data/playbook.json` 的 harness 章节 → decide-protocol / decide-fail-closed。

**仓库不附带默认决策脚本**。把推进实现成「机械取菜单首项」不是 goal 模式，而是无人值守自动播放：
它会在该驳回的验收上放行、在该停等的门禁上过关，等于伪造结论。

**goal 模式的决策者是 Agent 本身**：读回执的叙事与 `pendingChoice`，按现场证据自己判断该选哪一项，
再用 `--event ygg.choice_selected --payload <N>` 推进（该动作是菜单通道，不受人类动作门禁拦截）。
确需脚本化时，按具体场景自己写一个读同一份 `decide-input.json` 的命令，不要复用「默认 1」这类策略。
