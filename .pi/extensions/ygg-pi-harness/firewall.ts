/**
 * firewall.ts —— `tool_call` 闸门：物理数据红线（优先级 0）与批量入口禁令
 *
 * 两条规则都是**物理**的（拦动作，不靠自觉），且都写成纯函数
 * [`checkToolCallFirewall`]：`harness.test.ts` 直接测它，不需要起 pi 会话。
 *
 * 规则 1（数据红线，优先级 0）：任何 `read` / `write` / `edit` / `ls` / `find` /
 *   `grep` / `bash` 触碰 `/Volumes/WenshuSpace/**`，或任何 `*.xlsx` / `*.xls` /
 *   `*.csv`（`/tmp` 下的自造合成数据除外），一律 block。
 *   判据是**动作**，不是「读没读内容」：`ls` / `stat` 一个数据目录同样是违规
 *   （行数、列名、月份范围都属涉密信息）。
 *
 * 规则 2（批量入口永久禁令）：任何 `bash` 里的 `ygg-agent --events ...`，以及
 *   用 shell 循环（`for` / `while` / `until` … `do`）或多路展开（`xargs`）批量
 *   刷 `ygg-agent --resume --event` 的写法，一律 block——批量刷状态绕过的正是
 *   `requirement.dispatch → proposal → gate` 这条链。
 *
 * 只读面：本模块不写任何状态、不 spawn 进程、不读盘（除 `os.tmpdir()` /
 * `fs.realpathSync` 解析合成目录真实路径）。
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

export interface FirewallVerdict {
  /** `true` = 拦下这次工具调用（`tool_call` 返回 `{ block: true, reason }`）。 */
  block: boolean;
  /** 拦下的原因（给模型看的一句话；`block=false` 时缺失）。 */
  reason?: string;
}

/** 受保护的数据文件扩展名（涉密指标文件）。 */
export const DATA_FILE_EXTENSIONS = [".xlsx", ".xls", ".csv"];

/** 涉密挂载目录（整棵子树，含目录本身）。 */
export const PROTECTED_DIRECTORIES = ["/Volumes/WenshuSpace"];

/** 路径参数会真正打开文件的工具。 */
const FILE_TOUCHING_TOOLS = new Set(["read", "write", "edit", "ls", "find", "grep"]);

/** 所有工具都扫一遍的路径型入参键（**只取路径语义的键**，绝不递归正文）。 */
const PATH_INPUT_KEYS = [
  "path",
  "file",
  "filePath",
  "files",
  "paths",
  "dir",
  "directory",
  "cwd",
  "root",
  "target",
  "source",
  "destination",
  "glob",
];

/** `find` 的 `pattern` 是 glob（枚举文件），`grep` 的 `pattern` 是正则（不是路径）。 */
const GLOB_PATTERN_TOOLS = new Set(["find"]);

const DATA_FILE_RE = /\.(xlsx|xls|csv)(?![a-z0-9])/i;
const YGG_AGENT_RE = /\bygg-agent\b/;

/** 合成数据根（相对这些根下的数据文件不受红线限制）。 */
export function syntheticRoots(): string[] {
  const roots = ["/tmp", "/private/tmp", os.tmpdir()];
  try {
    roots.push(fs.realpathSync(os.tmpdir()));
  } catch {
    // tmpdir 不存在的环境不必额外补一条；`os.tmpdir()` 已经在列。
  }
  const out: string[] = [];
  for (const root of roots) {
    const normalized = normalizePathText(root);
    if (normalized !== "" && !out.includes(normalized)) {
      out.push(normalized);
    }
  }
  return out;
}

/** 路径文本归一化：去引号/反引号、折叠 `//`、去尾斜杠、抹掉 `/private` 前缀差异。 */
export function normalizePathText(raw: string): string {
  let text = String(raw ?? "").trim();
  if (text === "") {
    return "";
  }
  // 包裹的引号（shell / JSON 里都常见）
  while (
    text.length >= 2 &&
    ((text.startsWith('"') && text.endsWith('"')) ||
      (text.startsWith("'") && text.endsWith("'")) ||
      (text.startsWith("`") && text.endsWith("`")))
  ) {
    text = text.slice(1, -1).trim();
  }
  text = text.replace(/\/{2,}/g, "/");
  if (text.length > 1 && text.endsWith("/")) {
    text = text.slice(0, -1);
  }
  // macOS 上 `/var` 与 `/private/var` 是同一棵树：比较前统一成一种写法。
  if (text === "/private") {
    text = "/";
  } else if (text.startsWith("/private/")) {
    text = text.slice("/private".length);
  }
  return text;
}

function expandHome(raw: string): string {
  if (raw === "~") {
    return os.homedir();
  }
  if (raw.startsWith("~/")) {
    return path.join(os.homedir(), raw.slice(2));
  }
  return raw;
}

function stripQuotes(raw: string): string {
  let text = raw.trim();
  while (
    text.length >= 2 &&
    ((text.startsWith('"') && text.endsWith('"')) ||
      (text.startsWith("'") && text.endsWith("'")))
  ) {
    text = text.slice(1, -1).trim();
  }
  return text;
}

/** 绝对路径或显式相对路径 → 判定字符串（不做 cwd 解析：不知道调用方 cwd 时不猜）。 */
function canonicalCandidate(raw: string): string {
  const text = stripQuotes(raw);
  if (text === "") {
    return "";
  }
  return normalizePathText(expandHome(text));
}

function isInside(normalizedPath: string, normalizedRoot: string): boolean {
  if (normalizedRoot === "" || normalizedPath === "") {
    return false;
  }
  const lowerPath = normalizedPath.toLowerCase();
  const lowerRoot = normalizedRoot.toLowerCase();
  return lowerPath === lowerRoot || lowerPath.startsWith(`${lowerRoot}/`);
}

function protectedRootVerdict(candidate: string): FirewallVerdict | null {
  for (const root of PROTECTED_DIRECTORIES) {
    if (isInside(candidate, normalizePathText(root))) {
      return {
        block: true,
        reason:
          `[BLOCKED:DATA-RED-LINE] 禁止访问涉密挂载目录 ${root}（优先级 0）：` +
          `任何 read / write / edit / ls / find / grep / bash 触碰该子树都算违规，` +
          `元信息（行数 / 列名 / 文件名）同样涉密。命中的路径：${candidate}`,
      };
    }
  }
  return null;
}

function dataFileVerdict(candidate: string, rawHit: string): FirewallVerdict | null {
  if (!DATA_FILE_RE.test(candidate)) {
    return null;
  }
  const synthetic = syntheticRoots();
  if (candidate.startsWith("/")) {
    for (const root of synthetic) {
      if (isInside(candidate, root)) {
        return null;
      }
    }
  }
  return {
    block: true,
    reason:
      `[BLOCKED:DATA-RED-LINE] 禁止访问真实数据文件（${DATA_FILE_EXTENSIONS.join(" / ")}，优先级 0）：` +
      `只有 /tmp 下的自造合成数据可读。命中的路径：${rawHit || candidate}。` +
      `需要真实数据验证时，请把命令交给用户运行并回贴输出；` +
      `若这是正则/提交信息里的字面量而非路径，改写成不含真实扩展名的说法。`,
  };
}

/** 单条路径文本 → 判定（红线优先于合成豁免：涉密目录下即使叫 .csv 也拦）。 */
export function checkPathText(raw: string): FirewallVerdict | null {
  const hit = String(raw ?? "").trim();
  const candidate = canonicalCandidate(hit);
  if (candidate === "") {
    return null;
  }
  return protectedRootVerdict(candidate) ?? dataFileVerdict(candidate, hit);
}

/** `bash` 命令串里参与路径判定的片段（含 `--flag=/path` 的取值侧）。 */
export function commandTokens(command: string): string[] {
  const tokens: string[] = [];
  const rough = String(command ?? "").split(/[\s;|&()<>`]+/);
  for (const piece of rough) {
    const token = stripQuotes(piece);
    if (token === "") {
      continue;
    }
    tokens.push(token);
    const eq = token.indexOf("=");
    if (eq > 0 && eq < token.length - 1) {
      tokens.push(token.slice(eq + 1));
    }
  }
  return tokens;
}

/** 规则 2：批量入口（`--events` / shell 循环 / `xargs`）的纯文本判定。 */
export function checkBatchEntrypoint(command: string): FirewallVerdict | null {
  const text = String(command ?? "");
  if (!YGG_AGENT_RE.test(text)) {
    return null;
  }
  if (/(^|\s)--events(\s|=|$)/.test(text)) {
    return {
      block: true,
      reason:
        "[BLOCKED:BATCH-ENTRYPOINT] `ygg-agent --events` 是永久禁令：批量刷事件绕过的正是 " +
        "`requirement.dispatch → proposal → gate` 这条链。一次只派发一个事件（`--event`），" +
        "并且走 `ygg_session` / `ygg_run_lanes` 的会话面。",
    };
  }
  if (/\b(for|while|until)\b[\s\S]*\bdo\b[\s\S]*\bygg-agent\b/.test(text)) {
    return {
      block: true,
      reason:
        "[BLOCKED:BATCH-ENTRYPOINT] shell 循环批量派发 `ygg-agent` 是永久禁令（`for/while … do … ygg-agent … done`）：" +
        "单发 ≠ 合规，批量刷 `--resume --event` 不产生事件链。用 `ygg_session` 逐条推进，" +
        "或 `ygg_run_lanes` 交给泳道编排。",
    };
  }
  if (/\bxargs\b[^\n]*\bygg-agent\b/.test(text)) {
    return {
      block: true,
      reason:
        "[BLOCKED:BATCH-ENTRYPOINT] `xargs ygg-agent` 是永久禁令（多路批量派发 = 同一类批量入口）：" +
        "用 `ygg_session` 逐条推进，或 `ygg_run_lanes` 交给泳道编排。",
    };
  }
  return null;
}

function stringsOf(value: unknown): string[] {
  if (typeof value === "string") {
    return [value];
  }
  if (Array.isArray(value)) {
    return value.flatMap(stringsOf);
  }
  return [];
}

/** 从工具入参里收集**路径语义**的候选（不递归正文，避免把代码里的 `.csv` 当路径）。 */
function pathCandidates(toolName: string, input: Record<string, unknown>): string[] {
  const keys = [...PATH_INPUT_KEYS];
  if (GLOB_PATTERN_TOOLS.has(toolName)) {
    keys.push("pattern");
  }
  const out: string[] = [];
  for (const key of keys) {
    for (const value of stringsOf(input[key])) {
      out.push(value);
    }
  }
  return out;
}

/**
 * `tool_call` 闸门本体（纯函数）。
 *
 * @param toolName `event.toolName`（`bash` / `read` / `write` / `edit` / `ls` / `find` / `grep` …）
 * @param input    `event.input`（工具入参；只读，不做 IO）
 */
export function checkToolCallFirewall(
  toolName: string,
  input: Record<string, unknown>,
): FirewallVerdict {
  const name = String(toolName ?? "");

  // 规则 2 先于路径扫描只看命令行：批量入口与「碰了哪个路径」无关。
  if (name === "bash") {
    const command = typeof input.command === "string" ? input.command : "";
    const batch = checkBatchEntrypoint(command);
    if (batch !== null) {
      return batch;
    }
    // 规则 1：命令串里的每个路径片段都要过红线。
    for (const token of commandTokens(command)) {
      const verdict = checkPathText(token);
      if (verdict !== null) {
        return verdict;
      }
    }
    return { block: false };
  }

  if (!FILE_TOUCHING_TOOLS.has(name)) {
    return { block: false };
  }

  for (const candidate of pathCandidates(name, input)) {
    const verdict = checkPathText(candidate);
    if (verdict !== null) {
      return verdict;
    }
  }
  return { block: false };
}
