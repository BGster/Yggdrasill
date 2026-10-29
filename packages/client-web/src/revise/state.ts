// state.ts — 修改面板的纯状态机（A5）：把一次 /__revise 往返折叠成界面状态。
//
// 抽成纯函数的理由与生成面板同源：连续迭代（改一轮 → 再提一条意见 → 再改）会
// 让「上一轮的变更摘要/错误该不该留着」变成容易写错的时序问题；这里把规则固定
// 成三条可单测的转换：
//   startRevise     → 清结果面、保留轮次（起跑不残留上一轮摘要）
//   applyReviseResponse → 成功/被拒两种终态（含 rounds 递增）
//   failRevise      → 端点/网络层错误（不递增轮次——本轮没落地）
import type { ReviseIssueView, ReviseResponse } from "./protocol";

export type RevisePhase = "idle" | "running" | "done" | "error";

export interface ReviseState {
  phase: RevisePhase;
  /** 最近一轮的变更路径清单（成功面）。 */
  changes: string[];
  /** 空 diff：模型认为无需修改（成功但零变更）。 */
  noop: boolean;
  /** 端点/网络层错误文案（门/白名单拒绝走 issues，不走这里）。 */
  message: string | null;
  /** 门/白名单诊断（失败面）。 */
  issues: ReviseIssueView[];
  /** 已成功落盘的轮次数（连续迭代的可见进度）。 */
  rounds: number;
}

export function initialReviseState(): ReviseState {
  return { phase: "idle", changes: [], noop: false, message: null, issues: [], rounds: 0 };
}

/** 起跑：清掉上一轮的结果面，保留 rounds（轮次是跨轮累计的事实）。 */
export function startRevise(state: ReviseState): ReviseState {
  return { ...initialReviseState(), phase: "running", rounds: state.rounds };
}

/** 客户端侧异常（fetch 失败 / 非 2xx / 响应形状不符）：落失败态，轮次不变。 */
export function failRevise(state: ReviseState, message: string): ReviseState {
  return { ...state, phase: "error", message, issues: [], changes: [], noop: false };
}

/**
 * 折叠一次端点响应。ok:false 是**正常业务结果**（白名单/门拒绝）：它带诊断，
 * 不填 message——面板据此展示可操作提示而不是一句「端点错误」。
 */
export function applyReviseResponse(state: ReviseState, response: ReviseResponse): ReviseState {
  if (response.ok) {
    return {
      ...state,
      phase: "done",
      changes: [...response.changes],
      noop: response.noop,
      message: null,
      issues: [],
      rounds: state.rounds + 1,
    };
  }
  return {
    ...state,
    phase: "error",
    changes: [],
    noop: false,
    message: null,
    issues: [...response.issues],
  };
}
