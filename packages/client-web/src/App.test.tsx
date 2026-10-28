// UI 渲染路径测试：挂载序章、分支选择流、存读档按钮（含错误信封呈现）。
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { GamePanel, SAVE_STORAGE_KEY } from "./App";
import { ReplayKernel } from "./kernel";

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  cleanup(); // vitest 未开 globals：RTL 自动清理不生效，显式卸载。
  localStorage.clear();
});

function renderPanel(kernel = new ReplayKernel()) {
  return render(<GamePanel kernel={kernel} onRestart={() => {}} />);
}

describe("GamePanel 渲染路径", () => {
  it("挂载即派发序章：叙事可见 + 酒馆选择面两个按钮", () => {
    renderPanel();
    expect(screen.getByText(/序章·雨夜/)).toBeTruthy();
    expect(screen.getByText(/酒馆·大堂/)).toBeTruthy();
    expect(screen.getByText(/掌柜把铜提灯推向你/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "选项 1：接过提灯" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "选项 2：轻装出发" })).toBeTruthy();
  });

  it("点击选项 2 走分支 b：轻装出发节拍 + 岔路选择面", () => {
    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "选项 2：轻装出发" }));
    expect(screen.getByText(/你谢绝了提灯/)).toBeTruthy();
    expect(screen.getByText(/后廊岔路——你的脚步停在哪里/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "选项 1：沿石阶下地窖" })).toBeTruthy();
  });

  it("走完分支 a 到结局：结局行 + 收口行 + 选择面消失 + 选项禁用路径", () => {
    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "选项 1：接过提灯" }));
    fireEvent.click(screen.getByRole("button", { name: "选项 1：沿石阶下地窖" }));
    expect(screen.getByText(/【结局A】灯下秘藏/)).toBeTruthy();
    expect(screen.getByText("—— 冒险结束 ——")).toBeTruthy();
    expect(screen.queryByText(/后廊岔路/)).toBeNull();
  });

  it("存档按钮：写入 localStorage（ygg-save）并显示系统回执", () => {
    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "存档" }));
    expect(localStorage.getItem(SAVE_STORAGE_KEY)).toBeTruthy();
    expect(screen.getByText(/已存档 → 浏览器 localStorage/)).toBeTruthy();
  });

  it("读档往返：存档 → 读档 → 读档完成 + 进度恢复", () => {
    const { unmount } = renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "选项 2：轻装出发" }));
    fireEvent.click(screen.getByRole("button", { name: "存档" }));
    unmount();

    renderPanel(new ReplayKernel());
    // 新实例停在序章；读档恢复到岔路（分支 b 中盘）。
    fireEvent.click(screen.getByRole("button", { name: "读档" }));
    expect(screen.getByText(/（读档完成，进度已恢复。）/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "选项 2：上楼回客房" })).toBeTruthy();
  });

  it("无存档时读档：系统提示行（不报错）", () => {
    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "读档" }));
    expect(screen.getByText(/（无存档。先按「存档」保存进度。）/)).toBeTruthy();
  });

  it("错误信封呈现路径：损坏存档 → 红色错误框（码 + 文案）", () => {
    localStorage.setItem(SAVE_STORAGE_KEY, "not json");
    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "读档" }));
    const alertBox = screen.getByRole("alert");
    expect(alertBox.textContent).toContain("[错误 VALIDATION]");
    expect(alertBox.textContent).toContain("load rejected");
  });

  it("状态按钮：诊断面系统行（engine/revision/snapshot）", () => {
    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "状态" }));
    expect(screen.getByText(/状态 engine=0\.1\.0 revision=5/)).toBeTruthy();
  });
});
