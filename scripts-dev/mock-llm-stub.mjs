#!/usr/bin/env node
// mock-llm-stub.mjs — S4 同款本地 mock LLM（openai-chat 协议形态）：
// POST /v1/chat/completions → 依剧本关键词返回确定性 story JSON（无真凭据）。
// 用法：node scripts-dev/mock-llm-stub.mjs [port]   （仅本地开发/验收用）
//
// A2 追加：支持 `stream: true`（SSE 增量）——无真凭据时也能实测 provider.completeStream
// 的整条流式链路（CLI 进度行 / /__generate 的 progress+preview 事件）。分片按固定
// 字符数切片并带小延迟，让进度肉眼可见。

import { createServer } from "node:http";

const port = Number(process.argv[2] ?? 8931);

// 两则确定性 mock story（键集/结构 = STORY_SCHEMA 契约；Lua 过 ygg-lint 三规则、
// playtest 默认全 1 贪心可通关）。
const STORIES = {
  lighthouse: {
    slug: "lighthouse-keeper",
    title: "暴风雨夜的灯塔",
    modLua: `-- 守灯塔人与信天翁（mock provider 产物）
local T = ygg.data("ygg:scene/texts")
ygg.narrative.node("node:begin", function()
  ygg.narrate(T.scenes.storm)
  ygg.kv.set("stage", "storm")
  ygg.choice(T.prompts.rescue, { T.options.rope, T.options.wait })
end)
ygg.on("ygg.choice_selected", function(index)
  if index == 1 then
    ygg.narrate(T.beats.rescue)
    ygg.narrate(T.endings.dawn)
  else
    ygg.narrate(T.endings.alone)
  end
end)
ygg.on("event.start", function()
  ygg.narrative["goto"]("node:begin")
end)
`,
    scenes: {
      scenes: {
        storm: "暴风雨夜，灯塔的光柱在浪涛上摇晃。一只信天翁被断帆缠住，跌在你窗前的礁石上。",
      },
      beats: {
        rescue: "你冒雨下到礁石，把浸透的信天翁裹进油布。它在你掌心里安静下来。",
      },
      endings: {
        dawn: "【结局晨光】天亮了，信天翁振翅掠过灯塔，绕塔三圈，向北方飞去。",
        alone: "【结局守候】你在灯下守到天明，礁石上只剩断帆的残片与退潮的痕迹。",
      },
      prompts: { rescue: "向信天翁抛出救生索吗？" },
      options: { rope: "抛出绳索", wait: "守在灯下" },
      tuning: { lampOil: 2 },
    },
  },
  tidepool: {
    slug: "tide-pool-secret",
    title: "退潮之后",
    modLua: `-- 退潮之后（mock provider 产物）
local T = ygg.data("ygg:scene/texts")
ygg.narrative.node("node:begin", function()
  ygg.narrate(T.scenes.lowtide)
  ygg.kv.set("stage", "lowtide")
  ygg.choice(T.prompts.descend, { T.options.climb, T.options.leave })
end)
ygg.on("ygg.choice_selected", function(index)
  if index == 1 then
    ygg.narrate(T.beats.find)
    ygg.narrate(T.endings.treasure)
  else
    ygg.narrate(T.endings.tide)
  end
end)
ygg.on("event.start", function()
  ygg.narrative["goto"]("node:begin")
end)
`,
    scenes: {
      scenes: {
        lowtide: "百年一遇的大退潮，礁盘下的潮池露出一道幽蓝的缝隙，隐约有金属光泽。",
      },
      beats: {
        find: "你攀下礁盘，在缝隙深处摸到一只封蜡的铜匣，里面是一枚老灯塔的钥匙。",
      },
      endings: {
        treasure: "【结局铜匣】铜匣里的钥匙对上了灯塔地下室的锁，尘封的航志等你翻启。",
        tide: "【结局涨潮】潮水比你的决心更快地涨了上来，缝隙重新隐入幽蓝。",
      },
      prompts: { descend: "攀下礁盘探进缝隙吗？" },
      options: { climb: "攀下去", leave: "转身回家" },
      tuning: { rope: 1 },
    },
  },
};

function pickStory(userContent) {
  if (typeof userContent === "string") {
    if (userContent.includes("灯塔") || userContent.includes("信天翁")) return withPadding(STORIES.lighthouse);
    if (userContent.includes("潮池") || userContent.includes("退潮")) return withPadding(STORIES.tidepool);
  }
  return withPadding(STORIES.lighthouse);
}

// ── A5：revise 轮（prompt 含「## 修改意见」）→ 在基线上做确定性定向修改 ──────
//
// 为什么需要这一段：revise 的 user prompt 里已经包含整段 story JSON（基线），
// 而它几乎必然包含「灯塔」等关键词——直接用 pickStory 会把 revise 变成
// 「重新生成同一个故事」（恒空 diff）。所以先识别 revise 轮，解析出基线与意见，
// 再按意见做**确定的、可复现的**改动：
// - 总是给首个场景追加一句带意见摘要的尾巴→ 内容变更（scenes.scenes.<key>）；
// - 意见提到 lua → 额外追加一行 Lua 注释→ mod.lua 变更（实测 REVISE_LUA_FROZEN
//   拒绝面与 --allow-lua 放行面）。
// 两条都不会破坏 story 契约 / ygg-lint 三规则 / playtest 可通关性。
const REVISE_MARKER = "## 修改意见（本轮唯一的改动依据）";
const PRIOR_MARKER = "## 现有 Mod（当前磁盘产物，即修改基线）";

function parseRevisePrompt(userContent) {
  if (typeof userContent !== "string" || !userContent.includes(REVISE_MARKER)) {
    return null;
  }
  const feedback = userContent
    .slice(userContent.indexOf(REVISE_MARKER) + REVISE_MARKER.length)
    .split(PRIOR_MARKER)[0]
    .trim();
  const priorAt = userContent.indexOf(PRIOR_MARKER);
  const fenceStart = priorAt < 0 ? -1 : userContent.indexOf("```json", priorAt);
  const fenceEnd = fenceStart < 0 ? -1 : userContent.indexOf("```", fenceStart + 7);
  if (fenceStart < 0 || fenceEnd < 0) {
    return null;
  }
  try {
    return { feedback, prior: JSON.parse(userContent.slice(fenceStart + 7, fenceEnd).trim()) };
  } catch {
    return null; // 基线解析失败 → 回退 pickStory（不至于把 mock 打吐）
  }
}

/** 意见摘要（单行 + 截断）：写进场景文案尾巴，便于肉眼辨认改动来源。 */
function feedbackSnippet(text) {
  const flat = text.replace(/\s+/g, " ").trim();
  const LIMIT = 24;
  return flat.length > LIMIT ? `${flat.slice(0, LIMIT)}…` : flat;
}

function reviseStory(userContent) {
  const parsed = parseRevisePrompt(userContent);
  if (parsed === null || typeof parsed.prior !== "object" || parsed.prior === null) {
    return null;
  }
  const next = structuredClone(parsed.prior);
  const firstKey = Object.keys(next?.scenes?.scenes ?? {})[0];
  if (firstKey !== undefined) {
    next.scenes.scenes[firstKey] =
      `${next.scenes.scenes[firstKey]}（已按意见调整：${feedbackSnippet(parsed.feedback)}）`;
  }
  if (/lua/i.test(parsed.feedback) && typeof next.modLua === "string") {
    next.modLua = `${next.modLua}-- A5 mock：按意见调整（结构面）\n`;
  }
  return next;
}

/**
 * 可选垫长（MOCK_LLM_PAD_CHARS）：把首个场景文案按确定性句子重复到指定字符数。
 * 用途：A2 进度演示需要跨多个千 token 桶的长输出（默认 0 = 不垫，行为不变）。
 */
function withPadding(story) {
  const padChars = Number(process.env.MOCK_LLM_PAD_CHARS ?? 0);
  if (!Number.isFinite(padChars) || padChars <= 0) {
    return story;
  }
  const sentence = "潮声在礁石间来回。";
  const [firstKey, firstText] = Object.entries(story.scenes.scenes)[0];
  let padded = firstText;
  while (padded.length < padChars) {
    padded += sentence;
  }
  return {
    ...story,
    scenes: { ...story.scenes, scenes: { ...story.scenes.scenes, [firstKey]: padded } },
  };
}

/** 请求体是否要求流式（A2 实测路径）。 */
function wantsStream(body) {
  try {
    return JSON.parse(body)?.stream === true;
  } catch {
    return false;
  }
}

/** 流式响应：按 CHUNK_CHARS 切片推 delta，收尾补用量与 [DONE]。 */
async function streamStory(res, content) {
  const CHUNK_CHARS = 96;
  res.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache",
    connection: "keep-alive",
  });
  for (let i = 0; i < content.length; i += CHUNK_CHARS) {
    const piece = content.slice(i, i + CHUNK_CHARS);
    res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: piece } }] })}\n\n`);
    await new Promise((resolve) => setTimeout(resolve, 12));
  }
  res.write(
    `data: ${JSON.stringify({ choices: [], usage: { prompt_tokens: 128, completion_tokens: 512 } })}\n\n`,
  );
  res.write("data: [DONE]\n\n");
  res.end();
}

createServer((req, res) => {
  if (req.method !== "POST" || !req.url?.endsWith("/chat/completions")) {
    res.writeHead(404, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: { message: "not found" } }));
    return;
  }
  let body = "";
  req.on("data", (chunk) => {
    body += chunk;
  });
  req.on("end", () => {
    let user = "";
    try {
      const parsed = JSON.parse(body);
      user = parsed?.messages?.find((m) => m.role === "user")?.content ?? "";
    } catch {
      // 解析失败按缺省故事处理（mock 不做严格校验）。
    }
    // revise 轮优先：它有明确的基线（见 reviseStory 注释），不能走 pickStory。
    const story = reviseStory(user) ?? pickStory(user);
    const content = JSON.stringify(story, null, 2);
    if (wantsStream(body)) {
      void streamStory(res, content);
      return;
    }
    const payload = {
      choices: [{ message: { role: "assistant", content } }],
      usage: { prompt_tokens: 128, completion_tokens: 512 },
    };
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify(payload));
  });
}).listen(port, "127.0.0.1", () => {
  console.log(`mock LLM stub listening on http://127.0.0.1:${port}/v1/chat/completions`);
});
