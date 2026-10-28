// repair.ts — generate 编排（FR-1/FR-7）：story 契约 → pack 现算 → 三道门
// 串联（序位 schema → lint → playtest，FRD D-3：前两道静态廉价，走查最贵
// 放最后）→ 错误聚合反喂有界重生成（默认 3 次，全量重生成）。候选只在
// 内存与临时目录流转；真实 outDir 只在终态落盘（D-15）。token 累计计数
// 软警告（代决 4：单轮 max_tokens 上限 + 累计打印，无硬闸）。

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { emitModBundle } from "./emit.js";
import { lintGate } from "./lint-gate.js";
import {
  buildManifestJson,
  buildPackJson,
  buildScenesJson,
  safeSlug,
} from "./pack.js";
import {
  buildRepairPrompt,
  buildStoryPrompt,
  buildSystemPrompt,
} from "./prompt.js";
import { playtestGate } from "./playtest-gate.js";
import type { LlmProvider } from "./provider.js";
import { loadSchemaSchemas, validateBundle, validateStory } from "./schema-gate.js";
import type {
  CommandRunner,
  GateIssue,
  GenerateResult,
  ModCandidate,
  StoryMod,
} from "./types.js";

export const DEFAULT_MAX_RETRIES = 3;

export interface GenerateOptions {
  story: string;
  /** 重试上限（总轮数 = 1 + maxRetries；FR-7 默认 3）。 */
  maxRetries?: number;
  repoRoot: string;
  /** 默认 <repoRoot>/packages/mods/generated。 */
  outRoot?: string;
  /** 注入面：cli.ts 装配真实 provider；测试注入 mock。 */
  provider?: LlmProvider;
  /** 注入面：cli.ts 提供真实子进程 runner；测试注入 fake。 */
  runner?: CommandRunner;
  log?: (message: string) => void;
}

export async function generate(options: GenerateOptions): Promise<GenerateResult> {
  const log = options.log ?? (() => {});
  const maxRetries = options.maxRetries ?? DEFAULT_MAX_RETRIES;
  if (options.provider === undefined || options.runner === undefined) {
    throw new Error("provider/runner 未注入（cli.ts 装配真实实现；测试注入 fake）");
  }
  const provider = options.provider;
  const runner = options.runner;
  const schemaDir = join(options.repoRoot, "packages/schema/schemas");
  const schemas = loadSchemaSchemas(schemaDir);
  const sceneSchemaJson = readFileSync(join(schemaDir, "scene-texts.json"), "utf8");
  const outRoot = options.outRoot ?? join(options.repoRoot, "packages/mods/generated");
  let tokenUsage = { inputTokens: 0, outputTokens: 0 };
  let priorStoryJson: string | null = null;
  let issues: GateIssue[] = [];
  /** 最后一轮是否通过 story 契约校验（额度耗尽时决定是否保留现场，D-15）。 */
  let lastRoundContractValid = false;

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    const round = attempt + 1;
    const prompt =
      attempt === 0 || priorStoryJson === null
        ? { system: buildSystemPrompt(), user: buildStoryPrompt(options.story) }
        : {
            system: buildSystemPrompt(),
            user: buildRepairPrompt(options.story, priorStoryJson, formatIssues(issues)),
          };
    log(`[round ${round}/${maxRetries + 1}] 调用 LLM（${provider.protocol}）…`);
    const reply = await provider.complete(prompt);
    tokenUsage = {
      inputTokens: tokenUsage.inputTokens + reply.usage.inputTokens,
      outputTokens: tokenUsage.outputTokens + reply.usage.outputTokens,
    };
    log(
      `[token] 累计 in=${tokenUsage.inputTokens} out=${tokenUsage.outputTokens}` +
        "（软警告：单轮上限 = provider DEFAULT_MAX_TOKENS，M1 无硬闸）",
    );

    // 第 0 道：story 契约（客户端权威，D-13）。
    const storyIssues = validateStory(reply.story);
    lastRoundContractValid = storyIssues.length === 0;
    if (storyIssues.length > 0) {
      issues = storyIssues;
      priorStoryJson = JSON.stringify(reply.story, null, 2);
      log(`[round ${round}] story 契约校验失败（${storyIssues.length} 条）`);
      continue;
    }
    const story = reply.story as StoryMod;

    // pack 现算（纯内存，D-15）。
    const candidate = buildCandidate(story, sceneSchemaJson);

    // 三道门串联。
    issues = validateBundle(schemas, candidate);
    if (issues.length === 0) {
      log(`[round ${round}] schema 门通过`);
      issues = await lintGate(runner, options.repoRoot, candidate.modLua);
      if (issues.length === 0) {
        log(`[round ${round}] lint 门通过`);
        issues = await playtestGate(runner, options.repoRoot, candidate);
      }
    }

    if (issues.length === 0) {
      const slug = safeSlug(story.slug);
      const { dir } = emitModBundle(outRoot, slug, candidate);
      log(`[round ${round}] 三道门全绿 → ${dir}`);
      return { ok: true, slug, outDir: dir, attempts: round, issues: [], tokenUsage };
    }
    priorStoryJson = JSON.stringify(reply.story, null, 2);
    log(`[round ${round}] 门失败 ${issues.length} 条，进入重生成`);
  }

  // 重试额度耗尽（FR-7 × D-15）：仅当最后一轮通过 story 契约校验（产物已进入
  // 三道门阶段）才 emit 保留现场；契约未过的 story 落盘会是不可装载的残品，
  // 违反「outDir 完整五件套或不存在」——跳过落盘，只返回 issues（cli 据此措辞）。
  if (!lastRoundContractValid) {
    log("[exhausted] 重试额度耗尽：最后一轮未过 story 契约校验，不落盘（D-15）");
    return { ok: false, slug: "", outDir: "", attempts: maxRetries + 1, issues, tokenUsage };
  }
  // priorStoryJson 由上方 JSON.stringify 同进程产出（L118），不可能非法；
  // lastRoundContractValid 保证其已赋值（fence 原不变量注释）。try/catch 只把
  // 不可能路径转成可读内部错误（fail-fast 语义不变）。
  let lastStory: StoryMod;
  try {
    lastStory = JSON.parse(priorStoryJson!) as StoryMod;
  } catch (cause) {
    throw new Error("内部错误：priorStoryJson 重解析失败（同进程 stringify 产物不应出现）", { cause });
  }
  const candidate = buildCandidate(lastStory, sceneSchemaJson);
  const slug = safeSlug(lastStory.slug);
  const { dir } = emitModBundle(outRoot, slug, candidate);
  log(`[exhausted] 重试额度耗尽，最后产物保留于 ${dir}`);
  return { ok: false, slug, outDir: dir, attempts: maxRetries + 1, issues, tokenUsage };
}

function buildCandidate(story: StoryMod, sceneSchemaJson: string): ModCandidate {
  const scenesJson = buildScenesJson(story.scenes);
  return {
    story,
    modLua: story.modLua,
    scenesJson,
    packJson: buildPackJson(),
    manifestJson: buildManifestJson(scenesJson),
    sceneSchemaJson,
  };
}

function formatIssues(issues: GateIssue[]): string {
  return issues
    .map(
      (issue) =>
        `- [${issue.gate}/${issue.code}] ${issue.file}` +
        `${issue.line !== undefined ? `:${issue.line}:${issue.column ?? 1}` : ""} ${issue.message}` +
        (issue.hint !== undefined ? `\n  hint: ${issue.hint}` : ""),
    )
    .join("\n");
}
