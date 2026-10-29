// GeneratePanel — 「AI 生成」面板（A2 进度 + A3 流式文本预览）。
//
// 三段式：输入剧本描述 → 生成中显示进度条 + 实时预览（已到达 JSON 前缀的标题与
// 已闭合场景文案）→ 完成后给「开始游玩」链接（?mod=generated/<slug>，走既有
// mod-loader 装载路径，不新开路由）。
//
// A5 追加第四段：有了产物就挂「提出修改意见」面板（RevisePanel）——done 之后
// 针对本次 slug 迭代；?mod=generated/<slug> 进来时（initialSlug）针对 URL 里的
// 作品迭代，两处落的是同一个面板实例语义。
//
// 边界：面板只与 /__generate 的 SSE 事件面耦合（protocol.ts）；LLM 凭据只在
// vite 进程侧，浏览器侧全程不接触。run 可注入（测试用假流，不触网）。
import { useCallback, useState } from "react";
import { runGenerate } from "./client";
import type { GenerateEvent } from "./protocol";
import {
  failGenerate,
  initialGenerateState,
  reduceGenerateEvent,
  startGenerate,
  type GenerateState,
} from "./state";
import { RevisePanel } from "../revise/RevisePanel";

/** 进度条的「软满格」token 数：生成体量随故事浮动，这里只做量级参照。 */
const PROGRESS_TOKEN_HORIZON = 4000;

/** 面板注入面（测试替换成假流：不需要 dev server / 不触 LLM）。 */
export type RunGenerateStream = (
  story: string,
  onEvent: (event: GenerateEvent) => void,
) => Promise<void>;

const defaultRun: RunGenerateStream = (story, onEvent) => runGenerate(story, onEvent);

/** 进度百分比：done 才给 100，运行中封顶 99（不假称已完成）。 */
export function progressPercent(state: GenerateState): number {
  if (state.phase === "done") {
    return 100;
  }
  if (state.tokens <= 0) {
    return 0;
  }
  return Math.min(99, Math.round((state.tokens / PROGRESS_TOKEN_HORIZON) * 100));
}

export function GeneratePanel({
  run = defaultRun,
  initialSlug = null,
}: {
  run?: RunGenerateStream;
  /** ?mod=generated/<slug> 进来的作品：不必先生成就能直接提修改意见。 */
  initialSlug?: string | null;
}) {
  const [story, setStory] = useState("");
  const [state, setState] = useState<GenerateState>(initialGenerateState);
  const running = state.phase === "running";
  // 修改对象：本次生成产物优先，其次 URL 指定的既有作品（重跑期间不漂移）。
  const reviseSlug = state.phase === "done" ? state.slug : initialSlug;
  const submit = useCallback(() => {
    const text = story.trim();
    if (text.length === 0 || running) {
      return;
    }
    setState((prev) => startGenerate(prev));
    void (async () => {
      try {
        await run(text, (event) => setState((prev) => reduceGenerateEvent(prev, event)));
        // 流正常结束却没收到终态事件：不能把面板永久停在「生成中」。
        setState((prev) =>
          prev.phase === "running"
            ? failGenerate(prev, "生成流已结束，但未收到完成事件（检查 vite 终端日志）")
            : prev,
        );
      } catch (error) {
        setState((prev) =>
          failGenerate(prev, error instanceof Error ? error.message : String(error)),
        );
      }
    })();
  }, [run, running, story]);

  return (
    <main className="panel">
      <header className="panel-header">
        <h1 className="panel-title">AI 生成</h1>
        <p className="panel-subtitle">
          Yggdrasill M1 · 描述故事 → 流式生成 Mod → 直接开玩（仅本地 dev 可用）
        </p>
      </header>

      <form
        className="gen-form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="gen-label" htmlFor="gen-story">
          剧本描述
        </label>
        <textarea
          id="gen-story"
          className="gen-textarea"
          rows={3}
          value={story}
          disabled={running}
          placeholder="例：一个关于灯塔看守的悬疑短篇"
          onChange={(event) => setStory(event.target.value)}
        />
        <div className="gen-actions">
          <button
            type="submit"
            className="tool-button"
            disabled={running || story.trim().length === 0}
          >
            {running ? "生成中…" : "开始生成"}
          </button>
        </div>
      </form>

      {state.phase !== "idle" && (
        <section className="gen-progress" aria-label="生成进度">
          <div
            className="progress-track"
            role="progressbar"
            aria-label="生成进度条"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progressPercent(state)}
          >
            <div className="progress-fill" style={{ width: `${progressPercent(state)}%` }} />
          </div>
          <p className="progress-meta" aria-live="polite">
            已收到 ≈{state.tokens.toLocaleString("en-US")} tokens · 场景 ×{state.scenes}
            {state.endings > 0 ? ` · 结局 ×${state.endings}` : ""}
            {state.round > 1 ? ` · 第 ${state.round} 轮` : ""}
          </p>
        </section>
      )}

      {state.phase !== "idle" && (
        <section className="gen-preview" aria-label="生成预览" aria-busy={running}>
          <h2 className="preview-title">
            {state.title ?? (running ? "（标题解析中…）" : "（未解析出标题）")}
          </h2>
          {state.previewScenes.length > 0 && (
            <ul className="preview-list">
              {state.previewScenes.map((entry) => (
                <li key={entry.id} className="preview-item">
                  <span className="preview-id">{entry.id}</span>
                  <span className="preview-text">{entry.text}</span>
                </li>
              ))}
            </ul>
          )}
          {state.previewScenes.length === 0 && running && (
            <p className="preview-empty">（等待场景文案送达…）</p>
          )}
          {state.previewEndings.length > 0 && (
            <>
              <h3 className="preview-subtitle">结局</h3>
              <ul className="preview-list">
                {state.previewEndings.map((entry) => (
                  <li key={entry.id} className="preview-item">
                    <span className="preview-id">{entry.id}</span>
                    <span className="preview-text">{entry.text}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>
      )}

      {state.phase === "done" && state.slug !== null && (
        <p className="gen-done">
          生成完成 · 产物 packages/mods/generated/{state.slug}{" "}
          <a className="play-link" href={`?mod=generated/${state.slug}`}>
            开始游玩
          </a>
        </p>
      )}

      {reviseSlug !== null && (
        // key：换作品必须换面板实例（否则上一轮的摘要/勾选会串到新作品上）。
        <RevisePanel key={reviseSlug} slug={reviseSlug} />
      )}

      {state.phase === "error" && (
        <div className="error-box" role="alert">
          <p className="gen-error-line">
            <span className="error-code">[生成失败]</span> {state.message ?? "未知错误"}
          </p>
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

      <footer className="panel-footer">
        生成端点 /__generate 与修改端点 /__revise 仅在 vite dev 时存在（生产构建无此面）
      </footer>
    </main>
  );
}
