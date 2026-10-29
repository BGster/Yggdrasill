// messages.ts — revise 诊断码 → 用户可操作文案（A5）。
//
// 为什么单独一层：revise 的拒绝理由几乎都能翻译成「下一步做什么」（勾选授权 /
// 换个说法 / 重新生成），而这些文案要按**诊断码**归纳（同一轮可能有多条同码
// 诊断，逐条重复三遍「勾选允许修改 Lua」只会淹没有效信息）。抽成纯函数便于
// 单测锚定措辞，也避免文案散落在 JSX 里。
import type { ReviseIssueView } from "./protocol";

/** 空 diff 的提示（成功面上的 noop，不来自诊断列表）。 */
export const NOOP_HINT = "模型认为无需修改（本轮无变更）——试试更具体的意见（点名场景或句子）。";

/** 诊断码 → 可操作提示；无映射的码返回 null（面板只展示原始诊断行）。 */
export function reviseCodeHint(code: string): string | null {
  switch (code) {
    case "REVISE_LUA_FROZEN":
      return "本次修改涉及 Lua 逻辑——勾选「允许修改 Lua」后重试";
    case "REVISE_STRUCTURE":
      return "本次修改增删了场景/结局等键（结构变更）——勾选「允许修改 Lua」后重试";
    case "REVISE_IDENTITY":
      return "本次修改改动了作品身份（slug/title）——revise 不改身份；换条意见，或重新生成一个作品";
    case "REVISE_NOOP":
      return NOOP_HINT;
    default:
      return null;
  }
}

/** 一批诊断 → 去重后的提示清单（保持首次出现序，同码只留一条）。 */
export function reviseHints(issues: ReviseIssueView[]): string[] {
  const hints: string[] = [];
  for (const issue of issues) {
    const hint = reviseCodeHint(issue.code);
    if (hint !== null && !hints.includes(hint)) {
      hints.push(hint);
    }
  }
  return hints;
}
