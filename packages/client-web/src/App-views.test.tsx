// App-views.test.tsx — 顶层视图切换（A2）：?gen=1 直达生成面；两面对话都保状态。
// 说明：本测试只覆盖导航面；内核/生成面板各自的行为在各自测试里（wasm 在 vitest
// 下走 stub → 自动降级回放，确定性不变）。
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import App, { viewFromSearch } from "./App";

afterEach(() => {
  cleanup();
  window.history.pushState({}, "", "/");
});

describe("viewFromSearch（URL 视图参数）", () => {
  it("默认游玩；?gen / ?generate 走生成面；?mod= 仍是游玩面", () => {
    expect(viewFromSearch("")).toBe("play");
    expect(viewFromSearch("?mod=generated/lighthouse-keeper")).toBe("play");
    expect(viewFromSearch("?gen=1")).toBe("generate");
    expect(viewFromSearch("?generate")).toBe("generate");
  });
});

describe("App 视图切换", () => {
  it("切到生成面：游戏面隐藏但仍在树上；切回来进度未丢", async () => {
    render(<App />);
    expect(await screen.findByText(/序章·雨夜/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "AI 生成" }));
    expect(screen.getByLabelText("剧本描述")).toBeTruthy();
    // hidden 的游玩面：role 查询默认排除不可见元素。
    await waitFor(() => expect(screen.queryByRole("log")).toBeNull());

    fireEvent.click(screen.getByRole("button", { name: "游玩" }));
    expect(await screen.findByRole("log")).toBeTruthy();
    expect(screen.getByText(/序章·雨夜/)).toBeTruthy(); // 没有重新开局的迹象
  });

  it("?gen=1 直达生成面（首屏不挂游戏面）", async () => {
    window.history.pushState({}, "", "/?gen=1");
    render(<App />);
    expect(await screen.findByLabelText("剧本描述")).toBeTruthy();
    expect(screen.queryByRole("log")).toBeNull();
  });
});
