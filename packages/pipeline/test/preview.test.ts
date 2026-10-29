// preview.test.ts — 流式 story JSON 前缀解析（A3）契约测：任意位置截断都不得
// 报错、不得渲染半截文案、不得被文案内容里的伪键骗；随文本增长只增不减。
import { describe, expect, it } from "vitest";
import { emptyPreview, estimateTokens, parseStoryPreview } from "../src/preview.js";

const STORY = {
  slug: "lighthouse-keeper",
  title: "暴风雨夜的灯塔",
  modLua: "-- 守灯塔人\nlocal T = ygg.data(\"ygg:scene/texts\")\n",
  scenes: {
    scenes: {
      storm: "暴风雨夜，灯塔的光柱在浪涛上摇晃。",
      reef: "你听见礁石方向传来一声钝响。",
    },
    endings: {
      dawn: "【结局晨光】信天翁绕塔三圈，向北方飞去。",
      alone: "【结局守候】礁石上只剩断帆的残片。",
    },
    prompts: { rescue: "向信天翁抛出救生索吗？" },
    options: { rope: "抛出绳索", wait: "守在灯下" },
    beats: { rescue: "你冒雨下到礁石。" },
    tuning: { lampOil: 2, nested: { deep: [1, 2, 3] } },
  },
};

const STORY_JSON = JSON.stringify(STORY, null, 2);

describe("parseStoryPreview — 完整文本", () => {
  it("完整 JSON → title + 全部场景与结局", () => {
    expect(parseStoryPreview(STORY_JSON)).toEqual({
      title: "暴风雨夜的灯塔",
      scenes: [
        { id: "storm", text: "暴风雨夜，灯塔的光柱在浪涛上摇晃。" },
        { id: "reef", text: "你听见礁石方向传来一声钝响。" },
      ],
      endings: [
        { id: "dawn", text: "【结局晨光】信天翁绕塔三圈，向北方飞去。" },
        { id: "alone", text: "【结局守候】礁石上只剩断帆的残片。" },
      ],
    });
  });

  it("幂等：同一份文本重复调用结果相等", () => {
    expect(parseStoryPreview(STORY_JSON)).toEqual(parseStoryPreview(STORY_JSON));
  });

  it("markdown 围栏残留（降级路径）不影响解析：从第一个 { 起扫", () => {
    const fenced = `\`\`\`json\n${STORY_JSON}\n\`\`\``;
    expect(parseStoryPreview(fenced).title).toBe("暴风雨夜的灯塔");
    expect(parseStoryPreview(fenced).scenes).toHaveLength(2);
  });

  it("空文本 / 垃圾文本 → 空预览（不抛错）", () => {
    expect(parseStoryPreview("")).toEqual(emptyPreview());
    expect(parseStoryPreview("模型开始胡思乱想")).toEqual(emptyPreview());
    expect(parseStoryPreview("{")).toEqual(emptyPreview());
  });
});

describe("parseStoryPreview — 任意位置截断", () => {
  it("每一个前缀都不抛错，且结果是最终结果的子集（单调递增）", () => {
    let previous = emptyPreview();
    for (let cut = 1; cut <= STORY_JSON.length; cut++) {
      const preview = parseStoryPreview(STORY_JSON.slice(0, cut));
      expect(preview.scenes.length).toBeGreaterThanOrEqual(previous.scenes.length);
      expect(preview.endings.length).toBeGreaterThanOrEqual(previous.endings.length);
      // 已出现的条目文本必须与最终结果逐字一致（绝不渲染半截文案）。
      for (const entry of preview.scenes) {
        expect(entry.text).toBe(STORY.scenes.scenes[entry.id as "storm" | "reef"]);
      }
      for (const entry of preview.endings) {
        expect(entry.text).toBe(previous.endings.find((e) => e.id === entry.id)?.text ?? entry.text);
      }
      previous = preview;
    }
    expect(previous).toEqual({
      title: "暴风雨夜的灯塔",
      scenes: [
        { id: "storm", text: "暴风雨夜，灯塔的光柱在浪涛上摇晃。" },
        { id: "reef", text: "你听见礁石方向传来一声钝响。" },
      ],
      endings: [
        { id: "dawn", text: "【结局晨光】信天翁绕塔三圈，向北方飞去。" },
        { id: "alone", text: "【结局守候】礁石上只剩断帆的残片。" },
      ],
    });
  });

  it("title 值截断 → null（不猜半截标题）", () => {
    const prefix = '{"slug":"s","title":"暴风雨夜';
    expect(parseStoryPreview(prefix).title).toBeNull();
  });

  it("首个场景值截断 → 该条整条丢弃，第二条不越过截断点", () => {
    // 截断点落在 storm 的文案中间（开引号之后、闭合引号之前）。
    const prefix = '{"title":"灯塔","scenes":{"scenes":{"storm":"暴风雨夜，灯塔的光柱';
    const preview = parseStoryPreview(prefix);
    expect(preview.title).toBe("灯塔");
    expect(preview.scenes).toEqual([]); // 未闭合的值不渲染
    // 补上剩余文本与收口括号 → 该条才出现（且文本完整）。
    const rest = '在浪涛上摇晃。"}}';
    expect(parseStoryPreview(prefix + rest).scenes).toEqual([
      { id: "storm", text: "暴风雨夜，灯塔的光柱在浪涛上摇晃。" },
    ]);
  });

  it("键名截断 / 冒号缺失 → 停止扫描并保留已收集条目", () => {
    const ok =
      '{"title":"灯塔","scenes":{"scenes":{"storm":"甲","reef":"乙"';
    const preview = parseStoryPreview(ok);
    expect(preview.scenes.map((s) => s.id)).toEqual(["storm", "reef"]);
    expect(parseStoryPreview('{"scenes":{"scenes":{"storm"').scenes).toEqual([]);
    expect(parseStoryPreview('{"scenes":{"scenes":{}}').scenes).toEqual([]);
  });

  it("转义序列（\\\" \\\\ \\n）正确解码", () => {
    const json = JSON.stringify({ title: '灯塔"守望者"\\东', scenes: { scenes: { a: "第一行\n第二行" } } });
    const preview = parseStoryPreview(json);
    expect(preview.title).toBe('灯塔"守望者"\\东');
    expect(preview.scenes).toEqual([{ id: "a", text: "第一行\n第二行" }]);
  });
});

describe("parseStoryPreview — 不被文本内容骗", () => {
  it('场景文案内含 "title": 字样时，标题仍取顶层值', () => {
    const json = JSON.stringify({
      title: "真标题",
      scenes: { scenes: { tricky: '他说："title": "假标题" 然后笑了。' } },
    });
    const preview = parseStoryPreview(json);
    expect(preview.title).toBe("真标题");
    expect(preview.scenes).toEqual([{ id: "tricky", text: '他说："title": "假标题" 然后笑了。' }]);
  });

  it("嵌套对象 / 数组值（tuning）不打断后续扫描", () => {
    const json = JSON.stringify({
      scenes: {
        tuning: { lampOil: 2, nested: { deep: [1, 2, 3] }, text: "字符串里的 } 括号" },
        endings: { a: "【结局】终。" },
      },
    });
    expect(parseStoryPreview(json).endings).toEqual([{ id: "a", text: "【结局】终。" }]);
  });

  it("非字符串值不会进预览（scenes 里混入数字也只收字符串条目）", () => {
    const json = '{"scenes":{"scenes":{"a":"甲","b":3,"c":"丙"}}}';
    expect(parseStoryPreview(json).scenes).toEqual([
      { id: "a", text: "甲" },
      { id: "c", text: "丙" },
    ]);
  });
});

describe("estimateTokens", () => {
  it("4 字符 ≈ 1 token（向上取整）", () => {
    expect(estimateTokens(0)).toBe(0);
    expect(estimateTokens(1)).toBe(1);
    expect(estimateTokens(4)).toBe(1);
    expect(estimateTokens(5)).toBe(2);
    expect(estimateTokens(33_728)).toBe(8_432);
  });
});
