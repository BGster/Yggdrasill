#!/bin/sh
# 最小可用的决策命令示例（`--drive cmd:scripts/decide-choice.sh` 的兜底实现）。
#
# 这不是智能决策：它只做两件事——
#   1. 默认取 `pendingChoice.options` 的第 1 项（协议里「序号 1」= 菜单首项）；
#   2. 遇到**必须由人/主 Agent 判断**的情形就输出 `HUMAN:<理由>`（安全暂停，
#      状态不推进）：
#      - 回执 `meta.summary.conflicts` 非空（有未解决冲突——解决路径
#        keep_both / keep_first / revise 是架构决策，不能替你选）；
#      - 候选里出现提案验收 / 门禁 / 冲突解决类动作（`proposal.*`、
#        `gate.*`、`requirement.resolve`）——这些是「验收判断」，让脚本代选
#        等于伪造验收结论。
#
# 协议（权威说明见 `guide.show harness` → decide-protocol / decide-fail-closed）：
#   argv[1] = decide-input.json 的路径（宿主把完整当前回执写在那里）；
#   stdout 必须**恰好一行**：`N`（1..=选项数）或 `HUMAN[:理由]`；
#   非零退出 / 多行输出 / 越界序号一律被宿主判为 fail closed（安全暂停）。
#
# 真实编排请用自备的决策命令替换本脚本（读同一份 decide-input.json，
# 由主 Agent / 外部调度器给出判断），或用 `--drive stdin` 人工推进。
#
# 依赖：POSIX sh + python3（std-only，无第三方包、无绝对路径假设）。

set -eu

input="${1:-}"

if [ -z "$input" ]; then
    echo "HUMAN:用法：decide-choice.sh <decide-input.json>（宿主应以回执路径作为唯一参数）"
    exit 0
fi

if [ ! -f "$input" ]; then
    echo "HUMAN:决策输入不存在：$input"
    exit 0
fi

python3 - "$input" <<'PY'
import json
import sys

# 判断力门槛：这些动作的选项不由脚本代选（验收 / 门禁 / 冲突解决）。
JUDGEMENT_PREFIXES = (
    "proposal.accept",
    "proposal.reject",
    "gate.approve",
    "gate.reject",
    "requirement.resolve",
)


def human(reason):
    """输出协议里的安全暂停行并退出 0（宿主据此不推进状态）。"""
    print("HUMAN:" + " ".join(reason.split()))
    raise SystemExit(0)


try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        receipt = json.load(handle)
except OSError as exc:
    human("读取决策输入失败：%s" % exc)
except ValueError as exc:
    human("决策输入不是合法 JSON：%s" % exc)

if not isinstance(receipt, dict):
    human("决策输入不是回执对象")

choice = receipt.get("pendingChoice")
if not isinstance(choice, dict):
    human("当前回执没有 pendingChoice（无可推进的动作）")

options = choice.get("options")
if not isinstance(options, list) or not options:
    human("当前回执的 pendingChoice.options 为空")

meta = receipt.get("meta")
summary = meta.get("summary") if isinstance(meta, dict) else None
conflicts = summary.get("conflicts") if isinstance(summary, dict) else None
if isinstance(conflicts, list) and conflicts:
    labels = []
    for item in conflicts:
        if not isinstance(item, dict):
            continue
        labels.append(
            "%s ⇄ %s：%s"
            % (
                item.get("id", "?"),
                item.get("peer", "?"),
                str(item.get("note", "")).strip(),
            )
        )
    human(
        "有未解决冲突（%s）——keep_both / keep_first / revise 必须由人或主 Agent 决定"
        % ("；".join(labels) or "详见回执")
    )

for index, text in enumerate(options, start=1):
    if not isinstance(text, str):
        human("第 %d 个选项不是文本（回执格式异常）" % index)
    stripped = text.lstrip()
    for prefix in JUDGEMENT_PREFIXES:
        if stripped.startswith(prefix):
            human(
                "第 %d 项是验收/门禁类动作 `%s`——验收判断不能由示例脚本代做"
                % (index, prefix)
            )

# 兜底：按协议取菜单首项（序号从 1 开始）。
print(1)
PY
