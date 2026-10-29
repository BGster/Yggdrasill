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

/**
 * 定向修改轮（A4）：现有 story 全文 + 修改意见 + 冻结项 + 只改意见相关部分。
 *
 * 不带 few-shot（generate 的 13.3KB 样本）：revise 的最佳参考就是「现有 Mod
 * 自身」——它既是结构同构的现成范本，又是必须逐字保留的基线；再叠样本只会
 * 稀释「原样回填」的注意力，并把风格往样本上拽。
 *
 * 冻结清单与 diff-gate.ts 一一对应（提示面先说清楚，减少白跑一轮的概率）：
 * 拒什么、为什么拒、以及在 `--allow-lua` 下什么被解锁。
 */
export function buildRevisePrompt(
  priorStoryJson: string,
  feedback: string,
  options: { allowLua: boolean },
): string {
  const frozen = options.allowLua
    ? [
        "3. 本轮已授权大改（--allow-lua）：mod.lua 可以改、scenes 六节的键可以增删——",
        "   但六节结构、结局文案的【结局前缀、以及 ygg-lint 三规则与走查收口判据仍然必须满足；",
        "4. 未受意见影响的文本仍逐字保留（授权的是「可以改」，不是「必须改」）。",
      ]
    : [
        "3. mod.lua 必须逐字原样回填（字符串完全相等）——本轮未授权 Lua 变更；",
        "4. scenes 六节（scenes/endings/prompts/options/beats/tuning）的键集不得增删——",
        "   增删键是结构变更，会让冻结的 mod.lua 引用悬空（删）或变成死数据（增）；只改已有键的文案 / tuning 数值；",
        "5. 未受意见影响的文本逐字保留（不要顺手润色、不要重排键序）。",
      ];
  return [
    "## 任务",
    "对下面这个**既有**的 Yggdrasill 图文冒险 Mod 做定向修改：只改用户意见涉及的部分，其余内容逐字保留。",
    '只输出一个 JSON 对象（不要输出任何其他文本）：{"slug": …, "title": …, "modLua": …, "scenes": …}',
    "四键形态与生成轮完全一致——全量回填，不是 diff 片段（管线会自己做 diff 并逐条校验）。",
    "",
    "## 冻结项（违反即被白名单校验拒绝，整轮作废）",
    "1. slug 原样保留（作品 id / 目录名 / 存档键，改了就不是同一个作品）；",
    "2. title 原样保留（作品标题属于身份，修改意见只改内容）；",
    ...frozen,
    "",
    "## 修改意见（本轮唯一的改动依据）",
    feedback,
    "",
    "## 现有 Mod（当前磁盘产物，即修改基线）",
    "```json",
    priorStoryJson,
    "```",
    "",
    "## 输出要求",
    "回填完整 JSON：未被意见涉及的场景 / 结局 / 选项 / 提示 / 节拍文案、tuning 数值、",
    "以及（未授权时的）modLua 一律保持原样。意见本身有歧义时按最保守的读法改",
    "（改一两句文案优于重写整个场景）。",
  ].join("\n");
}
