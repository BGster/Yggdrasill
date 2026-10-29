// RevisePanel — 「提出修改意见」面板（A5）：revise 的浏览器闭环。
//
// 循环语义：提交意见 → loading → 变更摘要（或可操作的拒绝提示）→ 改完可以
// 继续提下一条意见（同一面板循环，不发新导航）。成功的唯一「看效果」方式是
// 重新加载游玩页（?mod=generated/<slug>）——内核实例与存档都绑在那个页面上，
// 这里不假装能热替换已装载的 Mod。
//
// 边界：面板只与 /__revise 的 JSON 契约耦合（protocol.ts）；凭据只在 vite 进程侧。
// run 可注入（测试用假实现，不触网）。
import { useCallback, useState } from "react";
import { runRevise } from "./client";
import { NOOP_HINT, reviseHints } from "./messages";
import type { ReviseRequest, ReviseResponse } from "./protocol";
import {
  applyReviseResponse,
  failRevise,
  initialReviseState,
  startRevise,
  type ReviseState,
} from "./state";

/** 面板注入面（测试替换成假实现：不需要 dev server / 不触 LLM）。 */
export type RunReviseCall = (request: ReviseRequest) => Promise<ReviseResponse>;

const defaultRun: RunReviseCall = (request) => runRevise(request);

/** 变更摘要的一行文案：「变更 2 处：a、b」/「无变更」。 */
export function describeChanges(changes: string[]): string {
  if (changes.length === 0) {
    return "无变更";
  }
  return `变更 ${changes.length} 处：${changes.join("、")}`;
}

export function RevisePanel({
  slug,
  run = defaultRun,
}: {
  /** 要修改的作品 slug（生成完成后为本次产物；?mod= 入口为 URL 里的作品）。 */
  slug: string;
  run?: RunReviseCall;
}) {
  const [feedback, setFeedback] = useState("");
  const [allowLua, setAllowLua] = useState(false);
  const [state, setState] = useState<ReviseState>(initialReviseState);
  const running = state.phase === "running";
  const hints = reviseHints(state.issues);

  const submit = useCallback(() => {
    const text = feedback.trim();
    if (text.length === 0 || running) {
      return;
    }
    setState((prev) => startRevise(prev));
    void (async () => {
      try {
        const response = await run({ slug, feedback: text, allowLua });
        setState((prev) => applyReviseResponse(prev, response));
        // 成功后清空输入面：下一轮是一条新意见（拒绝时保留原文与勾选，
        // 用户通常只需勾上授权再提交一次）。
        if (response.ok) {
          setFeedback("");
        }
      } catch (error) {
        setState((prev) =>
          failRevise(prev, error instanceof Error ? error.message : String(error)),
        );
      }
    })();
  }, [allowLua, feedback, run, running, slug]);

  return (
    <section className="revise-panel" aria-label="修改作品">
      <h2 className="revise-title">提出修改意见</h2>
      <p className="revise-subtitle">
        当前作品 <code className="revise-slug">{slug}</code>{" "}
        · 只改文案与数值；结构与 Lua 逻辑默认冻结（勾选下方选项可授权）
      </p>

      <form
        className="gen-form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="gen-label" htmlFor="revise-feedback">
          修改意见
        </label>
        <textarea
          id="revise-feedback"
          className="gen-textarea"
          rows={2}
          value={feedback}
          disabled={running}
          placeholder="例：把开场那句风雨写得更急一些"
          onChange={(event) => setFeedback(event.target.value)}
        />
        <label className="revise-check" htmlFor="revise-allow-lua">
          <input
            id="revise-allow-lua"
            type="checkbox"
            checked={allowLua}
            disabled={running}
            onChange={(event) => setAllowLua(event.target.checked)}
          />
          允许修改 Lua 逻辑（同时放行场景/结局键集增删）
        </label>
        <div className="gen-actions">
          <button
            type="submit"
            className="tool-button"
            disabled={running || feedback.trim().length === 0}
          >
            {running ? "修改中…（三道门校验需数秒）" : state.rounds > 0 ? "再提一条意见" : "提交修改"}
          </button>
        </div>
      </form>

      {state.phase === "done" && (
        <div className="revise-result" role="status">
          <p className="revise-summary">
            {state.rounds > 1 ? `第 ${state.rounds} 轮修改完成 · ` : "修改完成 · "}
            {describeChanges(state.changes)}
          </p>
          {state.noop ? (
            <p className="revise-hint">{NOOP_HINT}</p>
          ) : (
            <p className="revise-meta">
              <a className="play-link" href={`?mod=generated/${slug}`}>
                重新游玩
              </a>
              （页面重载后装载新版本；存档仍绑在同一作品上）
            </p>
          )}
        </div>
      )}

      {state.phase === "error" && (
        <div className="error-box" role="alert">
          <p className="gen-error-line">
            <span className="error-code">[修改失败]</span>{" "}
            {state.message ?? "本轮修改未通过，磁盘仍为修改前的版本"}
          </p>
          {hints.length > 0 && (
            <ul className="revise-hints">
              {hints.map((hint) => (
                <li key={hint}>{hint}</li>
              ))}
            </ul>
          )}
          {state.issues.length > 0 && (
            <ul className="gen-issues">
              {state.issues.map((issue, index) => (
                <li key={`${issue.gate}-${issue.code}-${index}`}>
                  [{issue.gate}/{issue.code}] {issue.file}：{issue.message}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
