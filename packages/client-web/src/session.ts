// ============================================================================
// session.ts — 游戏面板的会话状态机（纯函数，UI 无关，单测主靶）。
//
// 语义镜像 CLI adventure.rs 主循环：
// - 事件信封 → 叙事行按序追加 → ⚠ 警告行 → 待决选择更新；
// - 结局叙事行（【结局 前缀）出现即收口（ended），追加「—— 冒险结束 ——」；
// - 错误信封 → 错误条目（码 + 文案），保留原待决选择（CLI 会丢选择面，
//   UI 侧保留以便重试——刻意偏差，见 commit 汇报）；
// - 存档/状态/读档完成 → 系统行（不重放叙事——对应 CLI 的 fresh=false）。
// ============================================================================

import type { Envelope, PendingChoice } from "./kernel";

/** 日志条目（叙事面板按序渲染的原子）。 */
export type LogEntry =
  | { kind: "narrative"; id: number; text: string }
  | { kind: "warning"; id: number; text: string }
  | { kind: "system"; id: number; text: string }
  | { kind: "error"; id: number; code: string; message: string };

export interface SessionState {
  entries: LogEntry[];
  pendingChoice: PendingChoice | null;
  /** 结局叙事行已出现——选择面收口。 */
  ended: boolean;
  /** start() 信封已落账。 */
  started: boolean;
  nextId: number;
}

export function initialSession(): SessionState {
  return { entries: [], pendingChoice: null, ended: false, started: false, nextId: 1 };
}

/** 结局叙事行前缀（与 kernel.ts / CLI 同源常量，避免循环依赖就地声明）。 */
const ENDING_PREFIX = "【结局";

/**
 * 事件信封落账（叙事/警告/待决选择/结局收口/错误）。
 * 读档完成按 CLI 语义先合并 envelope（narrative 覆写为「读档完成」行）
 * 再调本函数；存档/状态走 withSystemLine，不经此函数。
 */
export function applyEventEnvelope(state: SessionState, envelope: Envelope): SessionState {
  // 错误信封：错误条目 + 保留待决选择（可重试）。
  if (!envelope.ok) {
    const error = envelope.error ?? { code: "UNKNOWN", message: "（信封未携带错误详情）" };
    return {
      ...state,
      entries: [
        ...state.entries,
        { kind: "error", id: state.nextId, code: error.code, message: error.message },
      ],
      nextId: state.nextId + 1,
    };
  }

  let nextId = state.nextId;
  const entries: LogEntry[] = [];

  // 叙事行按序追加（存档/状态类信封无 narrative，天然不重放）。
  for (const line of envelope.narrative ?? []) {
    entries.push({ kind: "narrative", id: nextId++, text: line });
  }
  // D1 警告行（⚠ 前缀，人读创作侧提示）。
  for (const warning of envelope.warnings ?? []) {
    entries.push({ kind: "warning", id: nextId++, text: warning });
  }

  // 结局收口：任一叙事行带【结局前缀 → 追加收口行 + ended。
  const reachedEnding = (envelope.narrative ?? []).some((line) => line.startsWith(ENDING_PREFIX));
  if (reachedEnding) {
    entries.push({ kind: "system", id: nextId++, text: "—— 冒险结束 ——" });
  }

  return {
    entries: [...state.entries, ...entries],
    pendingChoice: reachedEnding ? null : envelope.pendingChoice ?? null,
    ended: state.ended || reachedEnding,
    started: true,
    nextId,
  };
}

/** 系统行（存档完成 / 状态诊断 / 无存档提示等不推进剧情的回执）。 */
export function withSystemLine(state: SessionState, text: string): SessionState {
  return {
    ...state,
    entries: [...state.entries, { kind: "system", id: state.nextId, text }],
    nextId: state.nextId + 1,
  };
}

/**
 * 键盘数字 → 1-based 选项序号：仅 "1".."9" 且落在选项范围内才有效，
 * 其余（0 / 越位 / 非数字 / 修饰键组合由调用方先行排除）返回 null。
 */
export function optionIndexFromKey(key: string, optionCount: number): number | null {
  if (!/^[1-9]$/.test(key)) {
    return null;
  }
  const index = Number(key);
  return index >= 1 && index <= optionCount ? index : null;
}
