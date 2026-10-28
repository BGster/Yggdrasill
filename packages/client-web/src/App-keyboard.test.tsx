// 键盘快捷键映射测试：1-9 → ygg.choice_selected payload；越界/修饰键不派发。
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { GamePanel } from "./App";
import { ReplayKernel } from "./kernel";

afterEach(() => {
  cleanup(); // vitest 未开 globals：RTL 自动清理不生效，显式卸载。
});

/** 记录派发调用的 spy 内核（真实回放语义 + 调用留痕）。 */
class SpyKernel extends ReplayKernel {
  calls: Array<[cmd: string, payload: string]> = [];

  override dispatch(cmd: string, payload: string): ReturnType<ReplayKernel["dispatch"]> {
    this.calls.push([cmd, payload]);
    return super.dispatch(cmd, payload);
  }
}

function keyDown(key: string, modifiers: { ctrl?: boolean } = {}): void {
  fireEvent.keyDown(window, { key, ctrlKey: modifiers.ctrl ?? false });
}

describe("键盘快捷键（1-9 → 待决选择）", () => {
  it("按 2 派发 payload '2'（走分支 b），面板更新为岔路面", () => {
    const kernel = new SpyKernel();
    render(<GamePanel kernel={kernel} onRestart={() => {}} />);
    keyDown("2");
    expect(kernel.calls).toContainEqual(["ygg.choice_selected", "2"]);
    expect(screen.getByText(/你谢绝了提灯/)).toBeTruthy();
  });

  it("按 1 派发 payload '1'（走分支 a）", () => {
    const kernel = new SpyKernel();
    render(<GamePanel kernel={kernel} onRestart={() => {}} />);
    keyDown("1");
    expect(kernel.calls).toContainEqual(["ygg.choice_selected", "1"]);
    expect(screen.getByText(/你接过提灯/)).toBeTruthy();
  });

  it("越界数字（选项只有 2 个）不派发", () => {
    const kernel = new SpyKernel();
    render(<GamePanel kernel={kernel} onRestart={() => {}} />);
    keyDown("3");
    expect(kernel.calls).toHaveLength(0);
  });

  it("非数字键不派发", () => {
    const kernel = new SpyKernel();
    render(<GamePanel kernel={kernel} onRestart={() => {}} />);
    keyDown("Enter");
    keyDown("a");
    expect(kernel.calls).toHaveLength(0);
  });

  it("修饰键组合（Ctrl+1）不派发", () => {
    const kernel = new SpyKernel();
    render(<GamePanel kernel={kernel} onRestart={() => {}} />);
    keyDown("1", { ctrl: true });
    expect(kernel.calls).toHaveLength(0);
  });

  it("结局后数字键不派发（选择面收口）", () => {
    const kernel = new SpyKernel();
    render(<GamePanel kernel={kernel} onRestart={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "选项 1：接过提灯" }));
    fireEvent.click(screen.getByRole("button", { name: "选项 1：沿石阶下地窖" }));
    const choiceCalls = kernel.calls.filter(([cmd]) => cmd === "ygg.choice_selected");
    expect(choiceCalls).toHaveLength(2);
    keyDown("1");
    expect(kernel.calls.filter(([cmd]) => cmd === "ygg.choice_selected")).toHaveLength(2);
  });
});
