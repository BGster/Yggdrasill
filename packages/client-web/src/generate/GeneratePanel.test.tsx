// GeneratePanel.test.tsx — 生成面板渲染路径：输入面 / 进度条与计数 / 流式预览 /
// 完成链接 / 错误面。run 注入假流（不触网、不需要 dev server）。
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { GeneratePanel, progressPercent, type RunGenerateStream } from "./GeneratePanel";
import type { GenerateEvent } from "./protocol";
import { initialGenerateState, startGenerate } from "./state";

afterEach(() => {
  cleanup();
});

function fakeRun(events: GenerateEvent[]): RunGenerateStream {
  return async (_story, onEvent) => {
    for (const event of events) {
      onEvent(event);
    }
  };
}

function submit(story: string): void {
  fireEvent.change(screen.getByLabelText("剧本描述"), { target: { value: story } });
  fireEvent.click(screen.getByRole("button", { name: "开始生成" }));
}

describe("GeneratePanel — 输入面", () => {
  it("空描述时生成按钮禁用", () => {
    render(<GeneratePanel run={fakeRun([])} />);
    const button = screen.getByRole("button", { name: "开始生成" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("剧本描述"), { target: { value: "灯塔" } });
    expect((screen.getByRole("button", { name: "开始生成" }) as HTMLButtonElement).disabled).toBe(
      false,
    );
  });

  it("面板只渲染页脚提示，不显示进度/预览区（idle）", () => {
    render(<GeneratePanel run={fakeRun([])} />);
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.queryByLabelText("生成预览")).toBeNull();
  });
});

describe("GeneratePanel — 进度与流式预览（A2/A3）", () => {
  it("progress 事件驱动进度条与 token/场景计数", async () => {
    render(
      <GeneratePanel
        run={fakeRun([
          { type: "progress", round: 1, tokens: 1200, scenes: 0, endings: 0 },
          { type: "progress", round: 2, tokens: 8432, scenes: 3, endings: 1 },
        ])}
      />,
    );
    submit("一个关于灯塔看守的悬疑短篇");
    const meta = await screen.findByText(/已收到 ≈8,432 tokens · 场景 ×3 · 结局 ×1 · 第 2 轮/);
    expect(meta).toBeTruthy();
    const bar = screen.getByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("99"); // 运行中封顶 99
  });

  it("preview 事件实时渲染标题与已闭合场景/结局文案", async () => {
    render(
      <GeneratePanel
        run={fakeRun([
          {
            type: "preview",
            title: "暴风雨夜的灯塔",
            scenes: [
              { id: "storm", text: "暴风雨夜，灯塔的光柱在浪涛上摇晃。" },
              { id: "reef", text: "你听见礁石方向传来一声钝响。" },
            ],
            endings: [{ id: "dawn", text: "【结局晨光】信天翁绕塔三圈。" }],
          },
          { type: "progress", round: 1, tokens: 3000, scenes: 2, endings: 1 },
        ])}
      />,
    );
    submit("灯塔");
    expect(await screen.findByText("暴风雨夜的灯塔")).toBeTruthy();
    expect(screen.getByText("暴风雨夜，灯塔的光柱在浪涛上摇晃。")).toBeTruthy();
    expect(screen.getByText("你听见礁石方向传来一声钝响。")).toBeTruthy();
    expect(screen.getByText("结局")).toBeTruthy();
    expect(screen.getByText("【结局晨光】信天翁绕塔三圈。")).toBeTruthy();
  });

  it("场景尚未到达时显示等待提示（不报错）", async () => {
    render(
      <GeneratePanel
        run={fakeRun([
          {
            type: "preview",
            title: null,
            scenes: [],
            endings: [],
          },
          { type: "progress", round: 1, tokens: 40, scenes: 0, endings: 0 },
        ])}
      />,
    );
    submit("灯塔");
    expect(await screen.findByText("（标题解析中…）")).toBeTruthy();
  });
});

describe("GeneratePanel — 终态", () => {
  it("done → 「开始游玩」链接指向 ?mod=generated/<slug>，进度条满格", async () => {
    render(<GeneratePanel run={fakeRun([{ type: "done", slug: "lighthouse-keeper" }])} />);
    submit("灯塔");
    const link = (await screen.findByRole("link", { name: "开始游玩" })) as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe("?mod=generated/lighthouse-keeper");
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("100");
    expect(screen.getByText(/生成完成 · 产物 packages\/mods\/generated\/lighthouse-keeper/)).toBeTruthy();
  });

  it("error 事件 → 失败提示 + 门错误清单", async () => {
    render(
      <GeneratePanel
        run={fakeRun([
          {
            type: "error",
            message: "重试额度耗尽（最后产物保留在 packages/mods/generated/lighthouse-keeper）",
            issues: [
              {
                gate: "playtest",
                code: "PLAYTEST",
                file: "mod.lua:12",
                message: "走查未达验收锚",
              },
            ],
          },
        ])}
      />,
    );
    submit("灯塔");
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("[生成失败]");
    expect(alert.textContent).toContain("重试额度耗尽");
    expect(alert.textContent).toContain("[playtest/PLAYTEST] mod.lua:12：走查未达验收锚");
  });

  it("run 抛错 → 落失败态（不让面板停在生成中）", async () => {
    const run: RunGenerateStream = async () => {
      throw new Error("生成端点返回 HTTP 503");
    };
    render(<GeneratePanel run={run} />);
    submit("灯塔");
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("HTTP 503");
  });

  it("流正常结束但无终态事件 → 明确提示（不是永久生成中）", async () => {
    render(<GeneratePanel run={fakeRun([])} />);
    submit("灯塔");
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("未收到完成事件");
  });
});

describe("progressPercent（纯函数）", () => {
  it("idle/零 token → 0；运行中封顶 99；done → 100", () => {
    expect(progressPercent(initialGenerateState())).toBe(0);
    const running = startGenerate(initialGenerateState());
    expect(progressPercent({ ...running, tokens: 0 })).toBe(0);
    expect(progressPercent({ ...running, tokens: 1000 })).toBe(25);
    expect(progressPercent({ ...running, tokens: 99_999 })).toBe(99);
    expect(progressPercent({ ...running, phase: "done", tokens: 1000 })).toBe(100);
  });
});
