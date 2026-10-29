// diff-gate.ts — revise 的白名单 diff 校验（A4 核心新机制）。
//
// 分工：`revise`（LLM 定向改写）与 `generate`（LLM 从零创作）的唯一差别是
// **改动范围受约束**——意见驱动的是内容，不是结构。所以 revise 在跑三道门
// 之前先过本模块：把新 story 与磁盘现状逐叶子比对，产出一份「变更路径清单
// + 越权错误」，越权直接拒绝（不落盘，现场保持原样）。
//
// 三条冻结（违约即 REVISE_* 错误）：
//   1. 身份锚点 slug 恒冻结；title 在磁盘锚点可信时冻结（REVISE_IDENTITY）；
//   2. mod.lua 逐字冻结（REVISE_LUA_FROZEN）——Lua 是结构/流程面，改它等于
//      换实现，不是改文案；
//   3. scenes 六节的键集（场景/结局/…的 id）冻结（REVISE_STRUCTURE）——键增删
//      是结构变更，冻结的 mod.lua 引用会悬空（删）或成死数据（增）。
// 需要以上三者之一变动时，用户显式 `--allow-lua` 授权大改（一处开关同时解锁
// 2 与 3——「授权动代码」天然包含「授权动它引用的键集」）。
//
// 不设路径白名单：文本改动改哪个场景由意见驱动，硬过滤会误伤合理修改；
// 只做「结构 vs 内容」的区分。空 diff 不是错误（模型可能判断无需修改），
// 降级为 warning 放行。

import type { GateIssue, StoryMod } from "./types.js";

/** scenes 六节固定序（与 STORY_SCHEMA 的 required 序一致；变更摘要按此序输出）。 */
export const SCENE_SECTIONS = [
  "scenes",
  "endings",
  "prompts",
  "options",
  "beats",
  "tuning",
] as const;

export type SceneSection = (typeof SCENE_SECTIONS)[number];

export interface DiffOptions {
  /** 显式授权大改：放行 mod.lua 变更与六节键集增删。 */
  allowLua: boolean;
  /**
   * 原标题是否可信：A1-A3 期产物没有 title 锚点（pack.json 是 D-9 固定模板，
   * 不含 slug/title），此时无法判定 title 是否被改，降级为不校验。
   */
  titleKnown?: boolean;
}

export interface DiffOutcome {
  /** 白名单校验的诊断（error = 越权拒绝；warning = 空 diff 等提示）。 */
  issues: GateIssue[];
  /** 变更路径清单（点分：modLua / scenes.<节>.<键>），确定性序。 */
  changed: string[];
  /** 其中属于结构变更（六节键增删）的路径——allowLua 放行时仍如实报告。 */
  structural: string[];
}

export function diffStory(
  previous: StoryMod,
  next: StoryMod,
  options: DiffOptions,
): DiffOutcome {
  const issues: GateIssue[] = [];
  const changed: string[] = [];
  const structural: string[] = [];
  const titleKnown = options.titleKnown ?? true;

  // ① 身份锚点：slug 恒冻结（目录名 = 作品 id）；title 在有锚点时冻结。
  if (next.slug !== previous.slug) {
    changed.push("slug"); // 变更事实如实记录（即使被拒——日志/展示面需要看到动过什么）
    issues.push(identityIssue("slug", previous.slug, next.slug));
  }
  if (titleKnown && next.title !== previous.title) {
    changed.push("title");
    issues.push(identityIssue("title", previous.title, next.title));
  }

  // ② mod.lua 逐字冻结（字符串相等即可：管线不做 Lua 语义 diff）。
  if (next.modLua !== previous.modLua) {
    changed.push("modLua");
    if (!options.allowLua) {
      issues.push({
        gate: "revise",
        file: "mod.lua",
        code: "REVISE_LUA_FROZEN",
        severity: "error",
        message: "mod.lua 被改动，但本轮未授权 Lua 变更（mod.lua 逐字冻结）",
        hint: "修改意见应只改资源文本；确需改结构/流程时加 --allow-lua（revise -- <slug> \"<意见>\" --allow-lua）",
      });
    }
  }

  // ③ scenes 六节逐叶子比对：键增删 = 结构变更；值变 = 内容变更（放行）。
  for (const section of SCENE_SECTIONS) {
    const before = asMap(previous.scenes?.[section]);
    const after = asMap(next.scenes?.[section]);
    for (const key of unionKeys(before, after)) {
      const path = `scenes.${section}.${key}`;
      const inBefore = Object.hasOwn(before, key);
      const inAfter = Object.hasOwn(after, key);
      if (inBefore !== inAfter) {
        structural.push(path);
        changed.push(path);
        continue;
      }
      if (before[key] !== after[key]) {
        changed.push(path);
      }
    }
  }
  if (structural.length > 0 && !options.allowLua) {
    issues.push({
      gate: "revise",
      file: "data/scenes.json",
      code: "REVISE_STRUCTURE",
      severity: "error",
      message:
        `结构性变更（六节键集增删）${structural.length} 处未授权：${structural.join("、")}` +
        "——冻结的 mod.lua 引用会悬空或成死数据",
      hint: "只改已有键的文案/数值；确需增删键时加 --allow-lua（同时放行 mod.lua 变更）",
    });
  }

  // ④ 空 diff：模型判断无需修改 → 警告放行（不阻断用户流程）。
  // 已有错误时不报 NOOP：越权是主因，再叠加「无变更」只会误导入排查方向。
  if (changed.length === 0 && issues.length === 0) {
    issues.push({
      gate: "revise",
      file: "<response>",
      code: "REVISE_NOOP",
      severity: "warning",
      message: "本轮无任何变更（模型认为无需修改），按放行处理",
      hint: "意见可能过于笼统；换更具体的表述（指明场景/结局/句子）重试",
    });
  }

  return { issues, changed, structural };
}

function identityIssue(field: "slug" | "title", previous: string, next: string): GateIssue {
  return {
    gate: "revise",
    file: "<response>",
    code: "REVISE_IDENTITY",
    severity: "error",
    message: `${field} 被改动（"${previous}" → "${next}"）：revise 不得改变作品身份`,
    hint:
      field === "slug"
        ? "slug 是作品 id（目录名 + 存档键）；换 id 请重新 generate 一个作品"
        : "标题是作品身份的一部分；如确需改名，请重新 generate（或用 --allow-lua 之外的显式改名流程）",
  };
}

function asMap(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

/** 两图键并集（字典序——变更摘要的输出序稳定、可断言）。 */
function unionKeys(
  before: Record<string, unknown>,
  after: Record<string, unknown>,
): string[] {
  return [...new Set([...Object.keys(before), ...Object.keys(after)])].sort();
}
