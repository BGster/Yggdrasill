// 警告（D1 warnings）显示路径：stub 内核回放真实内核警告样本信封。
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { GamePanel } from "./App";
import type { Envelope, KernelBridge } from "./kernel";
import transcriptJson from "./fixtures/adventure-transcript.json";

afterEach(() => {
  cleanup(); // vitest 未开 globals：RTL 自动清理不生效，显式卸载。
});

const fixture = transcriptJson as unknown as {
  samples: { warnings: Envelope; choiceOutOfRange: Envelope; loadRejected: Envelope };
};

/** 最小 stub 内核：只回答 start（真实内核采集的警告样本信封）。 */
class WarningsStubKernel implements KernelBridge {
  start(): Envelope {
    return structuredClone(fixture.samples.warnings);
  }

  dispatch(): Envelope {
    throw new Error("stub：本测试只走 start 路径");
  }

  save(): Envelope {
    throw new Error("stub：本测试只走 start 路径");
  }

  load(): Envelope {
    throw new Error("stub：本测试只走 start 路径");
  }
}

describe("警告显示（D1 warnings 信封）", () => {
  it("信封 warnings 数组 → ⚠ 警告行渲染（真实内核警告文案）", () => {
    render(<GamePanel kernel={new WarningsStubKernel()} onRestart={() => {}} />);
    const warningLine = screen.getByText(/ygg\.choice 之前产生了 2 条派生事件/);
    expect(warningLine).toBeTruthy();
    expect(warningLine.textContent).toMatch(/^⚠ /);
    // 警告信封同样携带待决选择面（choice 样本含 prompt）。
    expect(screen.getByText(/灯芯要拨亮吗/)).toBeTruthy();
  });
});
