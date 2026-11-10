/**
 * archive.ts —— 会话归档结论的**纯判定**（`harness.archive_sessions` 回执 → 三态）
 *
 * 为什么单独一个文件：`index.ts` 顶部 import 了 pi 的扩展 SDK，
 * `node --experimental-strip-types --test` 在没有 pi 运行时的时候就加载不了它。
 * 把判定抽成零依赖模块，`harness.test.ts` 才能直接咬住「不得编造 0/0」这条契约；
 * `index.ts` 只做 `export * from "./archive.ts"` 与一处调用（装配层不含业务判定）。
 *
 * 语义要害：`settlement.sessions` 原本的证词是「本局 subagent 会话已归档」，
 * 但实际验的只是「某个 adapter 推导出来的目录里提到这个 parent 的 jsonl 数为 0」。
 * 在非 Pi harness 下这两件事**不等价**，所以结论必须先分态——
 *
 * | kind | 宿主输出 | 是否算证据 |
 * | --- | --- | --- |
 * | `swept` | `moved=N;remaining=0;dest=<真实目录>` | ✅ 是 |
 * | `no-store` | `dest=generic:no-session-store:<batch>` | ❌ 不是（但可允许收口） |
 * | `unavailable` | 非零退出 / `builtin_usage_error` / 外域 `dest` | ❌ 不是 |
 * | `unsettled` | 工具没回可判定结果（含 `remaining>0`） | ❌ 不是 |
 *
 * **禁止第四种**：`dest` 非空但**没有任何 harness 认领**（典型：DSH 局里
 * `dest=~/.pi/agent/sessions-trash/...`）。那条路径确实存在，但它归档的不是本局
 * 会话——把它写成「查空了」就是谎报。宁可说「没查」，不谎报「查空了」。
 */

/**
 * 会话归档的结论（**只允许这三种 + 一种「还没跑成」**）。
 */
export type ArchiveVerdict =
  | { kind: "swept"; moved: number; dest: string }
  | { kind: "no-store"; batch: string }
  | { kind: "unavailable"; reason: string }
  | { kind: "unsettled"; reason: string };

/** Pi / Claude Code / Codex 的会话库根（`~/.pi`、`~/.claude`、`~/.codex`）。 */
const FOREIGN_SESSION_ROOTS = ["/.pi/", "/.claude/", "/.codex/"];

/** `dest` 是否落在某个「别人家」的会话库根下。 */
function isForeignSessionRoot(dest: string): boolean {
  const normalized = `/${dest.replace(/\\/g, "/").replace(/^\/+/, "")}`;
  const withSlash = normalized.endsWith("/") ? normalized : `${normalized}/`;
  return FOREIGN_SESSION_ROOTS.some((root) => withSlash.includes(root));
}

/**
 * 从 `harness.archive_sessions` 的回执叙事里读出**可区分的**归档结论（纯函数）。
 *
 * 只读叙事，不改任何状态；失败一律是「没查成」，**绝不**降级成「查空了」（`moved 0`）。
 *
 * @param narrative 回执的 `narrative` 行（整段传进来，这里只做行内匹配）
 * @param options.destRoot `dest` 必须落在哪个根下才算本局声明的 adapter 的库；
 *                         缺省时按 {@link FOREIGN_SESSION_ROOTS} 判「外域」
 */
export function analyzeArchiveVerdict(
  narrative: readonly string[],
  options: { destRoot?: string } = {},
): ArchiveVerdict {
  const lines = [...(narrative ?? [])].map((line) => String(line ?? ""));
  const joined = lines.join("\n");

  // 「没有库可查」：宿主的 `generic` 适配器明确说了这件事——不是失败，也不是证据。
  const noStore = /dest=generic:no-session-store:(\S+)/.exec(joined);
  if (noStore !== null) {
    return { kind: "no-store", batch: noStore[1] };
  }

  // 明确失败：工具错误 / `builtin_usage_error` / 被拦。
  const failure = lines.find(
    (line) =>
      line.includes("[TOOL FAILED]") ||
      line.includes("[VFS REFUSED]") ||
      line.includes("builtin_usage_error") ||
      line.includes("[BLOCKED"),
  );
  if (failure !== undefined) {
    return { kind: "unavailable", reason: failure.trim() };
  }

  const movedMatch = /moved=(-?\d+)/.exec(joined);
  const remainingMatch = /remaining=(-?\d+)/.exec(joined);
  const destMatch = /dest=(\S+)/.exec(joined);
  if (movedMatch === null || remainingMatch === null || destMatch === null) {
    return {
      kind: "unsettled",
      reason:
        lines.length === 0
          ? "回执没有任何叙事行（工具没有回报结果）"
          : `回执里没有完整的 moved/remaining/dest 三元组：${lines[lines.length - 1] ?? ""}`.trim(),
    };
  }
  const moved = Number(movedMatch[1]);
  const remaining = Number(remainingMatch[1]);
  const dest = destMatch[1];

  if (remaining !== 0) {
    return { kind: "unsettled", reason: `remaining=${remaining}（还有会话没归档）` };
  }

  // 「查过且查空」还要求 `dest` 确实落在本局声明的 adapter 的根下。
  const root = (options.destRoot ?? "").trim();
  if (root === "") {
    if (isForeignSessionRoot(dest)) {
      return {
        kind: "unavailable",
        reason:
          `foreign-store：dest=${dest} 落在 Pi / Claude / Codex 的会话库根下，` +
          "但本局没有可认领它的 adapter 声明",
      };
    }
    return { kind: "swept", moved: Number.isFinite(moved) ? moved : 0, dest };
  }
  const normalizedDest = dest.replace(/\\/g, "/");
  const normalizedRoot = root.replace(/\\/g, "/").replace(/\/+$/, "");
  if (!normalizedDest.startsWith(normalizedRoot)) {
    return {
      kind: "unavailable",
      reason: `foreign-store：dest=${dest} 不在本局声明的会话库根 ${normalizedRoot} 下`,
    };
  }
  return { kind: "swept", moved: Number.isFinite(moved) ? moved : 0, dest };
}

/**
 * 归档结论 → `settlement.evolution` 的 `sessions=` 段（或 `null` = 不写这一段）。
 *
 * - `swept`：**不写**——让 Mod 的机证接管（`[SESSIONS SETTLED]` 已经落了 `remaining=0`）。
 * - 其余三态：写 `sessions=0/0/unsettled:<kind>`，让台账**自证是「没有库 / 没查成」
 *   而不是「清空了」**。`remaining` 必须是 0 才能 `review.note`，但 `dest` 不能再冒充
 *   一个真实目录——那正是本轮要消灭的假象。
 */
export function archiveSessionsSegment(verdict: ArchiveVerdict): string | null {
  if (verdict.kind === "swept") {
    return null;
  }
  return `sessions=0/0/unsettled:${verdict.kind}`;
}

/** 归档结论的一行回执（整行保留，不截断）。 */
export function archiveVerdictNarrative(verdict: ArchiveVerdict): string {
  switch (verdict.kind) {
    case "swept":
      return `[SESSIONS SWEPT] moved=${verdict.moved} remaining=0 dest=${verdict.dest}`;
    case "no-store":
      return (
        `[SESSIONS NOT SETTLED] no-store: 本局的 harness 没有会话库可查` +
        `（batch=${verdict.batch}）——这不是「清空了」，是没有可归档的落点`
      );
    case "unavailable":
      return `[SESSIONS NOT SETTLED] unavailable: ${verdict.reason}`;
    default:
      return `[SESSIONS NOT SETTLED] unsettled: ${verdict.reason}`;
  }
}
