// RevisePanel.test.tsx — 修改面板渲染路径：提交 → 变更摘要 / 无需修改 / 白名单
// 拒绝的友好提示 / 端点错误 / 连续迭代 / 授权勾选透传。run 注入假实现（不触网）。
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { describeChanges, RevisePanel, type RunReviseCall } from "./RevisePanel";
import type { ReviseRequest, ReviseResponse } from "./protocol";

afterEach(() => {
  cleanup();
});

/** 依次返回给定响应的假端点；同时记录每次请求体（断言 allowLua 透传）。 */
function queueRun(responses: ReviseResponse[], calls: ReviseRequest[] = []): RunReviseCall {
  let index = 0;
  return async (request) => {
    calls.push(request);
    const response = responses[Math.min(index, responses.length - 1)];
    index += 1;
    return response!;
  };
}

function typeFeedback(text: string): void {
  fireEvent.change(screen.getByLabelText("修改意见"), { target: { value: text } });
}

function submit(): void {
  fireEvent.click(screen.getByRole("button", { name: /提交修改|再提一条意见/ }));
}

const SUCCESS: ReviseResponse = {
  ok: true,
  slug: "lighthouse-keeper",
  changes: ["scenes.scenes.storm", "scenes.endings.dawn"],
  noop: false,
};

describe("describeChanges（纯函数）", () => {
  it("零变更为「无变更」；多变更用「、」连接", () => {
    expect(describeChanges([])).toBe("无变更");
    expect(describeChanges(["scenes.scenes.storm"])).toBe("变更 1 处：scenes.scenes.storm");
    expect(describeChanges(["a", "b"])).toBe("变更 2 处：a、b");
  });
});

describe("RevisePanel — 输入面", () => {
  it("空意见时提交按钮禁用；idle 不显示结果/错误区", () => {
    render(<RevisePanel slug="x1" run={queueRun([SUCCESS])} />);
    const button = screen.getByRole("button", { name: /提交修改/ }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
    typeFeedback("改一句");
    expect((screen.getByRole("button", { name: /提交修改/ }) as HTMLButtonElement).disabled).toBe(
      false,
    );
  });

  it("面板显示目标 slug 与授权勾选", () => {
    render(<RevisePanel slug="lighthouse-keeper" run={queueRun([SUCCESS])} />);
    expect(screen.getByText("lighthouse-keeper")).toBeTruthy();
    const checkbox = screen.getByLabelText(/允许修改 Lua 逻辑/) as HTMLInputElement;
    expect(checkbox.checked).toBe(false);
  });
});

describe("RevisePanel — 成功面", () => {
  it("提交 → 变更路径摘要 + 「重新游玩」链接指向 ?mod=generated/<slug>", async () => {
    render(<RevisePanel slug="lighthouse-keeper" run={queueRun([SUCCESS])} />);
    typeFeedback("把开场的风雨写得更急");
    submit();
    expect(await screen.findByText(/变更 2 处：scenes\.scenes\.storm、scenes\.endings\.dawn/)).toBeTruthy();
    const link = screen.getByRole("link", { name: "重新游玩" }) as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe("?mod=generated/lighthouse-keeper");
  });

  it("成功请求体带 slug / 意见 / allowLua（勾选后为 true）", async () => {
    const calls: ReviseRequest[] = [];
    render(<RevisePanel slug="x1" run={queueRun([SUCCESS], calls)} />);
    typeFeedback("改一句");
    fireEvent.click(screen.getByLabelText(/允许修改 Lua 逻辑/));
    submit();
    await screen.findByText(/变更 2 处/);
    expect(calls).toEqual([{ slug: "x1", feedback: "改一句", allowLua: true }]);
  });

  it("成功后清空输入框（下一轮是一条新意见）", async () => {
    render(<RevisePanel slug="x1" run={queueRun([SUCCESS])} />);
    typeFeedback("改一句");
    submit();
    await screen.findByText(/变更 2 处/);
    expect((screen.getByLabelText("修改意见") as HTMLTextAreaElement).value).toBe("");
  });

  it("noop（空 diff）→ 「模型认为无需修改」提示，不给重新游玩链接", async () => {
    render(
      <RevisePanel
        slug="x1"
        run={queueRun([{ ok: true, slug: "x1", changes: [], noop: true }])}
      />,
    );
    typeFeedback("随便改改");
    submit();
    expect(await screen.findByText(/模型认为无需修改/)).toBeTruthy();
    expect(screen.getByText(/修改完成 · 无变更/)).toBeTruthy();
    expect(screen.queryByRole("link", { name: "重新游玩" })).toBeNull();
  });

  it("连续迭代：第二轮显示轮次且摘要更新", async () => {
    const run = queueRun([
      { ok: true, slug: "x1", changes: ["scenes.scenes.storm"], noop: false },
      { ok: true, slug: "x1", changes: ["scenes.endings.dawn"], noop: false },
    ]);
    render(<RevisePanel slug="x1" run={run} />);
    typeFeedback("第一轮意见");
    submit();
    expect(await screen.findByText(/变更 1 处：scenes\.scenes\.storm/)).toBeTruthy();
    expect(screen.queryByText(/第 2 轮/)).toBeNull();

    typeFeedback("第二轮意见");
    submit();
    expect(await screen.findByText(/第 2 轮修改完成 · 变更 1 处：scenes\.endings\.dawn/)).toBeTruthy();
  });
});

describe("RevisePanel — 拒绝面", () => {
  it("REVISE_LUA_FROZEN → 「勾选允许修改 Lua 后重试」+ 原始诊断行", async () => {
    render(
      <RevisePanel
        slug="x1"
        run={queueRun([
          {
            ok: false,
            issues: [
              {
                gate: "revise",
                code: "REVISE_LUA_FROZEN",
                file: "mod.lua",
                message: "mod.lua 被改动，但本轮未授权 Lua 变更",
              },
            ],
          },
        ])}
      />,
    );
    typeFeedback("让灯油消耗更快");
    submit();
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("[修改失败]");
    expect(alert.textContent).toContain("勾选「允许修改 Lua」后重试");
    expect(alert.textContent).toContain("[revise/REVISE_LUA_FROZEN] mod.lua");
    // 被拒后输入框保留原文（通常只需勾上授权再提交一次）
    expect((screen.getByLabelText("修改意见") as HTMLTextAreaElement).value).toBe("让灯油消耗更快");
  });

  it("REVISE_STRUCTURE 与同码多条 → 提示去重，诊断行全留", async () => {
    render(
      <RevisePanel
        slug="x1"
        run={queueRun([
          {
            ok: false,
            issues: [
              { gate: "revise", code: "REVISE_STRUCTURE", file: "data/scenes.json", message: "键集增删 A" },
              { gate: "revise", code: "REVISE_STRUCTURE", file: "data/scenes.json", message: "键集增删 B" },
            ],
          },
        ])}
      />,
    );
    typeFeedback("加一个新结局");
    submit();
    const alert = await screen.findByRole("alert");
    const hints = alert.querySelector(".revise-hints");
    expect(hints?.querySelectorAll("li")).toHaveLength(1);
    expect(hints?.textContent).toContain("勾选「允许修改 Lua」后重试");
    expect(alert.querySelectorAll(".gen-issues li")).toHaveLength(2);
  });

  it("门错误（非 REVISE_*）→ 只展示原始诊断，不编造提示", async () => {
    render(
      <RevisePanel
        slug="x1"
        run={queueRun([
          {
            ok: false,
            issues: [{ gate: "playtest", code: "PLAYTEST", file: "mod.lua:12", message: "走查未达验收锚" }],
          },
        ])}
      />,
    );
    typeFeedback("改结局");
    submit();
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("[playtest/PLAYTEST] mod.lua:12");
    expect(alert.querySelector(".revise-hints")).toBeNull();
    // 未通过校验的默认文案
    expect(alert.textContent).toContain("磁盘仍为修改前的版本");
  });

  it("run 抛错（端点 5xx）→ [修改失败] + 错误文案", async () => {
    const run: RunReviseCall = async () => {
      throw new Error("缺少 LLM 环境变量：YGG_LLM_API_KEY");
    };
    render(<RevisePanel slug="x1" run={run} />);
    typeFeedback("改");
    submit();
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("缺少 LLM 环境变量");
  });
});

describe("RevisePanel — running 态", () => {
  it("提交中禁用输入与按钮，按钮文案变为进行中", async () => {
    let release: ((response: ReviseResponse) => void) | null = null;
    const run: RunReviseCall = () =>
      new Promise<ReviseResponse>((resolve) => {
        release = resolve;
      });
    render(<RevisePanel slug="x1" run={run} />);
    typeFeedback("改一句");
    submit();
    const button = screen.getByRole("button", { name: /修改中/ }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect((screen.getByLabelText("修改意见") as HTMLTextAreaElement).disabled).toBe(true);
    release!(SUCCESS);
    await waitFor(() => expect(screen.getByText(/变更 2 处/)).toBeTruthy());
  });
});
