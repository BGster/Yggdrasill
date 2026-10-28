// types.ts — 管线内部契约：LLM story 输出面 + 三道门错误面 + 产物/报告面。

/** LLM story 输出（D-9：LLM 只产双件，管线独占 pack.json / schemas / manifest 组装权）。 */
export interface StoryMod {
  slug: string;
  title: string;
  modLua: string;
  /** data/scenes.json 的对象形态（六节结构，键集故事自定）。 */
  scenes: Record<string, unknown>;
}

/**
 * story 契约 schema（客户端权威校验面，D-13：wire 层只传严格兼容子集）。
 * endings 值强制【结局前缀——与 playtest 收口判据（ENDING_PREFIX）互相咬合。
 * 同步义务：scenes 子树与 packages/schema/schemas/scene-texts.json（D-10 通用
 * 模板）的六节结构约束同构——两处并存是有意为之（story 面多 slug/title/modLua
 * 三键，不引入运行期派生装配的过度设计），改模板约束时必须同步本处 scenes 子树。
 */
export const STORY_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["slug", "title", "modLua", "scenes"],
  properties: {
    slug: { type: "string", pattern: "^[a-z0-9][a-z0-9-]{0,47}$" },
    title: { type: "string", minLength: 1, maxLength: 64 },
    modLua: { type: "string", minLength: 1 },
    scenes: {
      type: "object",
      required: ["scenes", "endings", "prompts", "options", "beats", "tuning"],
      additionalProperties: false,
      properties: {
        scenes: {
          type: "object",
          minProperties: 1,
          additionalProperties: { type: "string", minLength: 1 },
        },
        endings: {
          type: "object",
          minProperties: 1,
          additionalProperties: { type: "string", minLength: 1, pattern: "^【结局" },
        },
        prompts: {
          type: "object",
          additionalProperties: { type: "string", minLength: 1 },
        },
        options: {
          type: "object",
          additionalProperties: { type: "string", minLength: 1 },
        },
        beats: {
          type: "object",
          additionalProperties: { type: "string", minLength: 1 },
        },
        tuning: {
          type: "object",
          additionalProperties: { type: "integer", minimum: 0 },
        },
      },
    },
  },
} as const;

/** 三道门统一错误面（research Q1 错误聚合形态：同构后合并反喂）。 */
export interface GateIssue {
  gate: "schema" | "lint" | "playtest";
  /** Mod 包内相对路径；story 契约错误固定 "<response>"。 */
  file: string;
  line?: number;
  column?: number;
  /** ajv keyword / YGG-R1..R3 / YGG-PARSE / PLAYTEST / INTERNAL（稳定码）。 */
  code: string;
  severity: "error" | "warning";
  message: string;
  hint?: string;
}

/** 子进程命令运行器（测试注入面：单测不真跑 cargo）。 */
export interface CommandRunner {
  run(
    command: string,
    args: string[],
    options: { cwd: string },
  ): Promise<{ exitCode: number; stdout: string; stderr: string }>;
}

/** 单轮产物（内存形态，D-15：候选只在内存与临时目录流转）。 */
export interface ModCandidate {
  story: StoryMod;
  modLua: string;
  /** data/scenes.json 文本（pretty 2 空格 + 尾换行）。 */
  scenesJson: string;
  packJson: string;
  manifestJson: string;
  /** schemas/scene-texts.json 文本（自 packages/schema 复制，D-10）。 */
  sceneSchemaJson: string;
}

/** generate 结果（cli 退出码映射的依据）。 */
export interface GenerateResult {
  ok: boolean;
  slug: string;
  outDir: string;
  attempts: number;
  /** 最后一轮聚合错误（ok=false 时非空）。 */
  issues: GateIssue[];
  tokenUsage: { inputTokens: number; outputTokens: number };
}
