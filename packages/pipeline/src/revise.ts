// revise.ts — A4 编排：既有 story 定向修改 → diff 白名单 → 三道门 → 覆盖落盘。
//
// 与 generate（repair.ts）的关系：后四段完全复用（白名单之后的 buildCandidate /
// validateBundle / lintGate / playtestGate / emitModBundle 都是同一批实现），
// 差别只在「改变从哪来」：
//   generate = 一句剧本 → LLM 从零创作（有界重生成，错误反喂）；
//   revise   = 现有产物 + 一条意见 → LLM 定向改写（**单轮**，不重试）。
// 单轮的取舍：revise 的失败面是「越权」（改了冻结项）或「意见本身没落地」，
// 自动重试容易把越权改得更隐蔽；把诊断交给用户比让模型猜更可控。
//
// 现场语义（同 generate / D-15）：候选只在内存与临时目录流转；只有过完白名单
// + 三道门才覆盖同 slug 的 bundle。任何一段失败 → 磁盘一字不动。
//
// A4 不需要流式（改的是几十 KB 级既有文本，进度面收益小）：走 provider.complete。

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { diffStory } from "./diff-gate.js";
import { emitModBundle } from "./emit.js";
import { lintGate } from "./lint-gate.js";
import { playtestGate } from "./playtest-gate.js";
import { buildRevisePrompt, buildSystemPrompt } from "./prompt.js";
import { buildCandidate } from "./repair.js";
import { loadSchemaSchemas, validateBundle, validateStory } from "./schema-gate.js";
import { loadBundleStory, writeStoryMeta } from "./story-bundle.js";
import type { LlmProvider } from "./provider.js";
import { safeSlug } from "./pack.js";
import type { CommandRunner, GateIssue, ReviseResult, StoryMod } from "./types.js";

export interface ReviseOptions {
  /** 要修改的 bundle 目录名（= 作品 slug）。 */
  slug: string;
  /** 用户修改意见（本轮唯一改动依据）。 */
  feedback: string;
  repoRoot: string;
  /** 默认 <repoRoot>/packages/mods/generated。 */
  outRoot?: string;
  /** 注入面：cli 装配真实 provider；测试注入 mock。 */
  provider?: LlmProvider;
  /** 注入面：cli 提供真实子进程 runner；测试注入 fake。 */
  runner?: CommandRunner;
  /** 显式授权大改：放行 mod.lua 变更与 scenes 六节键集增删（默认都冻结）。 */
  allowLua?: boolean;
  log?: (message: string) => void;
}

export async function revise(options: ReviseOptions): Promise<ReviseResult> {
  const log = options.log ?? (() => {});
  if (options.provider === undefined || options.runner === undefined) {
    throw new Error("provider/runner 未注入（cli.ts 装配真实实现；测试注入 fake）");
  }
  const provider = options.provider;
  const runner = options.runner;
  const feedback = options.feedback.trim();
  if (feedback.length === 0) {
    throw new Error("修改意见不能为空（revise -- <slug> \"<修改意见>\"）");
  }
  const allowLua = options.allowLua ?? false;
  const outRoot = options.outRoot ?? join(options.repoRoot, "packages/mods/generated");

  // ① 磁盘回读：五件套重组 StoryMod + title 锚点（缺失则降级，见 story-bundle.ts）。
  const loaded = loadBundleStory(outRoot, options.slug);
  const previous = loaded.story;
  log(
    `[revise] 基线 ${loaded.dir}（slug=${previous.slug}，` +
      `title=${loaded.titleKnown ? `"${previous.title}"` : "未知（无锚点）"}，` +
      `授权=${allowLua ? "allow-lua" : "冻结"})`,
  );
  if (!loaded.titleKnown) {
    log("[revise] 无 title 锚点（A1-A3 期产物）：本轮跳过 title 冻结校验，成功后补写锚点");
  }

  const schemaDir = join(options.repoRoot, "packages/schema/schemas");
  const schemas = loadSchemaSchemas(schemaDir);
  const sceneSchemaJson = readFileSync(join(schemaDir, "scene-texts.json"), "utf8");

  // ② 定向修改（单轮）。
  log(`[revise] 调用 LLM（${provider.protocol}）…`);
  const reply = await provider.complete({
    system: buildSystemPrompt(),
    user: buildRevisePrompt(JSON.stringify(previous, null, 2), feedback, { allowLua }),
  });
  const tokenUsage = reply.usage;
  log(`[token] in=${tokenUsage.inputTokens} out=${tokenUsage.outputTokens}`);

  // ③ story 契约（客户端权威，D-13）：不合契约连 diff 都没法比。
  const storyIssues = validateStory(reply.story);
  if (storyIssues.length > 0) {
    log(`[revise] story 契约校验失败（${storyIssues.length} 条），不落盘`);
    return rejected(previous.slug, [], storyIssues, tokenUsage);
  }
  const next = reply.story as StoryMod;

  // ④ diff 白名单（A4 核心）：越权即整轮作废，磁盘一字不动。
  const diff = diffStory(previous, next, { allowLua, titleKnown: loaded.titleKnown });
  const blocking = diff.issues.filter((issue) => issue.severity === "error");
  if (diff.changed.length > 0) {
    log(`[revise] 变更 ${diff.changed.length} 处：${diff.changed.join("、")}`);
  } else if (blocking.length === 0) {
    log("[revise] 空 diff（无任何变更）"); // 有错误时不报空 diff（避免两句话互相矛盾）
  }
  if (diff.structural.length > 0 && allowLua) {
    log(`[revise] 结构性变更已授权（--allow-lua）：${diff.structural.join("、")}`);
  }
  if (blocking.length > 0) {
    log(`[revise] 白名单校验拒绝（${blocking.length} 条），不落盘`);
    return rejected(previous.slug, diff.changed, diff.issues, tokenUsage);
  }

  // ⑤ 三道门（复用 generate 的实现与序位：schema → lint → playtest）。
  const candidate = buildCandidate(next, sceneSchemaJson);
  let issues = validateBundle(schemas, candidate);
  if (issues.length === 0) {
    log("[revise] schema 门通过");
    issues = await lintGate(runner, options.repoRoot, candidate.modLua);
    if (issues.length === 0) {
      log("[revise] lint 门通过");
      issues = await playtestGate(runner, options.repoRoot, candidate);
      if (issues.length === 0) {
        log("[revise] playtest 门通过");
      }
    }
  }
  if (issues.length > 0) {
    log(`[revise] 门失败 ${issues.length} 条，不落盘（现场保持原样）`);
    return rejected(previous.slug, diff.changed, [...diff.issues, ...issues], tokenUsage);
  }

  // ⑥ 覆盖同 slug 落盘（含 title 锚点刷新）。
  const slug = safeSlug(next.slug);
  const { dir } = emitModBundle(outRoot, slug, candidate);
  writeStoryMeta(outRoot, slug, next);
  log(`[revise] 三道门全绿 → ${dir}`);
  return {
    ok: true,
    slug,
    outDir: dir,
    changed: diff.changed,
    issues: diff.issues, // 可能只含 REVISE_NOOP 警告
    tokenUsage,
  };
}

/** 被拒/失败的统一返回：不带 outDir（磁盘未动）。 */
function rejected(
  slug: string,
  changed: string[],
  issues: GateIssue[],
  tokenUsage: ReviseResult["tokenUsage"],
): ReviseResult {
  return { ok: false, slug, outDir: "", changed, issues, tokenUsage };
}
