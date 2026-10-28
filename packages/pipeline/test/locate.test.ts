import { describe, expect, it } from "vitest";
import { locatePointer } from "../src/locate.js";

const TEXT = `{
  "scenes": {
    "begin": "起点",
    "deep": {
      "inner": "值"
    }
  },
  "endings": {
    "a": "【结局】终"
  }
}
`;

describe("locatePointer（schema 门错误定位）", () => {
  it("顶层键", () => {
    // TEXT 中 "endings" 键在第 8 行（实现返回 found + 1 = 8）。
    expect(locatePointer(TEXT, "/endings")).toEqual({ line: 8, column: 3 });
  });

  it("嵌套键", () => {
    expect(locatePointer(TEXT, "/scenes/begin")).toEqual({ line: 3, column: 5 });
  });

  it("两层嵌套", () => {
    expect(locatePointer(TEXT, "/scenes/deep/inner")).toEqual({ line: 5, column: 7 });
  });

  it("不存在的 pointer → null", () => {
    expect(locatePointer(TEXT, "/scenes/missing")).toBeNull();
  });

  it("根 pointer → (1,1)", () => {
    expect(locatePointer(TEXT, "")).toEqual({ line: 1, column: 1 });
  });
});
