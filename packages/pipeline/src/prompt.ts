// prompt.ts — few-shot 组装（test-adventure 全套为 golden 种子，FR-2；
// 13.3KB 全套注入，research Q1 实测在预算内）+ 修复轮 prompt（FR-7 错误
// 反喂；全量重生成保持双件套键引用一致性，research Q1 推荐形态）。

import { readFileSync } from "node:fs";
import { repoRoot } from "./env.js";

const FEW_SHOT_FILES: Array<{ path: string; note?: string }> = [
  { path: "packages/mods/test-adventure/mod.lua" },
  { path: "packages/mods/test-adventure/data/scenes.json" },
  { path: "packages/mods/test-adventure/pack.json" },
  {
    path: "packages/mods/test-adventure/schemas/scene-texts.json",
    note: "（样本 schema 是故事特化形态——你的输出不生成 schema，scenes 键集按你的故事自定）",
  },
];

export function readFewShotSections(): string {
  return FEW_SHOT_FILES.map(({ path, note }) => {
    const text = readFileSync(`${repoRoot()}/${path}`, "utf8");
    return `### ${path}${note ? ` ${note}` : ""}\n\`\`\`\n${text}\`\`\``;
  }).join("\n\n");
}

export function buildSystemPrompt(): string {
  return [
    "你是 Yggdrasill 图文冒险引擎的 Mod 作者。引擎以 Lua 5.4 为唯一可执行接口（D11），",
    "全部文案与数值来自资源包（ygg.data 顶层快照进 local，handler 闭包消费），Mod 代码只做结构与流程。",
    "",
    "硬约束（违反即验收门失败）：",
    "1. 单一资源条目：ygg.data(\"ygg:scene/texts\") 顶层快照；",
    "2. 实体句柄：ygg.entity.spawn() 结果必须立即 ygg.kv.set 落库，且 spawn 带 kv 门防重复；禁止把 id 存任何顶层 local；",
    "3. ygg.narrative[\"goto\"] 必须用括号形态（goto 是 Lua 5.4 保留字，点形态是语法错误）；",
    "4. ygg.choice 必须是 handler 的最后一个输出调用；章节推进的 ygg.emit 一律放在 ygg.choice_selected 回应 handler 里；",
    "5. 每个结局的叙述行必须以【结局开头（自动走查的收口判据）；至少一个结局可经选择到达；",
    "6. ygg.fsm.define 一机一事件、声明序先到先得：同一 from 的多条出边必须拆成多台状态机（参考样本的 chapter + inn_route 双机形态）；",
    "7. 结构同构（API 用法 / 目录形态 / pack 声明面照样本），内容原创（叙事不得抄样本）。",
  ].join("\n");
}

export function buildStoryPrompt(story: string): string {
  return [
    "## 任务",
    "把下面的剧本描述做成一个可装载、可通关的 Yggdrasill 图文冒险 Mod。",
    '只输出一个 JSON 对象（不要输出任何其他文本）：{"slug": "<a-z0-9-短横线短id>", "title": "<标题>", "modLua": "<mod.lua 全文>", "scenes": <data/scenes.json 的 JSON 对象>}',
    "",
    "scenes 对象固定六节：scenes（场景文案 map）/ endings（结局文案 map，每个值以【结局开头）/ " +
      'prompts（选择提示 map）/ options（选项文案 map）/ beats（过场节拍 map）/ tuning（非负整数调参 map）。' +
      "键集按你的故事自定（scenes ≥1、endings ≥1 项）；prompts/options/beats/tuning 可为空对象。",
    "modLua 必须是完整可执行的 mod.lua 源码（作为 JSON 字符串转义），事件入口固定为 event.start。",
    "",
    "## 剧本描述",
    story,
    "",
    "## 参考样本（结构同构、内容原创）",
    readFewShotSections(),
  ].join("\n\n");
}

/** 修复轮：原任务 + 前次产物全文 + 错误清单 + 只修错误约束（FR-7）。 */
export function buildRepairPrompt(story: string, priorStoryJson: string, issues: string): string {
  return [
    buildStoryPrompt(story),
    "## 上一轮产出（有缺陷，需修正）",
    "```json",
    priorStoryJson,
    "```",
    "## 验收门错误清单（必须全部修复）",
    issues,
    "",
    "只修复错误清单指出的问题；已通过部分的 narrative 结构与键引用保持不变。重新输出完整 JSON（全量重生成）。",
  ].join("\n\n");
}
