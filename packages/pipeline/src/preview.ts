// preview.ts — 流式 story JSON 的容错前缀解析（A3）+ token 粗估（A2）。
//
// 背景：provider.completeStream 的分片是**任意位置切断**的文本前缀（可能在键名
// 中间、字符串中间、值中间断开），整段 JSON.parse 必然失败。本模块只回答「已
// 到达的部分能确定什么」，不做任何猜测：
//   - title：顶层 "title" 的字符串值；未闭合 → null（等后续分片）；
//   - scenes / endings：六节里的两个文案 map，逐个收集**已闭合**的 "键": "值"
//     对；截断在值中间的条目整条丢弃（不渲染半截文案）。
//
// 手工扫描而不是正则：①场景文案本身可能含 `"title":` 之类的字样，正则会被文本
// 内容骗；②必须正确处理 \" \\ 等转义与任意位置截断。零依赖（不引 JSON 解析器，
// 也不用 try/JSON.parse 猜前缀——那在截断点恰好闭合时会给出错误的半截结构）。
//
// 调用面：CLI 进度行（场景计数）与 dev 生成端点（preview 事件）共用同一解析器；
// 浏览器只渲染结构化结果——单一实现、单点测试（A3 纯函数）。
// 每次调用重扫全文（O(n)）：分片总量几十 KB 量级，整轮开销可忽略。

/** scenes / endings 节的一条文案（流式期间随文本增长逐条出现）。 */
export interface PreviewEntry {
  /** 节内键（场景 / 结局 id）。 */
  id: string;
  /** 已闭合的文案。 */
  text: string;
}

/** 流式前缀能确定的部分（缺省 = 什么都不知道）。 */
export interface StoryPreview {
  title: string | null;
  scenes: PreviewEntry[];
  endings: PreviewEntry[];
}

/** 空预览（每次新对象——调用方可能持有并累积，不共享可变引用）。 */
export function emptyPreview(): StoryPreview {
  return { title: null, scenes: [], endings: [] };
}

/** 无本地分词器的 token 粗估（4 字符 ≈ 1 token；仅供进度显示，真实用量见 usage）。 */
export function estimateTokens(chars: number): number {
  return Math.ceil(chars / 4);
}

const WHITESPACE = new Set([" ", "\t", "\n", "\r"]);

function skipWhitespace(text: string, index: number): number {
  let i = index;
  while (i < text.length && WHITESPACE.has(text[i])) {
    i += 1;
  }
  return i;
}

/**
 * 扫描一个 JSON 字符串字面量（index 指向开引号）。返回解码值与其后的下标；
 * 未闭合（流在字符串内截断）或转义序列被截断 → null。
 */
function scanString(text: string, index: number): { value: string; next: number } | null {
  let i = index + 1;
  let raw = "";
  while (i < text.length) {
    const char = text[i];
    if (char === "\\") {
      if (i + 1 >= text.length) {
        return null; // 转义序列只到了一半
      }
      raw += char + text[i + 1];
      i += 2;
      continue;
    }
    if (char === '"') {
      return { value: decodeStringBody(raw), next: i + 1 };
    }
    raw += char;
    i += 1;
  }
  return null;
}

/** 引号内的原始文本 → 实际文本（转义非法时退化为原文，不让预览整体失败）。 */
function decodeStringBody(raw: string): string {
  if (!raw.includes("\\")) {
    return raw;
  }
  try {
    return JSON.parse(`"${raw}"`) as string;
  } catch {
    return raw;
  }
}

/** 标量值（数字 / true / false / null）的截止字符。 */
function isValueDelimiter(char: string): boolean {
  return WHITESPACE.has(char) || char === "," || char === "}" || char === "]";
}

/**
 * 跳过一个非字符串值（数字 / true / false / null / 嵌套对象 / 数组），返回其后
 * 下标；值未闭合 → null。嵌套里的括号与引号按 JSON 语义配平（字符串内的括号
 * 不计数）——这样 scenes 之外的键（如 tuning）不会打断扫描。
 */
function skipValue(text: string, index: number): number | null {
  if (index >= text.length) {
    return null;
  }
  const char = text[index];
  if (char === '"') {
    return scanString(text, index)?.next ?? null;
  }
  if (char === "{" || char === "[") {
    let depth = 0;
    let i = index;
    while (i < text.length) {
      const current = text[i];
      if (current === '"') {
        const scanned = scanString(text, i);
        if (scanned === null) {
          return null; // 字符串内的括号不能当结构括号，未闭合即整体未闭合
        }
        i = scanned.next;
        continue;
      }
      if (current === "{" || current === "[") {
        depth += 1;
      } else if (current === "}" || current === "]") {
        depth -= 1;
        if (depth === 0) {
          return i + 1;
        }
      }
      i += 1;
    }
    return null;
  }
  let i = index;
  while (i < text.length && !isValueDelimiter(text[i])) {
    i += 1;
  }
  return i === index ? null : i;
}

/**
 * 扫描一个对象体（index 指向 '{' 之后）的 "键": "值" 对，只收已闭合的字符串值。
 * 返回已收集条目 + 下一个待扫下标；对象未闭合时 next = text.length（调用方据此
 * 结束扫描，已收集的条目全部保留）。
 */
function scanEntries(text: string, index: number): { entries: PreviewEntry[]; next: number } {
  const entries: PreviewEntry[] = [];
  let i = skipWhitespace(text, index);
  for (;;) {
    if (i >= text.length) {
      return { entries, next: text.length };
    }
    const char = text[i];
    if (char === "}") {
      return { entries, next: i + 1 };
    }
    if (char === ",") {
      i = skipWhitespace(text, i + 1);
      continue;
    }
    if (char !== '"') {
      return { entries, next: text.length }; // 形态不符：停止（不猜）
    }
    const key = scanString(text, i);
    if (key === null) {
      return { entries, next: text.length };
    }
    i = skipWhitespace(text, key.next);
    if (text[i] !== ":") {
      return { entries, next: text.length };
    }
    i = skipWhitespace(text, i + 1);
    if (text[i] === '"') {
      const value = scanString(text, i);
      if (value === null) {
        return { entries, next: text.length }; // 值未闭合 → 该条整条丢弃
      }
      entries.push({ id: key.value, text: value.value });
      i = skipWhitespace(text, value.next);
      continue;
    }
    const next = skipValue(text, i);
    if (next === null) {
      return { entries, next: text.length };
    }
    i = skipWhitespace(text, next);
  }
}

/**
 * 扫描六节容器体（index 指向 '{' 之后）：`scenes` / `endings` 两个文案 map 逐条
 * 收集，其余节（prompts / options / beats / tuning）整体跳过。
 * 返回已收集的两节 + 下一个待扫下标（容器未闭合时 next = text.length）。
 */
function scanSections(
  text: string,
  index: number,
): { scenes: PreviewEntry[]; endings: PreviewEntry[]; next: number } {
  // 严格按契约形态（story.scenes 固定六节）：扁平形态过不了 STORY_SCHEMA，
  // 预览不为它造期待——容错只针对**截断**，不针对形状漂移。
  const scenes: PreviewEntry[] = [];
  const endings: PreviewEntry[] = [];
  let i = skipWhitespace(text, index);
  for (;;) {
    if (i >= text.length) {
      return { scenes, endings, next: text.length };
    }
    const char = text[i];
    if (char === "}") {
      return { scenes, endings, next: i + 1 };
    }
    if (char === ",") {
      i = skipWhitespace(text, i + 1);
      continue;
    }
    if (char !== '"') {
      return { scenes, endings, next: text.length };
    }
    const key = scanString(text, i);
    if (key === null) {
      return { scenes, endings, next: text.length };
    }
    i = skipWhitespace(text, key.next);
    if (text[i] !== ":") {
      return { scenes, endings, next: text.length };
    }
    i = skipWhitespace(text, i + 1);
    const isSection = key.value === "scenes" || key.value === "endings";
    if (isSection && text[i] === "{") {
      const scanned = scanEntries(text, i + 1);
      if (key.value === "scenes") {
        scenes.push(...scanned.entries);
      } else {
        endings.push(...scanned.entries);
      }
      i = skipWhitespace(text, scanned.next);
      continue;
    }
    const next = skipValue(text, i);
    if (next === null) {
      return { scenes, endings, next: text.length };
    }
    i = skipWhitespace(text, next);
  }
}

/**
 * 从流式文本前缀解析预览（幂等：同一份文本任何时候调用都得到同一结果）。
 * 起始点取全文第一个 `{`——容忍降级路径下残留的 ```json 围栏。
 */
export function parseStoryPreview(text: string): StoryPreview {
  const preview = emptyPreview();
  const start = text.indexOf("{");
  if (start < 0) {
    return preview;
  }
  let i = skipWhitespace(text, start + 1);
  for (;;) {
    if (i >= text.length) {
      return preview; // 流还没收完：保留已收集的部分
    }
    const char = text[i];
    if (char === "}") {
      return preview;
    }
    if (char === ",") {
      i = skipWhitespace(text, i + 1);
      continue;
    }
    if (char !== '"') {
      return preview;
    }
    const key = scanString(text, i);
    if (key === null) {
      return preview;
    }
    i = skipWhitespace(text, key.next);
    if (text[i] !== ":") {
      return preview;
    }
    i = skipWhitespace(text, i + 1);
    if (key.value === "title" && text[i] === '"') {
      const value = scanString(text, i);
      if (value === null) {
        return preview; // title 未闭合：保持 null（下次分片重扫）
      }
      preview.title = value.value;
      i = skipWhitespace(text, value.next);
      continue;
    }
    if (key.value === "scenes" && text[i] === "{") {
      // 注意：顶层 scenes 是**六节容器**（story.scenes.scenes 才是文案 map）。
      const scanned = scanSections(text, i + 1);
      preview.scenes = scanned.scenes;
      preview.endings = scanned.endings;
      i = skipWhitespace(text, scanned.next);
      continue;
    }
    const next = skipValue(text, i);
    if (next === null) {
      return preview;
    }
    i = skipWhitespace(text, next);
  }
}
