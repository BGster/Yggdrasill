// App — 游戏面板容器：持有内核实例与会话状态，串起日志区 / 选择面 / 工具栏。
//
// 内核源选择（B3）：默认真实 wasm（WasmKernel，异步装载）；装载失败降级
// ReplayKernel 并落可见横幅（「wasm 不可用，回放模式」）；URL `?mock=1`
// 强制回放（离线开发 / fixture 测试保持确定性）。
//
// 重开语义：新 KernelBridge 实例 + key 变更整树重挂（两种内核同为全新
// 实例——wasm 侧重新装载 Mod，回放侧重放首屏）。
import { useCallback, useEffect, useRef, useState } from "react";
import { GeneratePanel } from "./generate/GeneratePanel";
import { ReplayKernel, type Envelope, type KernelBridge } from "./kernel";
import { createWasmKernel, describeInitError } from "./kernel-wasm";
import { GeneratedModError, loadGeneratedBundle } from "./mod-loader";
import {
  applyEventEnvelope,
  initialSession,
  optionIndexFromKey,
  withSystemLine,
  type SessionState,
} from "./session";
import { ChoiceList } from "./ui/ChoiceList";
import { NarrativeLog } from "./ui/NarrativeLog";
import { Toolbar } from "./ui/Toolbar";

/** localStorage 存档键（存档本体 = save 信封 save 字段的 JSON 原文）。 */
export const SAVE_STORAGE_KEY = "ygg-save";

export function GamePanel({
  kernel,
  mode = "replay",
  onRestart,
}: {
  kernel: KernelBridge;
  /** 内核源标签（副标题 / 页脚文案；测试直接挂 GamePanel 时缺省回放面）。 */
  mode?: "wasm" | "replay";
  onRestart: () => void;
}) {
  const [session, setSession] = useState<SessionState>(initialSession);
  const bootedFor = useRef<KernelBridge | null>(null);

  // 开机：每内核实例只派发一次序章（ref 门，StrictMode 双调安全）。
  useEffect(() => {
    if (bootedFor.current === kernel) {
      return;
    }
    bootedFor.current = kernel;
    setSession(applyEventEnvelope(initialSession(), kernel.start()));
  }, [kernel]);

  const choose = useCallback(
    (index: number) => {
      setSession((prev) => {
        if (!prev.pendingChoice || prev.ended) {
          return prev;
        }
        return applyEventEnvelope(prev, kernel.dispatch("ygg.choice_selected", String(index)));
      });
    },
    [kernel],
  );

  const onSave = useCallback(() => {
    const reply = kernel.save();
    if (reply.ok && reply.save) {
      localStorage.setItem(SAVE_STORAGE_KEY, JSON.stringify(reply.save));
      setSession((prev) => withSystemLine(prev, "已存档 → 浏览器 localStorage（ygg-save）"));
    } else {
      setSession((prev) => applyEventEnvelope(prev, reply));
    }
  }, [kernel]);

  const onLoad = useCallback(() => {
    const raw = localStorage.getItem(SAVE_STORAGE_KEY);
    if (raw === null) {
      setSession((prev) => withSystemLine(prev, "（无存档。先按「存档」保存进度。）"));
      return;
    }
    const reply = kernel.load(raw);
    if (reply.ok) {
      // CLI 同款合并：load 信封只带 restored/pendingChoice，叙事行由驱动补。
      const merged: Envelope = { ...reply, narrative: ["（读档完成，进度已恢复。）"] };
      setSession((prev) => applyEventEnvelope(prev, merged));
    } else {
      setSession((prev) => applyEventEnvelope(prev, reply));
    }
  }, [kernel]);

  const onState = useCallback(() => {
    const reply = kernel.dispatch("state", "");
    if (reply.ok) {
      const pending = reply.pendingChoice ? `「${reply.pendingChoice.prompt}」` : "无";
      setSession((prev) =>
        withSystemLine(
          prev,
          `状态 engine=${reply.engineVersion} revision=${reply.revision} ` +
            `kvRevision=${reply.kvRevision} snapshotBytes=${reply.snapshotBytes} ` +
            `pendingChoice=${pending}`,
        ),
      );
    } else {
      setSession((prev) => applyEventEnvelope(prev, reply));
    }
  }, [kernel]);

  // 键盘 1-9 → 待决选择（仅无修饰键；有选择且未结局时生效）。
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey) {
        return;
      }
      const index = optionIndexFromKey(e.key, session.pendingChoice?.options.length ?? 0);
      if (index !== null && !session.ended) {
        choose(index);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [choose, session.pendingChoice, session.ended]);

  return (
    <main className="panel">
      <header className="panel-header">
        <h1 className="panel-title">灯下谜径</h1>
        <p className="panel-subtitle">
          Yggdrasill M0 · 叙事面板（{mode === "wasm" ? "wasm 内核" : "回放内核"}）
        </p>
        <Toolbar
          onStartable={session.started}
          onSave={onSave}
          onLoad={onLoad}
          onState={onState}
          onRestart={onRestart}
        />
      </header>
      <NarrativeLog entries={session.entries} />
      {session.pendingChoice && !session.ended && (
        <ChoiceList choice={session.pendingChoice} ended={session.ended} onSelect={choose} />
      )}
      <footer className="panel-footer">
        {mode === "wasm"
          ? "内核：真实 wasm 绑定 · 资源包 packages/mods/test-adventure"
          : "回放源 fixtures/adventure-transcript.json（?mock=1 强制回放）"}
      </footer>
    </main>
  );
}

// ── 内核源选择（wasm 默认 / 降级回放 / ?mock=1 强制 / ?mod= 生成 Mod） ──

// ── ?mod=generated/<slug> 解析（?mock=1 同型 URL 参数模式，Architecture
// Insight 5：不引入路由库）──────────────────────────────

/** URL 生成 Mod 参数（?mod=generated/<slug>）；无参数 / 形态不符 → null。 */
export function generatedModSlug(search: string): string | null {
  const value = new URLSearchParams(search).get("mod");
  if (value === null || !value.startsWith("generated/")) {
    return null;
  }
  const slug = value.slice("generated/".length);
  return slug.length > 0 ? slug : null;
}

/** 内核装载状态机：装载中 → 就绪（wasm 或降级回放）。 */
type KernelSource =
  | { phase: "loading" }
  | {
      phase: "ready";
      kernel: KernelBridge;
      mode: "wasm" | "replay";
      /** wasm 装载失败降级（App 落可见横幅）。 */
      degraded: boolean;
      /** 降级原因文案（describeInitError 映射，横幅附注）。 */
      detail: string | null;
      /** 生成 Mod 回退等非致命提示（notice 横幅；degraded/detail 语义不变）。 */
      notice: string | null;
    };

/** URL 是否强制回放（`?mock=1`；有参数即命中，不校验取值）。 */
export function isMockForced(search: string): boolean {
  return new URLSearchParams(search).has("mock");
}

function replaySource(degraded: boolean, detail: string | null): KernelSource {
  return {
    phase: "ready",
    kernel: new ReplayKernel(),
    mode: "replay",
    degraded,
    detail,
    notice: null,
  };
}

/**
 * 内核源状态机 + 重开动作。装载副作用：wasm 装载成功 → wasm 源；任一环
 * 节失败（胶水缺失 / 实例化 / 构造域失败）→ 回放降级 + 横幅。取消闸：
 * 卸载 / 重开后丢弃过期回包（重开重试 wasm——环境可能已修复，失败快）。
 */
function useKernelSource(): {
  source: KernelSource;
  generation: number;
  restart: () => void;
} {
  const [generation, setGeneration] = useState(0);
  // 生成 Mod slug 取一次（重开不重读 URL——语义 = 同一目标重试）。
  const [modSlug] = useState(() => generatedModSlug(window.location.search));
  const [source, setSource] = useState<KernelSource>(initialSource);

  useEffect(() => {
    if (source.phase !== "loading") {
      return;
    }
    let cancelled = false;
    const boot = (async () => {
      if (modSlug !== null) {
        try {
          const bundle = await loadGeneratedBundle(modSlug);
          return {
            kernel: await createWasmKernel(bundle),
            degraded: false,
            detail: null,
            notice: null,
          };
        } catch (error) {
          const message =
            error instanceof GeneratedModError
              ? `生成 Mod「${modSlug}」不可用：${(error as Error).message}——已回退默认冒险`
              : `生成 Mod「${modSlug}」载入失败：${describeInitError(error)}——已回退默认冒险`;
          return { kernel: await createWasmKernel(), degraded: false, detail: null, notice: message };
        }
      }
      return { kernel: await createWasmKernel(), degraded: false, detail: null, notice: null };
    })();
    boot
      .then((ready) => {
        if (!cancelled) {
          setSource({ phase: "ready", mode: "wasm", ...ready });
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setSource(replaySource(true, describeInitError(err)));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [source]);

  const restart = useCallback(() => {
    setGeneration((g) => g + 1);
    // 重开：mock 强制 → 直接新回放实例；否则回到装载态重试 wasm。
    setSource(initialSource());
  }, []);

  return { source, generation, restart };
}

/** 初始源：mock 强制（?mock=1）→ 确定性回放；否则 wasm 装载态。 */
function initialSource(): KernelSource {
  return isMockForced(window.location.search) ? replaySource(false, null) : { phase: "loading" };
}

/** 顶层视图：游戏面板 / AI 生成面板（?gen=1 或 ?generate=1 直达后者）。 */
export type AppView = "play" | "generate";

/** URL 视图参数解析（纯函数，单测锚）：`?gen` / `?generate` 有参即生成面。 */
export function viewFromSearch(search: string): AppView {
  const params = new URLSearchParams(search);
  return params.has("gen") || params.has("generate") ? "generate" : "play";
}

/**
 * 游戏面：内核源状态机 + 降级横幅 + 叙事面板。
 * 从 App 拆出（A2）：生成面不该在挂载时启动 wasm 装载；拆开后两个视图各自
 * 只承担自己的副作用。
 */
function PlaySurface() {
  const { source, generation, restart } = useKernelSource();

  if (source.phase === "loading") {
    return (
      <main className="panel">
        <h1 className="panel-title">灯下谜径</h1>
        <p className="panel-subtitle">内核装载中…（wasm 绑定初始化）</p>
      </main>
    );
  }

  return (
    <>
      {source.degraded && (
        <div className="mode-banner" role="status">
          wasm 不可用，回放模式
          {source.detail ? `：${source.detail}` : ""}
        </div>
      )}
      {source.phase === "ready" && source.notice !== null && (
        <div className="mode-banner" role="status">
          {source.notice}
        </div>
      )}
      <GamePanel key={generation} kernel={source.kernel} mode={source.mode} onRestart={restart} />
    </>
  );
}

export default function App() {
  const [view, setView] = useState<AppView>(() => viewFromSearch(window.location.search));
  // 已挂载过的视图：面切换后保留在树上（hidden）——否则回生成面再回来，
  // 游戏会话/生成结果全丢（重开语义不该由导航承担）。
  const [mountedViews, setMountedViews] = useState<ReadonlySet<AppView>>(
    () => new Set<AppView>([viewFromSearch(window.location.search)]),
  );
  useEffect(() => {
    setMountedViews((previous) =>
      previous.has(view) ? previous : new Set([...previous, view]),
    );
  }, [view]);

  return (
    <>
      <nav className="view-tabs" aria-label="视图切换">
        <button
          type="button"
          className={`view-tab${view === "play" ? " view-tab-active" : ""}`}
          aria-pressed={view === "play"}
          onClick={() => setView("play")}
        >
          游玩
        </button>
        <button
          type="button"
          className={`view-tab${view === "generate" ? " view-tab-active" : ""}`}
          aria-pressed={view === "generate"}
          onClick={() => setView("generate")}
        >
          AI 生成
        </button>
      </nav>
      {mountedViews.has("play") && (
        <div className="view-surface" hidden={view !== "play"}>
          <PlaySurface />
        </div>
      )}
      {mountedViews.has("generate") && (
        <div className="view-surface" hidden={view !== "generate"}>
          <GeneratePanel />
        </div>
      )}
    </>
  );
}
