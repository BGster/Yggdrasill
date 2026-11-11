/**
 * harness.test.ts —— `.pi/extensions/ygg-pi-harness` 的行为测试
 *
 * 跑法（无外部依赖，只用 node 内置 test runner）：
 *   node --experimental-strip-types --test .pi/extensions/ygg-pi-harness/harness.test.ts
 *
 * 覆盖面：
 * ① [`checkToolCallFirewall`]：数据红线（`*.xlsx` / `*.xls` / `*.csv` / `/Volumes/WenshuSpace`）
 *    与批量入口禁令（`--events` / shell 循环 / `xargs`），以及**不误伤**正常代码与
 *    `/tmp` 合成数据；
 * ② [`buildSubagentBrief`]：自洽性（工作区边界、需求正文、验证门禁、红线、返回格式）；
 * ③ [`orchestrateActiveJobs`]：mock daemon + mock subagent，验证「先 claim worktree →
 *    并发派活 → vfs_put → lane_probe → proposal.submit / accept → 停在 plan_gate /
 *    accept_gate」这条链，且回执 narrative 不丢行；
 * ④ [`YggDaemon`] 的行协议（用 `process.execPath -e` 当假 daemon）+ `.env` 只暴露两项。
 */

import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { after, describe, it } from "node:test";

import {
  describeEnvKeys,
  formatReceipt,
  normalizeReceipt,
  parseEnvFile,
  parseSummary,
  statusLine,
  subagentSettings,
  toStepReceipt,
  unknownSummary,
  YggDaemon,
  type RunSummary,
  type StepReceipt,
  type SummaryJob,
  type YggActiveJob,
  type YggHarnessAction,
  type YggReceipt,
  type YggSubagentAction,
} from "./daemon.ts";
import { checkToolCallFirewall, checkBatchEntrypoint, checkPathText } from "./firewall.ts";
import {
  actorForLedger,
  artifactPathFor,
  branchNameFor,
  briefArtifactPathFor,
  buildLaneProbeArgs,
  buildProposalSubmitPayload,
  buildResumeDeltaPrompt,
  buildSubagentBrief,
  buildSubmitLine,
  buildWorktreeClaimArgs,
  checkSubagentActionConsumption,
  COMMIT_NUDGE_TEXT,
  DELTA_ASSESSABLE,
  ensureBriefVerificationAndFooter,
  ensureResumeHandoverSection,
  extractAgentId,
  isResumableAgentId,
  MANDATORY_REFERENCES_ANCHOR,
  laneProbeNeedsCommit,
  orchestrateActiveJobs,
  parseLaneProbeNarrative,
  parseSubagentOutput,
  pickFreeLane,
  readLaneProbeFromGit,
  requirementNumber,
  RESERVED_AGENT_IDS,
  RESUME_HANDOVER_HEADING,
  resolveResumeId,
  resumeReason,
  sanitizeSegment,
  selectCoreSubagentPrompt,
  slugify,
  UNKNOWN_LANE_PROBE,
  worktreePathFor,
  type AdapterContractPolicy,
  type OrchestratorDaemon,
  type SubagentRequest,
} from "./orchestrator.ts";
import {
  analyzeArchiveVerdict,
  archiveSessionsSegment,
  archiveVerdictNarrative,
} from "./archive.ts";
import {
  buildCompactInstructions,
  buildHandoffMarkdown,
  buildResumeMessage,
  DEFAULT_EFFECTIVE_WINDOW_CAP,
  evaluateContextPressure,
  formatPercent,
  HANDOFF_ARTIFACT_PATH,
  HANDOFF_RESUME_CUSTOM_TYPE,
  isStableCheckpoint,
  parsePositiveIntEnv,
  stableCheckpointReason,
  writeHandoffToVfs,
  type HandoffWrite,
} from "./compaction.ts";
import {
  AUTO_WAVE_CUSTOM_TYPE,
  AutoWatchdog,
  buildGateWakeMessage,
  buildWaveLogMarkdown,
  buildWaveWakeMessage,
  clampAutoIntervalMs,
  collectKey,
  collectJobs,
  DEFAULT_AUTO_INTERVAL_MS,
  initialAutoPilotState,
  isDeliveredJob,
  MAX_AUTO_INTERVAL_MS,
  MIN_AUTO_INTERVAL_MS,
  nextAutoAction,
  parseAutoWake,
  recordWaveOutcome,
  waveLogPath,
  WAVE_FAILURES_AUTO_OFF,
  WAVE_FAILURES_PARK,
} from "./watchdog.ts";

// ───────────────────────────── ① 数据红线 ─────────────────────────────

describe("firewall: 数据红线（优先级 0）", () => {
  it("拦下 read / ls / grep / find / edit / write 触碰涉密挂载目录", () => {
    const cases: [string, Record<string, unknown>][] = [
      ["read", { path: "/Volumes/WenshuSpace/notes.md" }],
      ["ls", { path: "/Volumes/WenshuSpace" }],
      ["grep", { path: "/Volumes/WenshuSpace/sub", pattern: "x" }],
      ["find", { path: "/Volumes/WenshuSpace", pattern: "*.md" }],
      ["edit", { path: "/Volumes/WenshuSpace/a/b.ts", edits: [] }],
      ["write", { path: "/Volumes/WenshuSpace/out.txt", content: "x" }],
    ];
    for (const [tool, input] of cases) {
      const verdict = checkToolCallFirewall(tool, input);
      assert.equal(verdict.block, true, `${tool} 应该被拦`);
      assert.match(verdict.reason ?? "", /DATA-RED-LINE/);
    }
  });

  it("拦下任何 .xlsx / .xls / .csv（大小写不敏感，含数组入参）", () => {
    assert.equal(checkToolCallFirewall("read", { path: "metrics.xlsx" }).block, true);
    assert.equal(checkToolCallFirewall("read", { path: "~/tmp/real.csv" }).block, true);
    assert.equal(checkToolCallFirewall("read", { path: "REPORT.XLS" }).block, true);
    assert.equal(checkToolCallFirewall("grep", { path: "data", glob: "*.csv" }).block, true);
    assert.equal(
      checkToolCallFirewall("read", { paths: ["src/app.ts", "data/raw.csv"] }).block,
      true,
      "数组里有一个数据文件也算违规",
    );
    assert.match(checkToolCallFirewall("read", { path: "a.xlsx" }).reason ?? "", /合成数据|synthetic/);
  });

  it("放行 /tmp 下的合成数据与正常代码文件", () => {
    assert.equal(checkToolCallFirewall("read", { path: "/tmp/synthetic.csv" }).block, false);
    assert.equal(checkToolCallFirewall("read", { path: "/private/tmp/made-up.xlsx" }).block, false);
    assert.equal(checkToolCallFirewall("write", { path: "/tmp/fixture/x.xls", content: "" }).block, false);
    for (const file of ["src/index.ts", "AGENTS.md", "crates/ygg-core/src/lib.rs", "a.lua", "data/config.json"]) {
      assert.equal(checkToolCallFirewall("read", { path: file }).block, false, `${file} 不该被拦`);
    }
  });

  it("bash 里的路径同样过红线（含 --flag=/path 与 cd）", () => {
    assert.equal(checkToolCallFirewall("bash", { command: "cat /Volumes/WenshuSpace/x" }).block, true);
    assert.equal(checkToolCallFirewall("bash", { command: "ls /Volumes/WenshuSpace" }).block, true);
    assert.equal(checkToolCallFirewall("bash", { command: "wc -l ~/tmp/real.csv" }).block, true);
    assert.equal(checkToolCallFirewall("bash", { command: "du --files0-from=data.csv" }).block, true);
    assert.equal(checkToolCallFirewall("bash", { command: "cat /tmp/synthetic.csv | head" }).block, false);
    assert.equal(checkToolCallFirewall("bash", { command: "cargo test --workspace" }).block, false);
  });

  it("只扫路径语义的入参：正文里出现 .csv 不误伤", () => {
    const write = { path: "src/parse.ts", content: "const EXTS = ['.csv', '.xlsx'];" };
    assert.equal(checkToolCallFirewall("write", write).block, false);
    // grep 的 pattern 是正则，不是路径
    assert.equal(checkToolCallFirewall("grep", { pattern: "\\.csv$", path: "src" }).block, false);
    // find 的 pattern 是 glob，枚举文件 → 要拦
    assert.equal(checkToolCallFirewall("find", { pattern: "*.csv", path: "data" }).block, true);
    assert.equal(checkToolCallFirewall("find", { pattern: "*.ts", path: "src" }).block, false);
  });

  it("checkPathText 对涉密目录下的 .csv 仍按红线优先处理", () => {
    const verdict = checkPathText("/Volumes/WenshuSpace/table.xlsx");
    assert.notEqual(verdict, null);
    assert.equal(verdict?.block, true);
  });
});

describe("firewall: 批量入口永久禁令", () => {
  it("拦 `ygg-agent --events`", () => {
    const verdict = checkToolCallFirewall("bash", { command: "ygg-agent --events 'a,b,c'" });
    assert.equal(verdict.block, true);
    assert.match(verdict.reason ?? "", /BATCH-ENTRYPOINT/);
    assert.equal(checkToolCallFirewall("bash", { command: "ygg-agent --events=a,b" }).block, true);
  });

  it("拦 shell 循环批量刷 --resume --event", () => {
    for (const command of [
      "for r in r1 r2; do ygg-agent --resume --event requirement.dispatch; done",
      "while true; do ygg-agent --resume --event proposal.accept; done",
      "until false\ndo\n  ygg-agent --resume --event gate.approve\ndone",
      "seq 1 9 | xargs -n1 ygg-agent --resume --event proposal.accept",
    ]) {
      assert.equal(checkBatchEntrypoint(command)?.block, true, command);
    }
  });

  it("放行单发 `--event`（一次一个事件）与无关命令", () => {
    for (const command of [
      "ygg-agent --mod .ygg/mods/pi-agent-seed --run-id demo --event requirement.dispatch",
      "ygg-agent --resume --event config.set --payload '\"goal.authorization=x\"'",
      'git commit -m "feat(harness): 讲清楚 why we never use --events in scripts"',
      "node --experimental-strip-types --test .pi/extensions/ygg-pi-harness/harness.test.ts",
    ]) {
      assert.equal(checkBatchEntrypoint(command), null, command);
      assert.equal(checkToolCallFirewall("bash", { command }).block, false, command);
    }
  });

  it("与红线无关的工具调用一律放行", () => {
    assert.equal(checkToolCallFirewall("todo", { action: "list" }).block, false);
    assert.equal(checkToolCallFirewall("Agent", { prompt: "x" }).block, false);
  });
});

// ───────────────────────────── ② brief ─────────────────────────────

const developmentJob: SummaryJob = {
  id: "r2",
  rev: 1,
  stage: "development",
  status: "developing",
  phase: "developing",
  lane: 0,
  branch: "",
  base: "",
  title: "接入泳道编排",
  description: "实现 orchestrator.ts 的作业生命周期\n并加测试",
};

describe("buildSubagentBrief", () => {
  it("development：带工作区硬边界、需求正文、验证门禁与返回格式", () => {
    const brief = buildSubagentBrief(
      developmentJob,
      "/tmp/ygg-worktrees/repo-lane-2",
      "artifacts/r2-development.md",
      "/tmp/repo",
      { verification: ["cargo test --workspace"], lane: 2, branch: "feature/r2-x", base: "abc123" },
    );
    assert.match(brief, /\/tmp\/ygg-worktrees\/repo-lane-2/);
    assert.match(brief, /只在这个目录里工作/);
    assert.match(brief, /lane-2/);
    assert.match(brief, /feature\/r2-x/);
    assert.match(brief, /abc123/);
    assert.match(brief, /接入泳道编排/);
    assert.match(brief, /实现 orchestrator\.ts 的作业生命周期/);
    assert.match(brief, /cargo test --workspace/);
    assert.match(brief, /artifacts\/r2-development\.md/);
    assert.match(brief, /数据红线/);
    assert.match(brief, /不要 push/);
    // 需求描述的多行必须逐行保留（分段丢失 = 需求失真）
    assert.match(brief, /并加测试/);
  });

  it("plan：没有 worktree 时声明只读，不写实现代码", () => {
    const planJob: SummaryJob = { ...developmentJob, id: "r1", stage: "plan" };
    const brief = buildSubagentBrief(planJob, null, "artifacts/r1-plan.md", "/tmp/repo");
    assert.match(brief, /plan 阶段/);
    assert.match(brief, /只读/);
    assert.match(brief, /不写实现代码/);
    assert.match(brief, /cargo test --workspace/, "缺省验证门禁要落进 brief");
  });

  it("buildSubagentBrief 与 ensureBriefVerificationAndFooter 包含必读文档 docs/CONTEXT.md 与 docs/DESIGN.md", () => {
    const brief = buildSubagentBrief(
      developmentJob,
      "/tmp/ygg-worktrees/repo-lane-2",
      "artifacts/r2-development.md",
      "/tmp/repo",
      { lane: 2, branch: "feature/r2-x", base: "abc123" },
    );
    assert.match(brief, /### 必读领域与架构文档（Mandatory References）/);
    assert.match(brief, /docs\/CONTEXT\.md/);
    assert.match(brief, /docs\/DESIGN\.md/);

    const augmented = ensureBriefVerificationAndFooter(
      "# Core Brief\nMinimal instructions.",
      "artifacts/r2-development.md",
      ["cargo test --workspace"],
      { worktreePath: "/tmp/ygg-worktrees/repo-lane-2", cwd: "/tmp/repo" },
    );
    assert.match(augmented, /\/tmp\/ygg-worktrees\/repo-lane-2/);
    assert.match(augmented, /### 必读领域与架构文档（Mandatory References）/);
    assert.match(augmented, /docs\/CONTEXT\.md/);
    assert.match(augmented, /docs\/DESIGN\.md/);
  });
});

// ───────────────────────────── ③ 编排 ─────────────────────────────

function jobRow(overrides: Partial<SummaryJob> = {}): SummaryJob {
  return {
    id: "r1",
    rev: 1,
    stage: "plan",
    status: "planning",
    phase: "planning",
    lane: 0,
    branch: "",
    base: "",
    title: "方案",
    description: "产出方案",
    ...overrides,
  };
}

function summaryWith(
  jobs: SummaryJob[],
  gates: { id: string; gate: string; rev: number; title: string }[] = [],
  overrides: Partial<RunSummary> = {},
): RunSummary {
  return {
    available: true,
    stage: "coordinate",
    ending: false,
    verifyPassed: false,
    nextEndsRun: false,
    pendingProposals: 0,
    requirementCount: jobs.length,
    pendingTools: 0,
    conflicts: [],
    jobs,
    gates,
    ...overrides,
  };
}

function receipt(narrative: string[], ok = true): StepReceipt {
  return { ok, narrative, pendingChoice: null, tools: [], raw: { ok } };
}

/** mock daemon：记录每次 dispatch / vfsPut，并允许在 claim 后「绑定泳道」。 */
class MockDaemon implements OrchestratorDaemon {
  readonly calls: { op: string; event: string; payload?: string | number }[] = [];
  readonly vfs: { path: string; content: string }[] = [];
  private readonly state: { jobs: SummaryJob[] };
  private readonly bindOnClaim: boolean;

  constructor(jobs: SummaryJob[], bindOnClaim = true) {
    this.state = { jobs: jobs.map((job) => ({ ...job })) };
    this.bindOnClaim = bindOnClaim;
  }

  async summary(): Promise<RunSummary> {
    return summaryWith(this.state.jobs);
  }

  async dispatch(event: string, payload?: string | number): Promise<StepReceipt> {
    this.calls.push({ op: "dispatch", event, ...(payload === undefined ? {} : { payload }) });
    if (event === "tool.call") {
      const text = String(payload ?? "");
      const [toolId, args = ""] = text.split("|");
      if (toolId === "git.worktree_claim" && this.bindOnClaim) {
        const lane = Number((args.match(/lane=(\d+)/) ?? [])[1] ?? 0);
        const req = (args.match(/req=([^;]+)/) ?? [])[1] ?? "";
        const branch = (args.match(/branch=([^;]+)/) ?? [])[1] ?? "";
        const base = (args.match(/base=([^;]+)/) ?? [])[1] ?? "";
        this.state.jobs = this.state.jobs.map((job) =>
          job.id === req ? { ...job, lane, branch, base } : job,
        );
      }
      return receipt([`[TOOL] ${toolId}`]);
    }
    return receipt([`[${event}] ok`]);
  }

  async vfsPut(artifactPath: string, content: string): Promise<{ ok: boolean; bytes: number }> {
    this.vfs.push({ path: artifactPath, content });
    return { ok: true, bytes: content.length };
  }

  claimCalls(): string[] {
    return this.calls
      .filter((call) => call.event === "tool.call")
      .map((call) => String(call.payload ?? ""));
  }
}

describe("orchestrateActiveJobs", () => {
  it("development 缺泳道 → 先 claim worktree → 并发派活 → 归档 → 探针 → 提案 accept", async () => {
    const job = jobRow({
      id: "r2",
      stage: "development",
      lane: 0,
      branch: "",
      title: "接泳道",
      description: "实现编排",
    });
    const daemon = new MockDaemon([job]);
    const seen: SubagentRequest[] = [];
    let inflight = 0;
    let maxInflight = 0;

    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        inflight += 1;
        maxInflight = Math.max(maxInflight, inflight);
        await new Promise((resolve) => setTimeout(resolve, 20));
        inflight -= 1;
        return { text: "实现完成\ncargo test --workspace: ok", ok: true };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        worktreeRoot: "/tmp/ygg-worktrees",
        resolveBase: () => "deadbeef",
        readArtifact: () => null,
        lanePollMs: 1,
        laneWaitMs: 50,
      },
    );

    // ⓪ claim 在派活之前：泳道 1 + 分支名 + 基线
    const claims = daemon.claimCalls();
    assert.equal(claims.length >= 1, true, "应该有一次 tool.call");
    assert.match(claims[0], /^git\.worktree_claim\|lane=1;req=r2;branch=feature\/r2-/);
    assert.match(claims[0], /base=deadbeef/);

    // ① 只派了一个 subagent，brief 里有隔离 worktree
    assert.equal(seen.length, 1);
    assert.equal(seen[0].description, "r2 development");
    assert.match(seen[0].prompt, /repo-lane-1/);
    assert.equal(maxInflight, 1);

    // ② VFS：先落 brief（C3，无 Agent 工具时的 fallback 承接），再归档制品
    const briefWrites = daemon.vfs.filter((entry) => entry.path.endsWith("-brief.md"));
    const artifactWrites = daemon.vfs.filter((entry) => !entry.path.endsWith("-brief.md"));
    assert.equal(briefWrites.length, 1, "每个作业都要落一份 brief");
    assert.equal(briefWrites[0].path, "artifacts/r2-development-brief.md");
    assert.equal(
      briefWrites[0].content,
      seen[0].prompt,
      "落盘的 brief 必须与真正发给 subagent 的 prompt 逐字节相同",
    );
    assert.equal(artifactWrites.length, 1);
    assert.equal(artifactWrites[0].path, "artifacts/r2-development.md");
    assert.match(artifactWrites[0].content, /cargo test --workspace: ok/);

    // ③ 探针 → 提案 → accept
    const probes = claims.filter((call) => call.startsWith("git.lane_probe|"));
    assert.equal(probes.length, 1);
    assert.match(probes[0], /lane=1/);
    const events = daemon.calls.map((call) => call.event);
    const submitIndex = events.indexOf("proposal.submit");
    const acceptIndex = events.indexOf("proposal.accept");
    assert.equal(submitIndex > 0, true, "要提 proposal.submit");
    assert.equal(acceptIndex > submitIndex, true, "accept 必须在 submit 之后");

    const submitPayload = String(daemon.calls[submitIndex].payload ?? "");
    const segments = submitPayload.split("|");
    assert.equal(segments[0], "r2");
    assert.equal(segments[1], "1");
    assert.equal(segments[2], "development");
    assert.equal(segments[3].startsWith("auto development:"), true);
    assert.equal(segments[4], "artifacts/r2-development.md");
    assert.equal(segments[5], "");
    assert.equal(segments[6], "auto-r2-development-r1");
    assert.equal(segments[7].startsWith("exec=pi-ygg-harness:"), true);
    assert.equal(String(daemon.calls[acceptIndex].payload), "r2");

    // ④ 落在 accept 门
    assert.equal(result.jobs.length, 1);
    assert.equal(result.jobs[0].expectedGate, "accept");
    assert.equal(result.jobs[0].lane, 1);
    assert.equal(result.jobs[0].worktreePath, path.join("/tmp/ygg-worktrees", "repo-lane-1"));
    // 回执整段保留
    assert.equal(result.narrative.some((line) => line.includes("[proposal.submit] ok")), true);
    assert.equal(
      result.narrative.some((line) => line.includes("[TOOL] git.worktree_claim")),
      true,
      "claim 的回执行必须留在 narrative 里",
    );
    assert.equal(
      result.narrative.some((line) => line.includes("[TOOL] git.lane_probe")),
      true,
      "探针的回执行必须留在 narrative 里",
    );
  });

  it("plan 阶段不需要 worktree，两个作业的 subagent 并发执行", async () => {
    const jobs = [
      jobRow({ id: "r1", stage: "plan", title: "方案一" }),
      jobRow({ id: "r2", stage: "plan", title: "方案二" }),
    ];
    const daemon = new MockDaemon(jobs);
    let inflight = 0;
    let maxInflight = 0;
    const started: string[] = [];

    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        started.push(request.description);
        inflight += 1;
        maxInflight = Math.max(maxInflight, inflight);
        await new Promise((resolve) => setTimeout(resolve, 25));
        inflight -= 1;
        return { text: `# ${request.description}\n方案正文`, ok: true };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        resolveBase: () => "abc",
        readArtifact: () => null,
      },
    );

    assert.equal(maxInflight, 2, "独立作业必须并发（Promise.all）而不是串行");
    assert.deepEqual(started.sort(), ["r1 plan", "r2 plan"]);
    assert.equal(daemon.claimCalls().length, 0, "plan 阶段不建 worktree");
    assert.equal(result.jobs.every((job) => job.worktreePath === null), true);
    assert.equal(result.jobs.every((job) => job.expectedGate === "plan"), true);
    assert.deepEqual(
      daemon.vfs.map((entry) => entry.path).sort(),
      [
        "artifacts/r1-plan-brief.md",
        "artifacts/r1-plan.md",
        "artifacts/r2-plan-brief.md",
        "artifacts/r2-plan.md",
      ],
      "每个作业两件：brief（C3 承接）+ 制品",
    );
  });

  it("subagent 自己写了制品就优先用落盘内容", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r3", stage: "plan" })]);
    await orchestrateActiveJobs(
      daemon,
      async () => ({ text: "聊天版报告", ok: true }),
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        readArtifact: (absPath) => (absPath.endsWith("r3-plan.md") ? "# 落盘版方案" : null),
      },
    );
    const archived = daemon.vfs.find((entry) => entry.path === "artifacts/r3-plan.md");
    assert.equal(archived?.content, "# 落盘版方案");
  });

  it("subagent 失败不阻断其他作业，错误进回执", async () => {
    const daemon = new MockDaemon([
      jobRow({ id: "r1", stage: "plan" }),
      jobRow({ id: "r2", stage: "plan" }),
    ]);
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        if (request.description.startsWith("r1")) {
          throw new Error("subagent 崩了");
        }
        return { text: "ok", ok: true };
      },
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(result.jobs.length, 2);
    const failed = result.jobs.find((job) => job.id === "r1");
    assert.equal(failed?.error, "subagent 崩了");
    const archiveWrites = daemon.vfs.filter((entry) => !entry.path.endsWith("-brief.md"));
    assert.equal(archiveWrites.length, 2, "失败的作业也要归档一条（写明失败原因）");
    assert.match(
      archiveWrites.find((entry) => entry.path.includes("r1"))?.content ?? "",
      /崩了/,
    );
  });

  it("摘要不可用时不猜：返回降级原因且不动状态", async () => {
    const daemon = new MockDaemon([]);
    daemon.summary = async () => unknownSummary("没有 meta.summary");
    const result = await orchestrateActiveJobs(
      daemon,
      async () => ({ text: "x", ok: true }),
      { cwd: "/tmp/repo", repoDir: "/tmp/repo" },
    );
    assert.equal(result.jobs.length, 0);
    assert.equal(result.degraded, "没有 meta.summary");
    assert.equal(daemon.calls.length, 0);
  });

  it("maxLanes 用满时不复用已占泳道", () => {
    const busy = [
      jobRow({ id: "r1", lane: 1, stage: "development" }),
      jobRow({ id: "r2", lane: 2, stage: "development" }),
    ];
    assert.equal(pickFreeLane(busy, 4), 3);
    assert.equal(pickFreeLane(busy, 2), null);
    assert.equal(pickFreeLane(busy, 4, "r2"), 2);
  });
});

describe("纯函数：命名与载荷", () => {
  it("branchNameFor 幂等且带需求号", () => {
    const job = jobRow({ id: "r7", title: "Fix: lane probe (radar)" });
    assert.equal(branchNameFor(job), "feature/r7-fix-lane-probe-radar");
    assert.equal(branchNameFor({ ...job, branch: "feature/keep" }), "feature/keep");
    assert.equal(requirementNumber("r12"), 12);
    assert.equal(slugify(""), "work");
  });

  it("slugify 只留小写字母数字与连字符", () => {
    assert.equal(slugify("A/b c__d"), "a-b-c-d");
    assert.equal(slugify("x".repeat(80)).length <= 40, true);
  });

  it("sanitizeSegment 抹掉竖线与换行（提案载荷的分段符）", () => {
    assert.equal(sanitizeSegment("a|b\nc"), "a b c");
    assert.equal(sanitizeSegment("x".repeat(500), 20).length <= 20, true);
  });

  it("buildWorktreeClaimArgs / buildLaneProbeArgs 形状稳定", () => {
    assert.equal(
      buildWorktreeClaimArgs({ lane: 2, req: "r4", branch: "feature/r4-x", base: "abc" }),
      "lane=2;req=r4;branch=feature/r4-x;base=abc",
    );
    assert.equal(
      buildLaneProbeArgs({ lane: 2, base: "abc", branch: "feature/r4-x" }),
      "lane=2;base=abc;branch=feature/r4-x",
    );
    assert.equal(buildWorktreeClaimArgs({ lane: 1, req: "r1", branch: "", base: "" }), "lane=1;req=r1");
  });

  it("buildProposalSubmitPayload 位次固定", () => {
    const payload = buildProposalSubmitPayload({
      job: jobRow({ id: "r3", rev: 2, stage: "plan" }),
      summary: "方案：a|b",
      artifactPath: "artifacts/r3-plan.md",
      actorId: "agent-x",
    });
    const segments = payload.split("|");
    assert.equal(segments.length, 8);
    assert.equal(segments[0], "r3");
    assert.equal(segments[1], "2");
    assert.equal(segments[2], "plan");
    assert.equal(segments[3], "方案：a b");
    assert.equal(segments[4], "artifacts/r3-plan.md");
    assert.equal(segments[5], "");
    assert.equal(segments[6], "auto-r3-plan-r2");
    assert.equal(segments[7], "exec=agent-x:auto r3 plan r2");
  });

  it("artifactPathFor / worktreePathFor 与宿主约定一致", () => {
    assert.equal(artifactPathFor(jobRow({ id: "r9", stage: "development" })), "artifacts/r9-development.md");
    assert.equal(
      worktreePathFor("/tmp/ygg-worktrees", "/tmp/MyRepo", 3),
      path.join("/tmp/ygg-worktrees", "MyRepo-lane-3"),
    );
  });
});

// ───────────────────────────── ④ daemon 客户端 ─────────────────────────────

describe("YggDaemon: 行协议", () => {
  const fakeDaemonScript = `
const readline = require("node:readline");
const rl = readline.createInterface({ input: process.stdin });
const jobs = [];
rl.on("line", (line) => {
  const text = line.trim();
  if (text === "") return;
  const msg = JSON.parse(text);
  const reply = (body) => process.stdout.write(JSON.stringify({ id: msg.id, ...body }) + "\\n");
  if (msg.op === "shutdown") {
    reply({ ok: true, op: "shutdown" });
    setTimeout(() => process.exit(0), 5);
    return;
  }
  if (msg.op === "ping") {
    reply({ ok: true, op: "ping" });
    return;
  }
  if (msg.op === "summary") {
    reply({
      ok: true,
      op: "summary",
      summary: {
        stage: "coordinate",
        ending: false,
        verifyPassed: false,
        nextEndsRun: false,
        pendingProposals: 1,
        requirementCount: 2,
        conflicts: [],
        pendingTools: 0,
        jobs: [{ id: "r1", rev: 1, stage: "plan", lane: 0, branch: "", base: "", title: "t", description: "d" }],
        gates: [{ id: "r1", gate: "plan", rev: 1, title: "t" }],
      },
      pendingChoice: null,
    });
    return;
  }
  if (msg.op === "dispatch") {
    reply({ ok: true, op: "dispatch", narrative: ["[STEP] " + msg.event, "second line"], pendingChoice: null, tools: [] });
    return;
  }
  if (msg.op === "vfs_put") {
    reply({ ok: true, op: "vfs_put", path: msg.path, bytes: msg.content.length, sha256: "a".repeat(64) });
    return;
  }
  if (msg.op === "vfs_get") {
    reply({ ok: true, op: "vfs_get", path: msg.path, bytes: 2, sha256: "b".repeat(64), content: "hi" });
    return;
  }
  if (msg.op === "vfs_list") {
    reply({ ok: true, op: "vfs_list", artifacts: ["artifacts/r1-plan.md"] });
    return;
  }
  reply({ ok: false, op: msg.op, error: "unknown op" });
});
`;

  const dirs: string[] = [];
  after(() => {
    for (const dir of dirs) {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  function makeDaemon(): YggDaemon {
    const dir = mkdtempSync(path.join(os.tmpdir(), "ygg-harness-test-"));
    dirs.push(dir);
    // 用真脚本文件而不是 `-e`：node 在 `-e` 之后仍会解析以 `--` 开头的参数，
    // 脚本文件则把 `--daemon …` 原样交给 `process.argv`（与真宿主同形）。
    const script = path.join(dir, "fake-daemon.cjs");
    writeFileSync(script, fakeDaemonScript, "utf8");
    return new YggDaemon({
      cwd: dir,
      runId: "test-run",
      runsRoot: path.join(dir, ".ygg", "runs"),
      modDir: path.join(dir, ".ygg", "mods", "pi-agent-seed"),
      agentBin: path.join(dir, ".ygg", "bin", "ygg-agent"),
      spawnBin: process.execPath,
      spawnArgs: [script],
      requestTimeoutMs: 5_000,
    });
  }

  it("ping / summary / dispatch / vfs 三个动作都按 id 配对（串行）", async () => {
    const daemon = makeDaemon();
    try {
      const mode = await daemon.start();
      assert.equal(mode, "daemon");

      const summary = await daemon.summary();
      assert.equal(summary.stage, "coordinate");
      assert.equal(summary.jobs.length, 1);
      assert.equal(summary.jobs[0].id, "r1");
      assert.equal(summary.gates[0].gate, "plan");

      const step = await daemon.dispatch("requirement.dispatch", "");
      assert.equal(step.ok, true);
      assert.deepEqual(step.narrative, ["[STEP] requirement.dispatch", "second line"]);

      const put = await daemon.vfsPut("artifacts/r1-plan.md", "hello");
      assert.equal(put.ok, true);
      assert.equal(put.bytes, 5);
      assert.equal(put.sha256, "a".repeat(64));

      const got = await daemon.vfsGet("artifacts/r1-plan.md");
      assert.equal(got.content, "hi");

      const listed = await daemon.vfsList();
      assert.deepEqual(listed.artifacts, ["artifacts/r1-plan.md"]);
    } finally {
      await daemon.closeAll();
    }
  });

  it("并发请求被串行化且各自拿到自己的回执", async () => {
    const daemon = makeDaemon();
    try {
      await daemon.start();
      const [a, b, c] = await Promise.all([
        daemon.dispatch("a"),
        daemon.dispatch("b"),
        daemon.dispatch("c"),
      ]);
      assert.equal(a.narrative[0], "[STEP] a");
      assert.equal(b.narrative[0], "[STEP] b");
      assert.equal(c.narrative[0], "[STEP] c");
    } finally {
      await daemon.closeAll();
    }
  });

  it("closeAll 幂等；超时/进程不可用不假装成功", async () => {
    const daemon = makeDaemon();
    await daemon.start();
    await daemon.closeAll();
    await daemon.closeAll();

    const broken = new YggDaemon({
      cwd: os.tmpdir(),
      runId: "broken",
      runsRoot: path.join(os.tmpdir(), ".ygg", "runs"),
      modDir: path.join(os.tmpdir(), ".ygg", "mods", "pi-agent-seed"),
      agentBin: path.join(os.tmpdir(), "definitely-missing-ygg-agent"),
      requestTimeoutMs: 2_000,
    });
    const mode = await broken.start();
    assert.equal(mode, "cli", "起不来的宿主必须降级为单发，而不是假装 daemon 可用");
    const summary = await broken.summary();
    assert.equal(summary.available, false);
    assert.match(summary.degraded ?? "", /单发降级/);
    await broken.closeAll();
  });

  it("磁盘上二进制文件的 mtimeMs / size 变化时在下一次请求前自动重启 daemon 子进程", async () => {
    const dir = mkdtempSync(path.join(os.tmpdir(), "ygg-hotreload-test-"));
    dirs.push(dir);
    const script = path.join(dir, "fake-daemon-pid.cjs");
    const binFile = path.join(dir, "fake-ygg-agent.bin");
    writeFileSync(binFile, "v1-binary-bytes", "utf8");
    writeFileSync(
      script,
      `
const readline = require("node:readline");
const rl = readline.createInterface({ input: process.stdin });
function reply(obj) { process.stdout.write(JSON.stringify(obj) + "\\n"); }
rl.on("line", (line) => {
  const msg = JSON.parse(line);
  if (msg.op === "ping") { reply({ id: msg.id, ok: true, op: "ping", pid: process.pid }); return; }
  if (msg.op === "shutdown") { reply({ id: msg.id, ok: true, op: "shutdown" }); process.exit(0); }
  if (msg.op === "dispatch") {
    reply({ id: msg.id, ok: true, op: "dispatch", narrative: ["pid=" + process.pid], pendingChoice: null, tools: [] });
    return;
  }
  reply({ id: msg.id, ok: true });
});
`,
      "utf8",
    );
    const daemon = new YggDaemon({
      cwd: dir,
      runId: "hotreload-run",
      runsRoot: path.join(dir, ".ygg", "runs"),
      modDir: path.join(dir, ".ygg", "mods", "pi-agent-seed"),
      agentBin: binFile,
      spawnBin: process.execPath,
      spawnArgs: [script],
      requestTimeoutMs: 5_000,
    });
    try {
      await daemon.start();
      const first = await daemon.dispatch("step.one");
      const firstPid = first.narrative[0];
      assert.match(firstPid, /^pid=\d+$/);

      // 未改二进制：复用同一个 daemon 进程（PID 不变）
      const same = await daemon.dispatch("step.two");
      assert.equal(same.narrative[0], firstPid);

      // 覆盖二进制（改变 size 与 mtimeMs）：下一次请求自动重启子进程（PID 变化）
      writeFileSync(binFile, "v2-binary-bytes-rebuilt-longer-payload", "utf8");
      const afterReload = await daemon.dispatch("step.three");
      const secondPid = afterReload.narrative[0];
      assert.match(secondPid, /^pid=\d+$/);
      assert.notEqual(secondPid, firstPid, "二进制变更后应自动 respawn 新的子进程");
    } finally {
      await daemon.closeAll();
    }
  });
});

describe("daemon: .env 只暴露两项", () => {
  it("parseEnvFile 处理注释 / export / 引号", () => {
    const env = parseEnvFile(
      ["# comment", "export A=1", 'B="two words"', "C='x=y'", "EMPTY=", "not a pair"].join("\n"),
    );
    assert.equal(env.get("A"), "1");
    assert.equal(env.get("B"), "two words");
    assert.equal(env.get("C"), "x=y");
    assert.equal(env.get("EMPTY"), "");
    assert.equal(env.size, 4);
  });

  it("subagentSettings 只返回 YGG_SUBAGENT_* 两项", () => {
    const env = parseEnvFile(
      ["YGG_SUBAGENT_MODEL=anthropic/claude", "YGG_SUBAGENT_THINKING=high", "SECRET_TOKEN=should-not-leak"].join("\n"),
    );
    const tuning = subagentSettings(env);
    assert.deepEqual(tuning, { model: "anthropic/claude", thinking: "high" });
    assert.equal(JSON.stringify(tuning).includes("should-not-leak"), false);
    // 清单面只有键名 + 长度
    const keys = describeEnvKeys(env);
    assert.equal(keys.some((entry) => entry.startsWith("SECRET_TOKEN(")), true);
    assert.equal(keys.some((entry) => entry.includes("should-not-leak")), false);
  });

  it("空值时两项都不出现", () => {
    assert.deepEqual(subagentSettings(parseEnvFile("YGG_SUBAGENT_MODEL=\n")), {});
  });
});

describe("daemon: 摘要与回执呈现", () => {
  it("parseSummary 容忍缺字段（旧 Mod）并给出保守默认值", () => {
    const summary = parseSummary({ stage: "coordinate" });
    assert.equal(summary.available, true);
    assert.equal(summary.stage, "coordinate");
    assert.deepEqual(summary.jobs, []);
    assert.deepEqual(summary.gates, []);
    assert.equal(summary.ending, false);
  });

  it("非对象摘要 → available=false（不猜）", () => {
    assert.equal(parseSummary(null).available, false);
    assert.equal(parseSummary("nope").available, false);
  });

  it("toStepReceipt 保留全部 narrative 行与 pendingChoice", () => {
    const receipt = toStepReceipt({
      ok: true,
      narrative: ["a", "b", "c"],
      pendingChoice: { prompt: "选", options: ["1", "2"] },
      tools: [{ id: "t1" }],
    });
    assert.deepEqual(receipt.narrative, ["a", "b", "c"]);
    assert.deepEqual(receipt.pendingChoice, { prompt: "选", options: ["1", "2"] });
    assert.equal(receipt.tools.length, 1);
  });

  it("toStepReceipt 缺 summary 时带上传输错误", () => {
    const receipt = toStepReceipt(null, "summary", "daemon 请求超时");
    assert.equal(receipt.ok, false);
    assert.equal(receipt.error, "daemon 请求超时");
  });

  it("formatReceipt 不截断任何一行", () => {
    const long = "x".repeat(500);
    const receipt = toStepReceipt({ ok: true, narrative: [long, "tail"], pendingChoice: null });
    const text = formatReceipt("step", receipt);
    assert.equal(text.includes(long), true);
    assert.equal(text.includes("tail"), true);
    assert.equal(text.includes("pendingChoice"), false);
  });

  it("statusLine 是三段式页脚", () => {
    assert.equal(statusLine("demo", summaryWith([jobRow({ id: "r1" }), jobRow({ id: "r2" })])), "[Ygg: demo | coordinate | jobs:2 gates:0]");
    assert.equal(statusLine("demo", null), "[Ygg: demo | unknown | jobs:0 gates:0]");
  });
});

describe("worktree claim 之后 subagent 才能拿到隔离目录", () => {
  it("claim 未绑定时仍然派活，但回执记录未确认", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r2", stage: "development" })], false);
    const logs: string[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async () => ({ text: "done", ok: true }),
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        resolveBase: () => "abc",
        readArtifact: () => null,
        lanePollMs: 1,
        laneWaitMs: 10,
        logger: (line) => logs.push(line),
      },
    );
    assert.equal(result.jobs.length, 1);
    assert.equal(logs.some((line) => line.includes("泳道绑定未在超时内确认")), true);
    assert.equal(result.jobs[0].lane, 1, "分配结果仍然记账（不吞）");
  });
});

// ───────────────────────────── ④ 上下文压力 / 稳定节点 / 交接单 ─────────────────────────────

describe("evaluateContextPressure", () => {
  it("水位以下 → normal：既不刹车也不交接", () => {
    const pressure = evaluateContextPressure({ usedTokens: 64_000, contextWindow: 128_000 });
    assert.equal(pressure.level, "normal");
    assert.equal(pressure.ratio, 0.5);
    assert.equal(pressure.shouldBrakeSubagents, false);
    assert.equal(pressure.shouldHandoffAndCompact, false);
    assert.equal(formatPercent(pressure.ratio), "50.0%");
  });

  it(">=60% 软刹车，>=75% 硬刹车（边界含等号）", () => {
    const soft = evaluateContextPressure({ usedTokens: 76_800, contextWindow: 128_000 });
    assert.equal(soft.level, "soft_brake");
    assert.equal(soft.shouldBrakeSubagents, true);
    assert.equal(soft.shouldHandoffAndCompact, true);

    const hard = evaluateContextPressure({ usedTokens: 96_000, contextWindow: 128_000 });
    assert.equal(hard.level, "hard_brake");

    // 溢出（> 100%）也是硬刹车，ratio 如实 > 1
    const over = evaluateContextPressure({ usedTokens: 200_000, contextWindow: 128_000 });
    assert.equal(over.level, "hard_brake");
    assert.equal(over.ratio > 1, true);
  });

  it("用量未知（null / undefined / NaN）按「不知道」处理：不刹车", () => {
    for (const usedTokens of [null, undefined, Number.NaN, 0, -1]) {
      const pressure = evaluateContextPressure({ usedTokens, contextWindow: 128_000 });
      assert.equal(pressure.level, "normal", `usedTokens=${String(usedTokens)} 不该刹车`);
      assert.equal(pressure.usedTokens, 0);
      assert.equal(pressure.shouldBrakeSubagents, false);
    }
  });

  it("窗口非法退回默认 128000；hard < soft 时抬到 soft（保持单调）", () => {
    const fallback = evaluateContextPressure({ usedTokens: 96_000, contextWindow: 0 });
    assert.equal(fallback.contextWindow, 128_000);
    assert.equal(fallback.level, "hard_brake");

    const clamped = evaluateContextPressure({
      usedTokens: 108_800, // 0.85
      contextWindow: 128_000,
      softWatermark: 0.8,
      hardWatermark: 0.5,
    });
    assert.equal(clamped.level, "hard_brake", "hard 被抬到 0.8，0.85 仍是硬刹车");

    const custom = evaluateContextPressure({
      usedTokens: 108_800,
      contextWindow: 128_000,
      softWatermark: 0.9,
      hardWatermark: 0.95,
    });
    assert.equal(custom.level, "normal", "自定义水位生效");
  });

  it("1M 模型窗口默认按 DEFAULT_EFFECTIVE_WINDOW_CAP (200_000) 封顶计算水位，支持自定义 effectiveWindowCap", () => {
    assert.equal(DEFAULT_EFFECTIVE_WINDOW_CAP, 200_000);

    // 1M 窗口下用掉 120k：按 200k 封顶即 60.0% → soft_brake（若不封顶只有 12.0% 会漏刹）
    const softAt120k = evaluateContextPressure({
      usedTokens: 120_000,
      contextWindow: 1_000_000,
    });
    assert.equal(softAt120k.contextWindow, 200_000);
    assert.equal(softAt120k.ratio, 0.6);
    assert.equal(softAt120k.level, "soft_brake");
    assert.equal(softAt120k.shouldBrakeSubagents, true);

    // 1M 窗口下用掉 150k：按 200k 封顶即 75.0% → hard_brake
    const hardAt150k = evaluateContextPressure({
      usedTokens: 150_000,
      contextWindow: 1_000_000,
    });
    assert.equal(hardAt150k.contextWindow, 200_000);
    assert.equal(hardAt150k.ratio, 0.75);
    assert.equal(hardAt150k.level, "hard_brake");

    // 自定义 effectiveWindowCap（如 YGG_CONTEXT_WINDOW=300000）覆盖默认 200k 上限
    const customCap = evaluateContextPressure({
      usedTokens: 150_000,
      contextWindow: 1_000_000,
      effectiveWindowCap: 300_000,
    });
    assert.equal(customCap.contextWindow, 300_000);
    assert.equal(customCap.ratio, 0.5);
    assert.equal(customCap.level, "normal");

    // 显式传 null 或 <= 0 则不封顶，保留原始 contextWindow
    const uncapped = evaluateContextPressure({
      usedTokens: 150_000,
      contextWindow: 1_000_000,
      effectiveWindowCap: null,
    });
    assert.equal(uncapped.contextWindow, 1_000_000);
    assert.equal(uncapped.ratio, 0.15);
    assert.equal(uncapped.level, "normal");

    assert.equal(parsePositiveIntEnv("200000"), 200_000);
    assert.equal(parsePositiveIntEnv("  250000 "), 250_000);
    assert.equal(parsePositiveIntEnv(""), undefined);
    assert.equal(parsePositiveIntEnv("0"), undefined);
    assert.equal(parsePositiveIntEnv("-100"), undefined);
    assert.equal(parsePositiveIntEnv("abc"), undefined);
  });
});

describe("isStableCheckpoint", () => {
  it("门禁上 / 空闲边界 / 冲突边界都是稳定节点", () => {
    const atGate = summaryWith([jobRow({ id: "r1", stage: "plan" })], [
      { id: "r1", gate: "plan", rev: 1, title: "方案" },
    ]);
    assert.equal(isStableCheckpoint(atGate), true);
    assert.match(stableCheckpointReason(atGate), /停在待审门上/);

    const idle = summaryWith([]);
    assert.equal(isStableCheckpoint(idle), true);
    assert.match(stableCheckpointReason(idle), /干净阶段边界/);

    const conflicted = summaryWith(
      [jobRow({ id: "r2", stage: "development" })],
      [],
      { conflicts: [{ id: "r2", peer: "r3", note: "同文件 feature/x.ts" }] },
    );
    assert.equal(isStableCheckpoint(conflicted), true);
    assert.match(stableCheckpointReason(conflicted), /冲突边界/);
  });

  it("工具在飞 / 作业在飞且无门 → 不是稳定节点", () => {
    const pending = summaryWith([], [], { pendingTools: 2 });
    assert.equal(isStableCheckpoint(pending), false);
    assert.match(stableCheckpointReason(pending), /工具仍在飞/);

    const inflight = summaryWith([jobRow({ id: "r2", stage: "development", lane: 2 })]);
    assert.equal(isStableCheckpoint(inflight), false);
    assert.match(stableCheckpointReason(inflight), /作业在飞且没有门禁/);
  });

  it("摘要不可用 → 不是稳定节点（不猜）", () => {
    const unknown = unknownSummary("没有 meta.summary");
    assert.equal(isStableCheckpoint(unknown), false);
    assert.match(stableCheckpointReason(unknown), /摘要不可用/);
  });
});

describe("buildHandoffMarkdown", () => {
  const summary = summaryWith(
    [
      jobRow({
        id: "r2",
        rev: 2,
        stage: "development",
        status: "developing",
        phase: "developing",
        lane: 2,
        branch: "feature/r2-x",
        base: "abc1234",
        title: "接泳道",
        executor: "agent-7",
        planExecutor: "agent-3",
      }),
    ],
    [{ id: "r1", gate: "plan", rev: 2, title: "方案一" }],
  );
  const markdown = buildHandoffMarkdown({
    runId: "demo",
    summary,
    pressure: evaluateContextPressure({ usedTokens: 96_000, contextWindow: 128_000 }),
    artifacts: ["artifacts/r1-plan.md", "artifacts/r2-development.md"],
    reason: "soft_brake",
    now: new Date("2026-01-02T03:04:05.000Z"),
  });

  it("带上 run / 阶段 / 用量 / 时间 / 触发原因", () => {
    assert.match(markdown, /run：`demo`/);
    assert.match(markdown, /阶段：`coordinate`/);
    assert.match(markdown, /96000 \/ 128000 \(75\.0%\)/);
    assert.match(markdown, /水位 hard_brake/);
    assert.match(markdown, /2026-01-02T03:04:05\.000Z/);
    assert.match(markdown, /触发：soft_brake/);
    assert.match(markdown, /jobs=1 · gates=1 · conflicts=0/);
  });

  it("作业表带 exec 账本（executor / planExecutor）与泳道信息", () => {
    assert.match(
      markdown,
      /\| r2 \| development \| developing \| 2 \| 2 \| feature\/r2-x \| abc1234 \| agent-7 \| agent-3 \| 接泳道 \|/,
    );
  });

  it("门禁表与 VFS 制品列表逐条列出", () => {
    assert.match(markdown, /\| r1 \| plan \| 2 \| 方案一 \|/);
    assert.match(markdown, /artifacts\/r1-plan\.md/);
    assert.match(markdown, /artifacts\/r2-development\.md/);
    assert.equal(markdown.includes(HANDOFF_ARTIFACT_PATH), true);
  });

  it("下一步是精确到命令的：先门禁再派活", () => {
    assert.match(markdown, /vfs_read/);
    assert.match(markdown, /approve_gate/);
    assert.match(markdown, /ygg_run_lanes\(\{ runId: "demo" \}\)/);
    assert.match(markdown, /不要重复派活/);
  });

  it("标题里的竖线被转义（表格不被撑破）", () => {
    const piped = buildHandoffMarkdown({
      runId: "demo",
      summary: summaryWith([jobRow({ id: "r1", stage: "plan", title: "方案 | 甲" })]),
    });
    assert.equal(piped.includes("方案 \\| 甲"), true);
  });

  it("摘要不可用：明说表格是空的，并给 map.show 的下一步", () => {
    const unknown = buildHandoffMarkdown({
      runId: "demo",
      summary: unknownSummary("宿主没给 meta.summary"),
    });
    assert.match(unknown, /摘要不可用：宿主没给 meta\.summary/);
    assert.match(unknown, /先 `ygg_session\(\{ action: "dispatch", event: "map\.show" \}\)` 重新取一次现场/);
    assert.doesNotMatch(unknown, /`step /, "交接单不得再用已退场的 `step` 别名给出下一步");
    assert.match(unknown, /event: "map\.show"/);
  });

  it("review 阶段的下一步是 settle_close", () => {
    const review = buildHandoffMarkdown({
      runId: "demo",
      summary: summaryWith([], [], { stage: "review" }),
    });
    assert.match(review, /settle_close/);
  });
});

describe("writeHandoffToVfs", () => {
  it("写进固定路径 artifacts/handoff-latest.md（内容 = 同一份 markdown）", async () => {
    const written: { path: string; content: string }[] = [];
    const daemon = {
      vfsPut: async (artifactPath: string, content: string) => {
        written.push({ path: artifactPath, content });
        return { ok: true, bytes: content.length, sha256: "a".repeat(64) };
      },
    };
    const write = await writeHandoffToVfs(daemon, { runId: "demo", summary: summaryWith([]) });
    assert.equal(write.ok, true);
    assert.equal(write.path, "artifacts/handoff-latest.md");
    assert.equal(written.length, 1);
    assert.equal(written[0].path, HANDOFF_ARTIFACT_PATH);
    assert.equal(written[0].content, write.markdown);
    assert.equal(write.sha256?.length, 64);
  });

  it("vfsPut 被拒 → ok=false 且如实带回原因（不自称写过）", async () => {
    const daemon = { vfsPut: async () => ({ ok: false, error: "路径形状非法" }) };
    const write = await writeHandoffToVfs(daemon, { runId: "demo", summary: summaryWith([]) });
    assert.equal(write.ok, false);
    assert.equal(write.error, "路径形状非法");
    assert.equal(write.markdown.length > 0, true, "即使写失败也要有正文可续跑");
  });
});

describe("压缩后的续跑消息", () => {
  it("续跑消息自带 run / 交接单 / 下一步（压完只剩它）", () => {
    const summary = summaryWith([]);
    const markdown = buildHandoffMarkdown({ runId: "demo", summary });
    const message = buildResumeMessage("demo", summary, markdown);
    assert.equal(HANDOFF_RESUME_CUSTOM_TYPE, "ygg-handoff-resume");
    assert.match(message, /上下文已在检查点压缩/);
    assert.equal(message.includes(markdown.trim()), true);
    assert.match(message, /下一步（从这里接）/);
    assert.match(message, /门禁（approve \/ reject）是人的决定/);
  });

  it("我们自己触发的压缩：指令要求摘要逐字保留状态块", () => {
    const markdown = buildHandoffMarkdown({ runId: "demo", summary: summaryWith([]) });
    const instructions = buildCompactInstructions("demo", markdown);
    assert.match(instructions, /检查点压缩/);
    assert.match(instructions, /必须原样保留/);
    assert.equal(instructions.includes(markdown.trim()), true);
    assert.match(instructions, /===== 必须原样保留的状态块（开始） =====/);
  });
});

// ───────────────────────────── ⑤ subagent create + resume 生命周期 ─────────────────────────────

describe("extractAgentId", () => {
  it("优先读 details 里的结构化 id", () => {
    assert.equal(extractAgentId("随便一段话", { agentId: "agent-7" }), "agent-7");
    assert.equal(extractAgentId("", { agent_id: "agent-8" }), "agent-8");
    assert.equal(extractAgentId("", { id: "agent-9" }), "agent-9");
  });

  it("退回正文里的 `Agent ID:`（后台运行的写法）", () => {
    const text = "Agent started in background.\nAgent ID: agent-abc_1\nUse get_subagent_result…";
    assert.equal(extractAgentId(text), "agent-abc_1");
  });

  it("都没有 → undefined（不猜 id）", () => {
    assert.equal(extractAgentId("报告正文", { status: "completed" }), undefined);
    assert.equal(extractAgentId("", null), undefined);
  });
});

describe("RESERVED_AGENT_IDS：占位黑名单单一来源（parity）", () => {
  it("扩展侧黑名单与 Mod 侧 render.lua is_resumable_agent_id 逐字一致", () => {
    // 单一来源（P1-1）：这串常量必须与 `.ygg/mods/pi-agent-seed/src/render.lua`
    // `is_resumable_agent_id` 里的字面量（unregistered / pi-ygg-harness / main / default）
    // 保持一致。单侧加占位符（如未来的 `pi-dsh-harness`）→ 本用例红。
    const modSideReserved = ["unregistered", "pi-ygg-harness", "main", "default"];
    assert.deepEqual([...RESERVED_AGENT_IDS].sort(), [...modSideReserved].sort());
    // 每个成员都必须被 isResumableAgentId 拒收（集合与实际守卫行为对齐，不空转）。
    for (const id of modSideReserved) {
      assert.equal(isResumableAgentId(id), false, `占位 ID ${id} 必须不可续跑`);
    }
    // mutation probe：一个真实 ID 不能被误拒。
    assert.equal(isResumableAgentId("agent-real-7"), true);
  });
});

describe("resolveResumeId（续谁）", () => {  it("job.executor 非空 → 同阶段重派续原执行者", () => {
    const job = jobRow({ id: "r3", stage: "plan", rev: 2, executor: "agent-42" });
    assert.equal(resolveResumeId(job), "agent-42");
    assert.match(resumeReason(job, false), /同阶段重派/);
  });

  it("plan → development 显式允许时续方案执行者", () => {
    const job = jobRow({ id: "r4", stage: "development", planExecutor: "agent-plan" });
    assert.equal(resolveResumeId(job, { resumePlanAgent: true }), "agent-plan");
    assert.equal(resolveResumeId(job, { resumePlanAgent: false }), "", "没开开关就新起一个");
    assert.match(resumeReason(job, true), /跨阶段续跑/);
  });

  it("没登记 → 空串（新起一个 subagent）", () => {
    assert.equal(resolveResumeId(jobRow()), "");
    assert.match(resumeReason(jobRow(), false), /新起一个/);
  });
});

describe("actorForLedger（exec= 段的 id）", () => {
  it("正常 id 直接用；空白 / 空串 / 超长一律退回默认执行者", () => {
    assert.equal(actorForLedger("agent-7", "pi-ygg-harness"), "agent-7");
    assert.equal(actorForLedger(" agent-7 ", "pi-ygg-harness"), "agent-7");
    assert.equal(actorForLedger("agent 7", "pi-ygg-harness"), "pi-ygg-harness", "空白会被 Mod 拒掉整条提案");
    assert.equal(actorForLedger("", "pi-ygg-harness"), "pi-ygg-harness");
    assert.equal(actorForLedger(undefined, "pi-ygg-harness"), "pi-ygg-harness");
    assert.equal(actorForLedger("a".repeat(65), "pi-ygg-harness"), "pi-ygg-harness");
  });
});

describe("parseLaneProbeNarrative / laneProbeNeedsCommit", () => {
  it("读 commits / clean，取最后一次出现的值", () => {
    const verdict = parseLaneProbeNarrative([
      "[LANE PROBE] lane-2 commits=3 clean=1 head=abc files=src/a.ts",
      "[LANE PROBE FAILED] lane-2 commits=0 clean=0",
    ]);
    assert.deepEqual(verdict, { known: true, commits: 0, clean: false });
    assert.equal(laneProbeNeedsCommit(verdict), true);
  });

  it("读不到 → known=false，不触发自愈", () => {
    const verdict = parseLaneProbeNarrative(["[TOOL] git.lane_probe"]);
    assert.equal(verdict.known, false);
    assert.equal(laneProbeNeedsCommit(verdict), false);
    assert.equal(laneProbeNeedsCommit(UNKNOWN_LANE_PROBE), false);
  });

  it("commits=0 或 clean=0 任一为真都算「没提交」", () => {
    assert.equal(laneProbeNeedsCommit({ known: true, commits: 0, clean: true }), true);
    assert.equal(laneProbeNeedsCommit({ known: true, commits: 2, clean: false }), true);
    assert.equal(laneProbeNeedsCommit({ known: true, commits: 2, clean: true }), false);
    assert.equal(laneProbeNeedsCommit({ known: true, commits: null, clean: null }), false);
  });

  it("本地 git 探测读不到工作树 → unknown（宁可不报）", () => {
    assert.deepEqual(readLaneProbeFromGit(null, "abc"), UNKNOWN_LANE_PROBE);
    assert.deepEqual(
      readLaneProbeFromGit(path.join(os.tmpdir(), "definitely-not-a-worktree-ygg"), "abc"),
      UNKNOWN_LANE_PROBE,
    );
  });
});

describe("orchestrateActiveJobs：subagent create + resume", () => {
  it("create：没登记执行者就不带 resume，并把真实 agentId 记进 exec= 段", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r2", stage: "development", lane: 2, branch: "feature/r2-x" })]);
    const seen: SubagentRequest[] = [];
    await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        return { text: "实现完成", ok: true, agentId: "agent-77" };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        readArtifact: () => null,
        readLaneProbe: () => UNKNOWN_LANE_PROBE,
      },
    );
    assert.equal(seen.length, 1);
    assert.equal(seen[0].resume, undefined, "没登记执行者时是新起一个");
    const submit = daemon.calls.find((call) => call.event === "proposal.submit");
    const segments = String(submit?.payload ?? "").split("|");
    assert.equal(segments[7].startsWith("exec=agent-77:"), true, "账本要写真实 subagent id");
  });

  it("gate 打回后同阶段重派 → 自动 resume job.executor", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r3", stage: "plan", rev: 2, executor: "agent-42" })]);
    const seen: SubagentRequest[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        return { text: "修好重提", ok: true, agentId: "agent-42" };
      },
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(seen[0].resume, "agent-42");
    assert.equal(
      result.narrative.some((line) => line.includes("[RESUME] r3 plan") && line.includes("agent-42")),
      true,
      "续跑这件事要留在回执里",
    );
  });

  it("plan → development 只在开 resumePlanAgent 时续跑", async () => {
    const job = jobRow({
      id: "r4",
      stage: "development",
      lane: 1,
      branch: "feature/r4-x",
      planExecutor: "agent-plan",
    });
    const times: (string | undefined)[] = [];
    const run = async (resumePlanAgent: boolean) => {
      const daemon = new MockDaemon([{ ...job }]);
      await orchestrateActiveJobs(
        daemon,
        async (request) => {
          times.push(request.resume);
          return { text: "实现完成", ok: true, agentId: "agent-new" };
        },
        {
          cwd: "/tmp/repo",
          repoDir: "/tmp/repo",
          readArtifact: () => null,
          readLaneProbe: () => UNKNOWN_LANE_PROBE,
          resumePlanAgent,
        },
      );
    };
    await run(true);
    await run(false);
    assert.deepEqual(times, ["agent-plan", undefined]);
  });

  it("流断开（ok=false）→ 用 agentId 自愈续跑一次，报告取重试结果", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r5", stage: "plan" })]);
    const seen: SubagentRequest[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        if (seen.length === 1) {
          return { text: "部分输出", ok: false, error: "stream 断开", agentId: "agent-9" };
        }
        return { text: "重试后完整方案", ok: true, agentId: "agent-9" };
      },
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(seen.length, 2);
    assert.equal(seen[1].resume, "agent-9");
    assert.match(result.jobs[0].report, /重试后完整方案/);
    assert.equal(
      result.narrative.some((line) => line.includes("[SELF-HEAL] r5") && line.includes("agent-9")),
      true,
    );
  });

  it("失败且不知道续谁 → 不重试（不猜 id）", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r6", stage: "plan" })]);
    const seen: SubagentRequest[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        return { text: "", ok: false, error: "崩了" };
      },
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(seen.length, 1);
    assert.equal(result.jobs[0].error, "崩了");
  });

  it("探针报 commits=0 → 续跑同一 subagent 补提交，然后复跑探针", async () => {
    const job = jobRow({
      id: "r7",
      stage: "development",
      lane: 3,
      branch: "feature/r7-x",
      status: "developing",
      phase: "developing",
    });
    const daemon = new MockDaemon([job]);
    const seen: SubagentRequest[] = [];
    let reads = 0;
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        return request.resume === undefined
          ? { text: "实现完成", ok: true, agentId: "agent-5" }
          : { text: "已 git add + commit", ok: true, agentId: "agent-5" };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        readArtifact: () => null,
        readLaneProbe: () => {
          reads += 1;
          return reads === 1
            ? { known: true, commits: 0, clean: true }
            : { known: true, commits: 1, clean: true };
        },
      },
    );

    assert.equal(seen.length, 2, "第二次是自愈续跑");
    assert.equal(seen[1].resume, "agent-5");
    assert.equal(seen[1].prompt, COMMIT_NUDGE_TEXT);
    const probes = daemon.claimCalls().filter((call) => call.startsWith("git.lane_probe|"));
    assert.equal(probes.length, 2, "补提交后要复跑探针");
    assert.equal(
      result.narrative.some((line) => line.includes("已续跑要求补提交")),
      true,
    );
    assert.equal(
      result.narrative.some((line) => line.includes("补提交后 commits=1")),
      true,
    );
    const archived = daemon.vfs.filter((entry) => entry.path === "artifacts/r7-development.md").pop();
    assert.match(archived?.content ?? "", /自愈补提交/);
  });

  it("探针真值未知 → 不催提交（也没有 agentId 时更不该动）", async () => {
    const daemon = new MockDaemon([
      jobRow({ id: "r8", stage: "development", lane: 4, branch: "feature/r8-x" }),
    ]);
    let spawned = 0;
    await orchestrateActiveJobs(
      daemon,
      async () => {
        spawned += 1;
        return { text: "实现完成", ok: true, agentId: "agent-8" };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        readArtifact: () => null,
        readLaneProbe: () => UNKNOWN_LANE_PROBE,
      },
    );
    assert.equal(spawned, 1);
    assert.equal(
      daemon.claimCalls().filter((call) => call.startsWith("git.lane_probe|")).length,
      1,
    );
  });
});

describe("上下文压力刹车（编排层）", () => {
  it("硬刹车：一个 subagent 都不派、不建 worktree，先写交接单进 VFS", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r2", stage: "development" })]);
    let spawned = 0;
    const result = await orchestrateActiveJobs(
      daemon,
      async () => {
        spawned += 1;
        return { text: "不该跑", ok: true };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        runId: "demo",
        readArtifact: () => null,
        checkContextPressure: () =>
          evaluateContextPressure({ usedTokens: 100_000, contextWindow: 128_000 }),
      },
    );
    assert.equal(spawned, 0, "硬刹车一个都不派");
    assert.equal(daemon.claimCalls().length, 0, "硬刹车不建 worktree");
    assert.equal(result.contextBraked, true);
    assert.equal(result.jobs.length, 0);
    assert.equal(daemon.vfs.length, 1);
    assert.equal(daemon.vfs[0].path, HANDOFF_ARTIFACT_PATH);
    assert.match(daemon.vfs[0].content, /hard_brake/);
    assert.match(daemon.vfs[0].content, /run：`demo`/);
    assert.equal(result.handoff?.ok, true);
    assert.equal(
      result.narrative.some((line) => line.includes("[HANDOFF]")),
      true,
    );
  });

  it("软刹车：本波照跑并落在门上，落地后再写交接单（不在半路压）", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r2", stage: "plan" })]);
    const result = await orchestrateActiveJobs(
      daemon,
      async () => ({ text: "方案正文", ok: true, agentId: "agent-1" }),
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        runId: "demo",
        readArtifact: () => null,
        checkContextPressure: () =>
          evaluateContextPressure({ usedTokens: 83_200, contextWindow: 128_000 }),
      },
    );
    assert.equal(result.jobs.length, 1, "软刹车这一波要跑完");
    assert.equal(
      daemon.calls.some((call) => call.event === "proposal.accept"),
      true,
      "要落在门上才算稳定节点",
    );
    assert.equal(result.contextBraked, true);
    const handoff = daemon.vfs.find((entry) => entry.path === HANDOFF_ARTIFACT_PATH);
    assert.notEqual(handoff, undefined);
    assert.match(handoff?.content ?? "", /soft_brake/);
  });

  it("没接压力探针 → 不刹车、不写交接单", async () => {
    const daemon = new MockDaemon([jobRow({ id: "r2", stage: "plan" })]);
    const result = await orchestrateActiveJobs(
      daemon,
      async () => ({ text: "方案正文", ok: true }),
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(result.contextBraked, false);
    assert.equal(result.handoff, undefined);
    assert.equal(
      daemon.vfs.some((entry) => entry.path === HANDOFF_ARTIFACT_PATH),
      false,
    );
  });
});

describe("thin execution shell over core subagentAction & inline tools", () => {
  it("parseSummary / normalizeReceipt 解析 job.worktreePath、job.subagentAction 与 receipt.harnessActions", () => {
    const rawSummary = {
      stage: "coordinate",
      jobs: [
        {
          id: "r2",
          rev: 1,
          stage: "development",
          status: "developing",
          phase: "developing",
          lane: 2,
          branch: "feature/r2-thin",
          base: "2e24c45",
          title: "Thin shell",
          description: "Consume subagentAction",
          worktreePath: "/tmp/ygg-worktrees/repo-lane-2",
          subagentAction: {
            action: "subagent.spawn",
            reqId: "r2",
            rev: 1,
            stage: "development",
            lane: 2,
            worktreePath: "/tmp/ygg-worktrees/repo-lane-2",
            artifactPath: "artifacts/r2-custom-dev.md",
            brief: "# Core Brief for r2\nImplement thin shell.",
            submitCommand: "proposal.submit",
          },
        },
      ],
      gates: [],
    };
    const summary = parseSummary(rawSummary);
    assert.equal(summary.available, true);
    assert.equal(summary.jobs.length, 1);
    assert.equal(summary.jobs[0].worktreePath, "/tmp/ygg-worktrees/repo-lane-2");
    assert.equal(summary.jobs[0].subagentAction?.action, "subagent.spawn");
    assert.equal(summary.jobs[0].subagentAction?.reqId, "r2");
    assert.equal(summary.jobs[0].subagentAction?.artifactPath, "artifacts/r2-custom-dev.md");
    assert.equal(summary.harnessActions?.length, 1);
    assert.equal(summary.harnessActions?.[0].action, "subagent.spawn");

    const receipt = normalizeReceipt({
      ok: true,
      narrative: ["[DISPATCH] r2"],
      summary: rawSummary,
    });
    assert.equal(receipt.harnessActions?.length, 1);
    assert.equal(receipt.harnessActions?.[0].action, "subagent.spawn");
  });

  it("orchestrateActiveJobs：Core 已填好 job.worktreePath 或 subagentAction.worktreePath 时跳过 tool.call git.worktree_claim", async () => {
    const job = jobRow({
      id: "r2",
      stage: "development",
      lane: 1,
      branch: "feature/r2-thin",
      base: "2e24c45",
      worktreePath: "/tmp/ygg-worktrees/preclaimed-lane-1",
      subagentAction: {
        action: "subagent.spawn",
        reqId: "r2",
        rev: 1,
        stage: "development",
        lane: 1,
        worktreePath: "/tmp/ygg-worktrees/preclaimed-lane-1",
        brief: "# Core Preclaimed Brief",
      },
    });
    const daemon = new MockDaemon([job]);
    const seen: SubagentRequest[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async (req) => {
        seen.push(req);
        return { text: "thin shell done", ok: true, agentId: "agent-thin-1" };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        readArtifact: () => null,
        readLaneProbe: () => UNKNOWN_LANE_PROBE,
      },
    );
    const claimCalls = daemon
      .claimCalls()
      .filter((payload) => payload.startsWith("git.worktree_claim|"));
    assert.equal(claimCalls.length, 0, "Core 已绑好 worktree 时不得再发 git.worktree_claim");
    assert.equal(result.jobs[0].worktreePath, "/tmp/ygg-worktrees/preclaimed-lane-1");
    assert.equal(seen.length, 1);
  });

  it("orchestrateActiveJobs：消费 subagentAction.brief / deltaPrompt / resumeAgentId / artifactPath", async () => {
    const spawnJob = jobRow({
      id: "r1",
      stage: "plan",
      rev: 1,
      subagentAction: {
        action: "subagent.spawn",
        reqId: "r1",
        rev: 1,
        stage: "plan",
        lane: 0,
        artifactPath: "artifacts/r1-core-plan.md",
        brief: "# Core Spawn Brief\nUse core plan prompt.",
      },
    });
    const resumeJob = jobRow({
      id: "r2",
      stage: "development",
      rev: 2,
      lane: 2,
      branch: "feature/r2-dev",
      base: "2e24c45",
      worktreePath: "/tmp/ygg-worktrees/repo-lane-2",
      subagentAction: {
        action: "subagent.resume",
        reqId: "r2",
        rev: 2,
        stage: "development",
        lane: 2,
        resumeAgentId: "agent-core-resume-9",
        worktreePath: "/tmp/ygg-worktrees/repo-lane-2",
        artifactPath: "artifacts/r2-core-dev.md",
        deltaPrompt: "# Core Delta Prompt\nAddress gate rejection notes.",
      },
    });
    const daemon = new MockDaemon([spawnJob, resumeJob]);
    const byDesc = new Map<string, SubagentRequest>();
    const result = await orchestrateActiveJobs(
      daemon,
      async (req) => {
        byDesc.set(req.description, req);
        return { text: `# Done ${req.description}\nAll green`, ok: true, agentId: req.resume ?? "agent-spawn-1" };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        verificationCommands: ["node --test harness.test.ts"],
        readArtifact: () => null,
        readLaneProbe: () => UNKNOWN_LANE_PROBE,
      },
    );

    const r1Req = byDesc.get("r1 plan");
    const r2Req = byDesc.get("r2 development");
    assert.equal(r1Req?.resume, undefined);
    assert.match(r1Req?.prompt ?? "", /# Core Spawn Brief/);
    assert.match(r1Req?.prompt ?? "", /node --test harness\.test\.ts/);
    assert.equal(r2Req?.resume, "agent-core-resume-9");
    assert.match(r2Req?.prompt ?? "", /# Core Delta Prompt/);
    assert.match(r2Req?.prompt ?? "", /artifacts\/r2-core-dev\.md/);
    assert.deepEqual(
      result.jobs.map((j) => j.artifactPath).sort(),
      ["artifacts/r1-core-plan.md", "artifacts/r2-core-dev.md"],
    );
    assert.equal(
      parseSubagentOutput( ensureBriefVerificationAndFooter("# Title\nBody", "artifacts/x.md", ["cargo test"]) ).summary,
      "Title",
    );
    assert.match(
      buildResumeDeltaPrompt(resumeJob, "/tmp/ygg-worktrees/repo-lane-2", "artifacts/r2-core-dev.md", "/tmp/repo"),
      /续跑增量指令/,
    );
  });

  it("orchestrateActiveJobs：useInlineLaneProbe=true 时跳过 proposal.submit 前的显式 tool.call git.lane_probe", async () => {
    const job = jobRow({
      id: "r2",
      stage: "development",
      lane: 1,
      branch: "feature/r2-inline-probe",
      base: "2e24c45",
      worktreePath: "/tmp/ygg-worktrees/repo-lane-1",
    });
    const daemon = new MockDaemon([job]);
    await orchestrateActiveJobs(
      daemon,
      async () => ({ text: "implemented with inline probe", ok: true, agentId: "agent-inline" }),
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        useInlineLaneProbe: true,
        readArtifact: () => null,
      },
    );
    const probeCalls = daemon
      .claimCalls()
      .filter((payload) => payload.startsWith("git.lane_probe|"));
    assert.equal(probeCalls.length, 0, "useInlineLaneProbe=true 时不发显式 git.lane_probe");
    assert.equal(
      daemon.calls.some((call) => call.event === "proposal.submit"),
      true,
    );
  });

  it("isResumableAgentId 与 resolveResumeId 拒绝占位执行者（pi-ygg-harness / unregistered / main / default / 自定义 actorId），绝不把占位名传给 executeSubagent", async () => {
    assert.equal(isResumableAgentId(undefined), false);
    assert.equal(isResumableAgentId(""), false);
    assert.equal(isResumableAgentId("   "), false);
    assert.equal(isResumableAgentId("unregistered"), false);
    assert.equal(isResumableAgentId("pi-ygg-harness"), false);
    assert.equal(isResumableAgentId("main"), false);
    assert.equal(isResumableAgentId("default"), false);
    assert.equal(isResumableAgentId("custom-actor", "custom-actor"), false);
    assert.equal(isResumableAgentId("agent-real-42", "custom-actor"), true);

    const placeholderJob = jobRow({
      id: "r2",
      stage: "development",
      rev: 2,
      lane: 2,
      branch: "feature/r2-dev",
      base: "e768d4a",
      worktreePath: "/tmp/ygg-worktrees/repo-lane-2",
      executor: "pi-ygg-harness",
      planExecutor: "unregistered",
      subagentAction: {
        action: "subagent.resume",
        reqId: "r2",
        rev: 2,
        stage: "development",
        lane: 2,
        resumeAgentId: "pi-ygg-harness",
        worktreePath: "/tmp/ygg-worktrees/repo-lane-2",
      },
    });
    assert.equal(resolveResumeId(placeholderJob, { resumePlanAgent: true }), "");
    assert.equal(
      resolveResumeId(
        jobRow({ id: "r2", stage: "development", executor: "custom-actor" }),
        { actorId: "custom-actor" },
      ),
      "",
    );
    assert.match(resumeReason(placeholderJob, true), /新起一个/);

    const daemon = new MockDaemon([placeholderJob]);
    const seen: SubagentRequest[] = [];
    await orchestrateActiveJobs(
      daemon,
      async (req) => {
        seen.push(req);
        return { text: "fresh spawn succeeded", ok: true, agentId: "agent-fresh-99" };
      },
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        resumePlanAgent: true,
        readArtifact: () => null,
        readLaneProbe: () => UNKNOWN_LANE_PROBE,
      },
    );
    assert.equal(seen.length, 1);
    assert.equal(seen[0].resume, undefined, "占位执行者 ID 绝不能作为 resume 传给 executeSubagent");
  });

  it("parseSummary 解析 SummaryJob 上的 worktreeStatus、subagentStatus 与 probeStatus", () => {
    const summary = parseSummary({
      stage: "coordinate",
      jobs: [
        {
          id: "r2",
          rev: 1,
          stage: "development",
          status: "developing",
          phase: "developing",
          lane: 2,
          branch: "feature/r2",
          base: "e768d4a",
          title: "Live status",
          description: "Test live status fields",
          worktreePath: "/tmp/ygg-worktrees/repo-lane-2",
          worktreeStatus: "已绑定 /tmp/ygg-worktrees/repo-lane-2",
          subagentStatus: "已派发 agent-42",
          probeStatus: "commits=1 clean=1",
        },
      ],
      gates: [],
    });
    assert.equal(summary.available, true);
    assert.equal(summary.jobs.length, 1);
    assert.equal(summary.jobs[0].worktreeStatus, "已绑定 /tmp/ygg-worktrees/repo-lane-2");
    assert.equal(summary.jobs[0].subagentStatus, "已派发 agent-42");
    assert.equal(summary.jobs[0].probeStatus, "commits=1 clean=1");
  });
});

// ───────────────────────────── ⑤ index.ts：kickoff 默认 guide 与 clientFor 并发锁 ─────────────────────────────

async function loadIndexExports() {
  const { registerHooks } = await import("node:module");
  registerHooks({
    resolve(specifier, context, nextResolve) {
      if (
        specifier === "@earendil-works/pi-coding-agent" ||
        specifier === "@earendil-works/pi-ai" ||
        specifier === "typebox"
      ) {
        return {
          url:
            "data:text/javascript," +
            encodeURIComponent(`
              export const isToolCallEventType = () => false;
              export const StringEnum = (values, opts) => ({ type: "string", enum: values, ...opts });
              export const Type = {
                Object: (props, opts) => ({ type: "object", properties: props, ...opts }),
                Optional: (schema) => ({ ...schema, optional: true }),
                String: (opts) => ({ type: "string", ...opts }),
                Number: (opts) => ({ type: "number", ...opts }),
                Array: (item, opts) => ({ type: "array", items: item, ...opts }),
                Union: (anyOf, opts) => ({ anyOf, ...opts }),
              };
            `),
          shortCircuit: true,
        };
      }
      return nextResolve(specifier, context);
    },
  });
  return await import("./index.ts");
}

describe("index.ts: kickoff 模式选择与 clientFor 并发启动锁", () => {
  it("kickoff：goalAuth 为空时不派发 goal.authorization 与 play.mode=goal（保持默认 guide 模式）；非空时才一并派发", async () => {
    const { kickoffSession } = await loadIndexExports();
    const fakeCtx = {
      cwd: "/tmp/repo",
      ui: { setStatus() {}, notify() {} },
    } as never;

    const guideDispatches: [string, string | number | undefined][] = [];
    const guideClient = {
      async dispatch(event: string, payload?: string | number): Promise<StepReceipt> {
        guideDispatches.push([event, payload]);
        return { ok: true, narrative: [`[OK] ${event}`], pendingChoice: null, tools: [], raw: {} };
      },
      async summary(): Promise<RunSummary> {
        return summaryWith([]);
      },
    };

    await kickoffSession(fakeCtx, guideClient, {
      runId: "run-guide",
      title: "Guide Run",
      description: "Stay in default guide mode",
      goalAuth: "   ",
      requirements: [{ title: "r1", description: "desc" }],
    });
    assert.deepEqual(
      guideDispatches.map(([event]) => event),
      ["task.intake", "requirement.add", "requirement.dispatch"],
      "goalAuth 为空时不得硬编码切入 play.mode=goal",
    );

    const goalDispatches: [string, string | number | undefined][] = [];
    const goalClient = {
      async dispatch(event: string, payload?: string | number): Promise<StepReceipt> {
        goalDispatches.push([event, payload]);
        return { ok: true, narrative: [`[OK] ${event}`], pendingChoice: null, tools: [], raw: {} };
      },
      async summary(): Promise<RunSummary> {
        return summaryWith([]);
      },
    };

    await kickoffSession(fakeCtx, goalClient, {
      runId: "run-goal",
      title: "Goal Run",
      description: "User authorized goal mode",
      goalAuth: "user explicitly authorized goal mode",
      requirements: [{ title: "r1", description: "desc" }],
    });
    assert.deepEqual(goalDispatches, [
      ["config.set", "goal.authorization=user explicitly authorized goal mode"],
      ["config.set", "play.mode=goal"],
      ["task.intake", "Goal Run|User authorized goal mode"],
      ["requirement.add", "r1|desc"],
      ["requirement.dispatch", ""],
    ]);
  });

  it("clientFor：同一 runId 并发调用复用 daemonsInflight Promise，只创建并 start 一次", async () => {
    const { clientFor, resetClientsForTest, resolveEffectiveWindowCapFromEnv } = await loadIndexExports();
    resetClientsForTest();

    let createdCount = 0;
    let startCount = 0;
    const fakeCtx = {
      cwd: "/tmp/repo",
      ui: { setStatus() {}, notify() {} },
    } as never;

    const makeFakeDaemon = () => {
      createdCount += 1;
      const instance = {
        mode: "daemon" as const,
        degradedReason: "",
        async start() {
          startCount += 1;
          await new Promise((resolve) => setTimeout(resolve, 25));
          return "daemon" as const;
        },
        async closeAll() {},
      };
      return instance as unknown as YggDaemon;
    };

    const [c1, c2, c3] = await Promise.all([
      clientFor(fakeCtx, "run-race", { createDaemon: makeFakeDaemon }),
      clientFor(fakeCtx, "run-race", { createDaemon: makeFakeDaemon }),
      clientFor(fakeCtx, "run-race", { createDaemon: makeFakeDaemon }),
    ]);

    assert.equal(createdCount, 1, "并发调用同一 runId 只应构造一个 YggDaemon");
    assert.equal(startCount, 1, "并发调用同一 runId 只应调用一次 start()");
    assert.equal(c1, c2);
    assert.equal(c2, c3);

    // .env.local / process.env 的 YGG_CONTEXT_WINDOW 解析验证
    const tmpDir = mkdtempSync(path.join(os.tmpdir(), "ygg-ctx-win-"));
    try {
      assert.equal(resolveEffectiveWindowCapFromEnv(tmpDir, {}), undefined);
      assert.equal(
        resolveEffectiveWindowCapFromEnv(tmpDir, { YGG_CONTEXT_WINDOW: "180000" }),
        180_000,
      );
      writeFileSync(path.join(tmpDir, ".env.local"), "YGG_CONTEXT_WINDOW=220000\n", "utf8");
      assert.equal(
        resolveEffectiveWindowCapFromEnv(tmpDir, { YGG_CONTEXT_WINDOW: "180000" }),
        220_000,
        ".env.local 优先于 process.env",
      );
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
      resetClientsForTest();
    }
  });
});

// ───────────────────────────── (a) brief 消费义务：C1 / C2 / C5 ─────────────────────────────

/** `scenes.json` 的 `subagent_brief` 模板填充结果（**逐字**取样子集，两条锚线在其中）。 */
function coreBriefFromScenes(id = "r1", stage = "plan"): string {
  return [
    `Requirement ${id} (rev 1, stage=${stage}): 落地契约`,
    "Description: 落地 brief 消费义务",
    "Mandatory references: read docs/CONTEXT.md (domain invariants) and docs/DESIGN.md (architecture & contracts) before proposing or editing.",
    "Worktree: /tmp/repo (branch=feature/r1, base=abc123)",
    "Artifact: artifacts/r1-plan.md",
    `Submit: .ygg/bin/ygg-agent --mod .ygg/mods/pi-agent-seed --resume --event proposal.submit --payload '"${id}|1|${stage}|<summary>|artifacts/${id}-plan.md||exec=<agent_id>"'`,
  ].join("\n");
}

function spawnAction(overrides: Partial<YggSubagentAction> = {}): YggSubagentAction {
  return {
    action: "subagent.spawn",
    reqId: "r1",
    rev: 1,
    stage: "plan",
    lane: 0,
    artifactPath: "artifacts/r1-plan.md",
    brief: coreBriefFromScenes(),
    ...overrides,
  };
}

describe("checkSubagentActionConsumption（C1 brief 消费义务）", () => {
  it("正例：scenes.json 的真实 brief + 续跑承接段 → 零违规", () => {
    const prompt = ensureResumeHandoverSection(coreBriefFromScenes(), {
      basis: "subagentAction.brief",
      priorExecutor: "",
      outstanding: [],
      artifacts: [],
    });
    assert.deepEqual(checkSubagentActionConsumption(spawnAction(), prompt), []);
    // 锚线确实在它该在的位置（行首，独立成行）
    assert.equal(
      prompt.split(/\r?\n/).some((line) => line.startsWith(MANDATORY_REFERENCES_ANCHOR)),
      true,
    );
    assert.match(prompt, /^Submit:\s*\S/m);
  });

  it("反例：删掉 Mandatory references 行 → 恰好一条 brief-missing-mandatory-references", () => {
    const prompt = ensureResumeHandoverSection(
      coreBriefFromScenes()
        .split("\n")
        .filter((line) => !line.startsWith(MANDATORY_REFERENCES_ANCHOR))
        .join("\n"),
      { basis: "subagentAction.brief", priorExecutor: "", outstanding: [], artifacts: [] },
    );
    const violations = checkSubagentActionConsumption(spawnAction(), prompt);
    assert.equal(violations.length, 1);
    assert.equal(violations[0].code, "brief-missing-mandatory-references");
    assert.match(violations[0].message, /Mandatory references:/);
  });

  it("反例：删掉 Submit: 行 → 恰好一条 brief-missing-submit", () => {
    const prompt = ensureResumeHandoverSection(
      coreBriefFromScenes()
        .split("\n")
        .filter((line) => !line.startsWith("Submit:"))
        .join("\n"),
      { basis: "subagentAction.brief", priorExecutor: "", outstanding: [], artifacts: [] },
    );
    const violations = checkSubagentActionConsumption(spawnAction(), prompt);
    assert.equal(violations.length, 1);
    assert.equal(violations[0].code, "brief-missing-submit");
  });

  it("反例：Submit: 后面空着（只有前缀）也算缺失", () => {
    const prompt = ensureResumeHandoverSection(
      `${coreBriefFromScenes()}\nSubmit:   `,
      { basis: "subagentAction.brief", priorExecutor: "", outstanding: [], artifacts: [] },
    );
    // 真锚线还在，所以先删掉它再补一个空 Submit
    const replaced = prompt
      .split("\n")
      .filter((line) => !/^Submit:\s*\S/.test(line))
      .concat(["Submit:   "])
      .join("\n");
    const codes = checkSubagentActionConsumption(spawnAction(), replaced).map((v) => v.code);
    assert.deepEqual(codes, ["brief-missing-submit"]);
  });

  it("锚线只认行首：需求正文里出现 `… Mandatory references: …` 不算数（防自指断言空转）", () => {
    // 需求 desc 是「前面有别的字，再出现 Mandatory references:」——任意 includes 会误判通过
    const smuggled = [
      "Requirement r1 (rev 1, stage=plan): 落地契约",
      "  Description: 正文里我故意写一句 xx Mandatory references: read docs/CONTEXT.md",
      `Submit: .ygg/bin/ygg-agent --event proposal.submit --payload '"r1|1|plan|<summary>|artifacts/r1-plan.md||exec=<agent_id>"'`,
    ].join("\n");
    const prompt = ensureResumeHandoverSection(smuggled, {
      basis: "subagentAction.brief",
      priorExecutor: "",
      outstanding: [],
      artifacts: [],
    });
    assert.equal(prompt.includes(MANDATORY_REFERENCES_ANCHOR), true, "字样确实在正文里");
    const codes = checkSubagentActionConsumption(spawnAction(), prompt).map((v) => v.code);
    assert.deepEqual(codes, ["brief-missing-mandatory-references"]);
  });

  it("反例：没有 subagentAction（Core 已产出作业却自拼 brief）→ no-core-action", () => {
    const violations = checkSubagentActionConsumption(undefined, "随便一份 brief");
    assert.equal(violations.length, 1);
    assert.equal(violations[0].code, "no-core-action");
    // 旧 Mod 兼容豁免：只查锚线，不再因为「没有 action」判死
    const compat: AdapterContractPolicy = { allowAdapterLocalBrief: true };
    const compatCodes = checkSubagentActionConsumption(undefined, "随便一份 brief", compat).map(
      (v) => v.code,
    );
    assert.deepEqual(compatCodes, [
      "brief-missing-mandatory-references",
      "brief-missing-submit",
      "brief-missing-resume-handover",
    ]);
  });

  it("C5：每份 brief 都必须带 `## 续跑承接（Resume Handover）`，缺了就是违规", () => {
    const codes = checkSubagentActionConsumption(spawnAction(), coreBriefFromScenes()).map(
      (v) => v.code,
    );
    assert.deepEqual(codes, ["brief-missing-resume-handover"]);
  });

  it("ensureResumeHandoverSection 幂等，并写全 basis / 前任执行者 / 未落地 / 硬约束 / 已归档制品", () => {
    const once = ensureResumeHandoverSection("# Brief", {
      basis: "subagentAction.deltaPrompt",
      priorExecutor: "agent-plan-7",
      outstanding: ["门禁 cargo test --workspace 未全绿"],
      artifacts: ["artifacts/r1-plan.md"],
    });
    assert.equal(once.includes(RESUME_HANDOVER_HEADING), true);
    assert.match(once, /- basis: subagentAction\.deltaPrompt/);
    assert.match(once, /- prior executor: agent-plan-7/);
    assert.match(once, /门禁 cargo test --workspace 未全绿/);
    assert.match(once, /不操作 git 历史/);
    assert.match(once, /artifacts\/r1-plan\.md/);
    // 幂等：再来一次原样返回（不会套两层）
    assert.equal(
      ensureResumeHandoverSection(once, {
        basis: "x",
        priorExecutor: "y",
        outstanding: [],
        artifacts: [],
      }),
      once,
    );
    // 没有前任时如实写「本次为新起」，不编一个 id
    assert.match(
      ensureResumeHandoverSection("# B", {
        basis: "adapter-local",
        priorExecutor: "",
        outstanding: [],
        artifacts: [],
      }),
      /- prior executor: \(none — 本次为新起\)/,
    );
  });

  it("C2：deltaPrompt 口径 fail-closed —— 当前模板不可判定 → delta-unassessable；标 assessable 后放行", () => {
    assert.equal(DELTA_ASSESSABLE.assessable, false, "口径必须先钉死为 false（缺锚线）");
    const delta = "Resume r1 (rev 1) for stage `plan` in worktree `/tmp/repo`; write `artifacts/r1-plan.md` and submit.";
    const action = spawnAction({ action: "subagent.resume", deltaPrompt: delta });
    const codes = checkSubagentActionConsumption(action, delta).map((v) => v.code);
    assert.equal(codes.includes("delta-unassessable"), true);
    // 把同一段文本标成可判定后，只剩真实的锚线缺失（这里全缺）
    const assessable = checkSubagentActionConsumption(action, delta, { deltaAssessable: true }).map(
      (v) => v.code,
    );
    assert.equal(assessable.includes("delta-unassessable"), false);
    assert.deepEqual(assessable, [
      "brief-missing-mandatory-references",
      "brief-missing-submit",
      "brief-missing-resume-handover",
    ]);
  });

  it("selectCoreSubagentPrompt：delta 不可判定时**退回 brief** 并说明为什么退（不是静默丢锚线）", () => {
    const action = spawnAction({
      action: "subagent.resume",
      deltaPrompt: "Resume r1 (rev 2) for stage `plan` ...",
      brief: coreBriefFromScenes(),
    });
    const job = jobRow({ subagentAction: action });
    const selected = selectCoreSubagentPrompt(job, true, true);
    assert.equal(selected.basis, "subagentAction.brief");
    assert.match(selected.prompt, /Mandatory references:/);
    assert.match(selected.fallbackNote, /deltaPrompt 缺少可判定锚线，已退回 brief/);
    // 口径放开后才真的用 delta
    const loosened = selectCoreSubagentPrompt(job, true, true, { deltaAssessable: true });
    assert.equal(loosened.basis, "subagentAction.deltaPrompt");
    assert.equal(loosened.fallbackNote, "");
    // 没有 action → 本地拼装（basis 明确写 adapter-local）
    assert.equal(selectCoreSubagentPrompt(jobRow(), false, true).basis, "adapter-local");
  });

  it("本地拼装路径也满足两条锚线（旧 Mod 兼容路径不会自己违反契约）", () => {
    const local = buildSubagentBrief(jobRow(), null, "artifacts/r1-plan.md", "/tmp/repo");
    assert.equal(
      local.split(/\r?\n/).some((line) => line.startsWith(MANDATORY_REFERENCES_ANCHOR)),
      true,
    );
    assert.match(local, /^Submit:\s*\S/m);
    assert.match(buildSubmitLine(jobRow(), "artifacts/r1-plan.md"), /proposal\.submit/);
    // 补尾注的工具也不会破坏锚线
    const augmented = ensureBriefVerificationAndFooter("# Core", "artifacts/r1-plan.md", ["cargo test"], {
      job: jobRow(),
    });
    assert.match(augmented, /^Mandatory references:\s*\S/m);
    assert.match(augmented, /^Submit:\s*\S/m);
  });
});

describe("orchestrateActiveJobs：brief 落盘义务（C3，无 Agent 工具的 fallback）", () => {
  it("briefArtifactPathFor 是 artifacts/<rN>-<stage>-brief.md", () => {
    assert.equal(briefArtifactPathFor(jobRow({ id: "r1", stage: "plan" })), "artifacts/r1-plan-brief.md");
    assert.equal(
      briefArtifactPathFor(jobRow({ id: "r7", stage: "development" })),
      "artifacts/r7-development-brief.md",
    );
  });

  it("没有人能读会话 id 的环境：brief 必须落盘，且与 executeSubagent 收到的 prompt 逐字节相同", async () => {
    const job = jobRow({ id: "r1", stage: "plan", subagentAction: spawnAction() });
    const daemon = new MockDaemon([job]);
    const seen: SubagentRequest[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        // 模拟「没有 Agent 工具」的 harness：拿不到 agentId
        return { text: "# 方案完成", ok: true };
      },
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(seen.length, 1);
    assert.equal(seen[0].resume, undefined, "没有可续的 id 时不猜");
    const briefs = daemon.vfs.filter((entry) => entry.path === "artifacts/r1-plan-brief.md");
    assert.equal(briefs.length, 1, "无 Agent 工具时必须落一份 brief 承接上下文");
    assert.equal(briefs[0].content, seen[0].prompt, "落盘内容必须与真正派发的 prompt 逐字节相同");
    // 落盘发生在派活之前：VFS 里 brief 的写入序早于制品
    assert.equal(daemon.vfs[0].path, "artifacts/r1-plan-brief.md");
    // 回执里有 [BRIEF] 行 + 一行 byte/sha
    assert.equal(
      result.narrative.some((line) => line.includes("[BRIEF] artifacts/r1-plan-brief.md")),
      true,
    );
    assert.equal(
      result.narrative.some((line) => line.includes("(basis=subagentAction.brief)")),
      true,
      "回执要说清这一次读的是 Core 的哪一段",
    );
    // 落盘的那一份自己也要满足契约（含续跑承接段）
    assert.equal(briefs[0].content.includes(RESUME_HANDOVER_HEADING), true);
    assert.equal(result.degraded, undefined, "正常路径不该被标记降级");
  });

  it("Core brief 缺锚线时，adapter 先补齐再派活（补齐路径生效，闸门看到的是补齐后的字节）", async () => {
    const broken = coreBriefFromScenes()
      .split("\n")
      .filter((line) => !line.startsWith(MANDATORY_REFERENCES_ANCHOR))
      .join("\n");
    const job = jobRow({ id: "r1", stage: "plan", subagentAction: spawnAction({ brief: broken }) });
    const daemon = new MockDaemon([job]);
    const seen: SubagentRequest[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        return { text: "# 方案完成", ok: true };
      },
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(seen.length, 1, "缺的那一行可以被补齐，所以能派活（不是整次失败）");
    assert.match(seen[0].prompt, /^Mandatory references:\s*\S/m, "补齐后的 prompt 带锚线");
    // 落盘的 brief 就是补齐后的那一份（审计面看到的就是子代理看到的）
    assert.equal(
      daemon.vfs.find((entry) => entry.path === "artifacts/r1-plan-brief.md")?.content,
      seen[0].prompt,
    );
    assert.equal(result.degraded, undefined);
  });

  it("契约闸门是**派活前**的最后一道：违规的 prompt 立刻被拒（绕不过去）", () => {
    // 诚实说明：adapter 的补齐逻辑让「正常路径」总能产出合规 brief，所以
    // 「编排层因违规而不 spawn」这条分支在今天的输入下不可达——它防的是
    // 「有人删掉补齐逻辑」或「brief 由别处生成不带锚线」这两种回归。
    // 因此这里直接咬住闸门本身（纯函数），并断言它的返回值就是「不许派发」的信号。
    const bad = "Requirement r1 (rev 1, stage=plan): 什么锚线都没有";
    const violations = checkSubagentActionConsumption(spawnAction(), bad);
    assert.deepEqual(
      violations.map((v) => v.code),
      [
        "brief-missing-mandatory-references",
        "brief-missing-submit",
        "brief-missing-resume-handover",
      ],
    );
    // 一条违规都不许漏：只要非空，编排层就把它当作「不 spawn」
    assert.notEqual(violations.length, 0);
  });

  it("delta 不可判定 → 退回 brief 再派发，回执写 [ADAPTER NOTE] 说明原因", async () => {
    const action = spawnAction({
      action: "subagent.resume",
      resumeAgentId: "agent-plan-9",
      deltaPrompt: "Resume r1 (rev 2) for stage `plan` in worktree `/tmp/repo`; write `artifacts/r1-plan.md` and submit.",
    });
    const job = jobRow({
      id: "r1",
      stage: "plan",
      executor: "agent-plan-9",
      subagentAction: action,
    });
    const daemon = new MockDaemon([job]);
    const seen: SubagentRequest[] = [];
    const result = await orchestrateActiveJobs(
      daemon,
      async (request) => {
        seen.push(request);
        return { text: "# 方案 v2", ok: true, agentId: "agent-plan-9" };
      },
      { cwd: "/tmp/repo", repoDir: "/tmp/repo", readArtifact: () => null },
    );
    assert.equal(seen.length, 1);
    assert.match(seen[0].prompt, /Mandatory references:/, "退回到 brief 才带锚线");
    assert.equal(
      result.narrative.some((line) => line.includes("deltaPrompt 缺少可判定锚线，已退回 brief")),
      true,
    );
  });

  it("preferCoreBrief=false 是**显式降级**：回执有 [ADAPTER NOTE]、degraded 非空，但仍要满足锚线", async () => {
    const job = jobRow({ id: "r1", stage: "plan", subagentAction: spawnAction() });
    const daemon = new MockDaemon([job]);
    const result = await orchestrateActiveJobs(
      daemon,
      async () => ({ text: "# 方案", ok: true }),
      {
        cwd: "/tmp/repo",
        repoDir: "/tmp/repo",
        readArtifact: () => null,
        preferCoreBrief: false,
      },
    );
    assert.notEqual(result.degraded, undefined);
    assert.match(result.degraded ?? "", /preferCoreBrief=false/);
    assert.equal(
      result.narrative.some((line) => line.includes("preferCoreBrief=false")),
      true,
      "降级必须在回执里说出来，不能静默",
    );
    // 本地拼装路径也带锚线，所以这里能起来
    const seen = daemon.vfs.find((entry) => entry.path === "artifacts/r1-plan-brief.md");
    assert.match(seen?.content ?? "", /^Submit:\s*\S/m);
  });
});

// ───────────────────────────── (b②-1) 归档结论三态 ─────────────────────────────

describe("analyzeArchiveVerdict（归档结论必须三态可区分，不得「没查到就填 0」）", () => {
  const HOME = "/Users/someone";

  it("① 查过且查空：remaining=0 且 dest 落在本局声明的库根下 → swept", () => {
    const verdict = analyzeArchiveVerdict([
      `[TOOL] harness.archive_sessions moved=1;remaining=0;dest=${HOME}/.pi/agent/sessions-trash/demo`,
    ], { destRoot: `${HOME}/.pi/agent/sessions-trash` });
    assert.deepEqual(verdict, {
      kind: "swept",
      moved: 1,
      dest: `${HOME}/.pi/agent/sessions-trash/demo`,
    });
    assert.equal(archiveSessionsSegment(verdict), null, "swept 时让 Mod 的机证接管，不写 sessions= 段");
    assert.match(archiveVerdictNarrative(verdict), /^\[SESSIONS SWEPT\] moved=1 remaining=0 dest=/);
  });

  it("② 没有库可查：dest=generic:no-session-store:<batch> → no-store（不是失败，也不是证据）", () => {
    const verdict = analyzeArchiveVerdict([
      "[TOOL] harness.archive_sessions moved=0;remaining=0;dest=generic:no-session-store:demo-run",
    ]);
    assert.deepEqual(verdict, { kind: "no-store", batch: "demo-run" });
    assert.equal(archiveSessionsSegment(verdict), "sessions=0/0/unsettled:no-store");
    assert.match(archiveVerdictNarrative(verdict), /没有会话库可查/);
  });

  it("③ 查不了：外域 dest（DSH 局里 dest=~/.pi/...）→ unavailable:foreign-store，绝不判 swept", () => {
    // 这正是 dsh-zh-docs 那次的形态：ok=true、remaining=0，但归档的是 Pi 的库
    const verdict = analyzeArchiveVerdict([
      `[TOOL] harness.archive_sessions moved=1;remaining=0;dest=${HOME}/.pi/agent/sessions-trash/default`,
    ]);
    assert.equal(verdict.kind, "unavailable");
    assert.match((verdict as { reason: string }).reason, /foreign-store/);
    const segment = archiveSessionsSegment(verdict);
    assert.equal(segment, "sessions=0/0/unsettled:unavailable");
    assert.equal(
      segment?.includes(`${HOME}/.pi`),
      false,
      "台账里不许再出现一个冒充真实目录的 Pi 路径",
    );
  });

  it("③ 查不了：工具明确失败 / builtin_usage_error → unavailable（不是 settled）", () => {
    const failed = analyzeArchiveVerdict(["[TOOL FAILED] harness.archive_sessions: builtin_usage_error: HOME 未设置"]);
    assert.equal(failed.kind, "unavailable");
    assert.match(archiveVerdictNarrative(failed), /builtin_usage_error/);
  });

  it("④ 没查成：remaining>0 / 空叙事 / 三元组不全 → unsettled（宁可说没查，不谎报查空了）", () => {
    const remaining = analyzeArchiveVerdict([
      `[TOOL] harness.archive_sessions moved=0;remaining=2;dest=${HOME}/.pi/agent/sessions-trash/x`,
    ], { destRoot: `${HOME}/.pi/agent/sessions-trash` });
    assert.equal(remaining.kind, "unsettled");
    assert.match((remaining as { reason: string }).reason, /remaining=2/);

    const empty = analyzeArchiveVerdict([]);
    assert.equal(empty.kind, "unsettled");
    assert.match((empty as { reason: string }).reason, /没有任何叙事行/);

    const partial = analyzeArchiveVerdict(["moved=0"]);
    assert.equal(partial.kind, "unsettled");
  });

  it("dest 不在声明的库根下（哪怕不是 Pi 家）也算 foreign-store", () => {
    const verdict = analyzeArchiveVerdict(
      [`[TOOL] harness.archive_sessions moved=0;remaining=0;dest=/tmp/elsewhere`],
      { destRoot: `${HOME}/.pi/agent/sessions-trash` },
    );
    assert.equal(verdict.kind, "unavailable");
    assert.match((verdict as { reason: string }).reason, /不在本局声明的会话库根/);
  });

  it("回归：旧实现「工具跑过就认为已 settle」会把 DSH 那次的假象当成成功", () => {
    // 旧判据：narrative 含 `[TOOL SETTLED] harness.archive_sessions` → 不写 sessions= 段。
    // 新判据：只看三元组，`dest` 未被本局 adapter 认领就必须是 unavailable。
    const narrative = [
      `[TOOL] harness.archive_sessions moved=1;remaining=0;dest=${HOME}/.pi/agent/sessions-trash/default`,
      "[TOOL SETTLED] harness.archive_sessions",
    ];
    const verdict = analyzeArchiveVerdict(narrative);
    assert.equal(verdict.kind, "unavailable");
    assert.equal(archiveSessionsSegment(verdict), "sessions=0/0/unsettled:unavailable");
  });
});

describe("buildHandoffMarkdown：(c) 下一步必须补 continuation 提示", () => {
  it("交接单点名「先读 brief 与制品再续」，并指到续跑承接段", () => {
    const markdown = buildHandoffMarkdown({
      runId: "demo",
      summary: summaryWith([jobRow({ id: "r1", stage: "plan" })]),
    });
    assert.match(markdown, /continuation/);
    assert.match(markdown, /artifacts\/<rN>-<stage>-brief\.md/);
    assert.match(markdown, /Resume Handover/);
  });
});

// 让 tsc/编辑器知道这些类型确实被用到（运行时无副作用）。
export type _Types = [
  RunSummary,
  StepReceipt,
  SummaryJob,
  YggActiveJob,
  YggSubagentAction,
  YggHarnessAction,
  YggReceipt,
  SubagentRequest,
  OrchestratorDaemon,
  HandoffWrite,
];

// ─────────────────── ⑤ dispatch 不触发压缩（DESIGN §3.4 回归钉子）───────────────────

describe("index.ts: dispatch 只报告上下文水位，绝不触发压缩", () => {
  /** 水位拉满（>`hard_brake` 的 0.75 且远超软水位）的 ctx：旧实现会在这里真的压缩。 */
  function pressureCtx() {
    const compactCalls: unknown[] = [];
    const ctx = {
      cwd: "/tmp/repo",
      ui: { setStatus() {}, notify() {} },
      getContextUsage: () => ({ tokens: 195_000, contextWindow: 200_000 }),
      compact: (options: unknown) => {
        compactCalls.push(options);
      },
    } as never;
    return { ctx, compactCalls };
  }

  function dispatchClient() {
    const dispatched: [string, string | number | undefined][] = [];
    const client = {
      runId: "run-dispatch",
      async dispatch(event: string, payload?: string | number): Promise<StepReceipt> {
        dispatched.push([event, payload]);
        return {
          ok: true,
          // 回执必须是「整段可见」的证据：两行都必须出现在返回文本里。
          narrative: ["[MAP] coordinate -> planning", "[CHOICE] 1. dispatch  2. pause"],
          pendingChoice: null,
          tools: [],
          raw: {},
        };
      },
      async summary(): Promise<RunSummary> {
        // 稳定节点（无作业、无门、无冲突）：旧实现会「稳定 + 水位到了 → 压缩」。
        return summaryWith([]);
      },
      async vfsList() {
        return { ok: true, artifacts: [] };
      },
    };
    return { client, dispatched };
  }

  it("水位 >= 硬刹车且落在稳定节点时，dispatch 仍不调用 ctx.compact，并整段回贴 narrative", async () => {
    const { dispatchSessionEvent } = await loadIndexExports();
    const { ctx, compactCalls } = pressureCtx();
    const { client, dispatched } = dispatchClient();
    // `pi` 只有 `sendMessage` 会被压缩路径用到；它若被调用说明压缩发生了。
    const sent: unknown[] = [];
    const pi = {
      async sendMessage(message: unknown) {
        sent.push(message);
      },
    } as never;

    const result = await dispatchSessionEvent(
      pi,
      ctx,
      client as never,
      "run-dispatch",
      "map.show",
      "",
    );

    assert.deepEqual(dispatched, [["map.show", ""]], "dispatch 只派发一次、payload 原样透传");
    assert.equal(compactCalls.length, 0, "dispatch 分支绝不能触发 ctx.compact（DESIGN §3.4）");
    assert.equal(sent.length, 0, "不压缩就不会有自动续跑消息");
    const text = result.content[0]!.text;
    assert.match(text, /\[MAP\] coordinate -> planning/, "回执第一行必须到达调用方");
    assert.match(text, /\[CHOICE\] 1\. dispatch {2}2\. pause/, "回执必须整段可见，不截断");
    assert.match(text, /水位 hard_brake/, "仍然报告水位");
    assert.match(text, /dispatch` 只报告水位、不触发压缩/, "必须点名「需要时显式调 handoff_compact」");
    assert.match(text, /handoff_compact/, "报告行必须给出显式入口");
  });

  it("dispatch 的 reportOnly 报告不声称写入过 VFS 交接单", async () => {
    const { dispatchSessionEvent } = await loadIndexExports();
    const { ctx } = pressureCtx();
    const { client } = dispatchClient();
    const pi = { async sendMessage() {} } as never;

    const result = await dispatchSessionEvent(pi, ctx, client as never, "run-dispatch", "map.show", "");
    assert.doesNotMatch(
      result.content[0]!.text,
      /已写入 VFS/,
      "reportOnly 不写交接单，所以不得出现「已写入 VFS」",
    );
  });
});


// ───────────────────────────── ⑥ 看门狗（r1 自动收波） ─────────────────────────────

function wdJob(partial: Partial<SummaryJob> & { id: string }): SummaryJob {
  return {
    rev: 1,
    stage: "plan",
    status: "planning",
    phase: "",
    lane: 0,
    branch: "",
    base: "",
    title: "t",
    description: "d",
    ...partial,
  };
}

/** resident 摘要（r9 三字段 + r1 play 字段就位的形状）。 */
function wdSummary(
  jobs: SummaryJob[],
  overrides: Partial<RunSummary> = {},
): RunSummary {
  return summaryWith(jobs, [], {
    lifecycle: "resident",
    batch: 1,
    batchDrained: false,
    play: "guide",
    ...overrides,
  });
}

/** 手动时钟 + 记录器顶替全部 IO 的看门狗（与手发共享的「会话闸」用可变旗标模拟）。 */
function makeWatchdog(
  summary: RunSummary,
  overrides: {
    idle?: boolean;
    pressureLevel?: "normal" | "soft_brake" | "hard_brake" | null;
    wakeEnabled?: boolean;
    intervalMs?: number;
  } = {},
): {
  wd: AutoWatchdog;
  sent: { content: string; triggerTurn?: boolean }[];
  summaryCalls: () => number;
  setIdle: (idle: boolean) => void;
  setSummary: (summary: RunSummary) => void;
  logs: string[];
} {
  let currentSummary = summary;
  let summaryCalls = 0;
  let idle = overrides.idle ?? true;
  const sent: { content: string; triggerTurn?: boolean }[] = [];
  const logs: string[] = [];
  const wd = new AutoWatchdog({
    client: {
      async summary(): Promise<RunSummary> {
        summaryCalls += 1;
        return currentSummary;
      },
    } as never,
    sender: {
      sendMessage(message: { content: string }, options?: { triggerTurn?: boolean }): void {
        sent.push({ content: message.content, ...(options ?? {}) });
      },
    },
    runId: "run-wd",
    isIdle: () => idle,
    ...(overrides.pressureLevel !== undefined
      ? {
          pressure: () =>
            overrides.pressureLevel === null
              ? null
              : { level: overrides.pressureLevel! },
        }
      : {}),
    wakeEnabled: overrides.wakeEnabled ?? false,
    ...(overrides.intervalMs !== undefined ? { intervalMs: overrides.intervalMs } : {}),
    timer: {
      set: () => ({ native: null }),
      clear: () => {},
    },
    logger: (line) => logs.push(line),
  });
  return {
    wd,
    sent,
    summaryCalls: () => summaryCalls,
    setIdle: (value: boolean) => {
      idle = value;
    },
    setSummary: (value: RunSummary) => {
      currentSummary = value;
    },
    logs,
  };
}

describe("watchdog: 可收波谓词（纯核，r1 方案 §4.1）", () => {
  it("plan / development 作业收波（ACTIVE_STAGES 谓词与手发同集）", () => {
    const state = initialAutoPilotState();
    const plan = nextAutoAction(wdSummary([wdJob({ id: "r1", stage: "plan" })]), state);
    assert.equal(plan.kind, "wave", "plan 作业可收");
    assert.deepEqual(plan.jobs.map((job) => job.stage), ["plan"]);
    const dev = nextAutoAction(
      wdSummary([wdJob({ id: "r2", stage: "development", status: "developing" })]),
      state,
    );
    assert.equal(dev.kind, "wave", "development 作业可收");
  });

  it("lifecycle 缺失 / 非 resident（旧 mod）保守不收；摘要不可用不猜", () => {
    const state = initialAutoPilotState();
    const legacy = wdSummary([wdJob({ id: "r1" })], { lifecycle: undefined });
    assert.equal(nextAutoAction(legacy, state).kind, "none", "lifecycle 缺失 → 保守不收");
    const foreign = wdSummary([wdJob({ id: "r1" })], { lifecycle: "batch" });
    assert.equal(nextAutoAction(foreign, state).kind, "none", "非 resident → 不收");
    assert.equal(
      nextAutoAction(unknownSummary("没有摘要"), state).kind,
      "none",
      "摘要不可用 → none（不猜）",
    );
  });

  it("delivered:* 不收——guide 模式 accept 被票拒即设计停点，不重试", () => {
    const state = initialAutoPilotState();
    const delivered = wdSummary([
      wdJob({ id: "r1", stage: "development", subagentStatus: "delivered:p2(a1)" }),
    ]);
    assert.equal(nextAutoAction(delivered, state).kind, "none", "delivered 不收波");
    assert.equal(isDeliveredJob(wdJob({ id: "r1", subagentStatus: "delivered:p2(a1)" })), true);
    assert.equal(isDeliveredJob(wdJob({ id: "r1", subagentStatus: "subagent pending" })), false);
    const mixed = wdSummary([
      wdJob({ id: "r1", stage: "development", subagentStatus: "delivered:p2(a1)" }),
      wdJob({ id: "r2", stage: "plan" }),
    ]);
    const decision = nextAutoAction(mixed, state);
    assert.deepEqual(
      decision.jobs.map((job) => job.id),
      ["r2"],
      "混合清单只收非 delivered 的那一个",
    );
  });

  it("未决冲突不收（永不越过冲突）；停车作业不收；rev 变化换键自动复位", () => {
    const state = initialAutoPilotState();
    const conflict = wdSummary([wdJob({ id: "r1" })], {
      conflicts: [{ id: "r1", peer: "lane-2", note: "branch overlaps" }],
    });
    assert.equal(nextAutoAction(conflict, state).kind, "none", "有冲突 → none");

    const failed = recordWaveOutcome(state, [
      { key: collectKey(wdJob({ id: "r1", rev: 1, stage: "plan" })), ok: false },
    ]);
    assert.equal((failed.parked[collectKey({ id: "r1", rev: 1, stage: "plan" })] ?? 0), 1);
    const onceFailed = wdSummary([wdJob({ id: "r1", rev: 1, stage: "plan" })]);
    assert.equal(nextAutoAction(onceFailed, failed).kind, "wave", "失败 1 次 < 阈值 2：预算内仍可收");
    const parkedTwice = recordWaveOutcome(failed, [
      { key: collectKey({ id: "r1", rev: 1, stage: "plan" }), ok: false },
    ]);
    assert.equal(nextAutoAction(onceFailed, parkedTwice).kind, "none", "连续 2 次失败 → 停车不收");
    // rev 变化（amend / reject 打回）= 新键 = 自动复位
    const amended = wdSummary([wdJob({ id: "r1", rev: 2, stage: "plan" })]);
    assert.equal(nextAutoAction(amended, parkedTwice).kind, "wave", "rev+1 换键 → 重新可收");
  });

  it("goal 模式 ∧ YGG_AUTO_WAKE=on ∧ 门禁在场 → 唤醒门禁；guide 模式永不唤醒", () => {
    const state = initialAutoPilotState();
    const gated = wdSummary([], {
      play: "goal",
      gates: [{ id: "r1", gate: "plan", rev: 1, title: "T" }],
    });
    assert.equal(nextAutoAction(gated, state).kind, "none", "wake 未 opt-in → none");
    const opted = nextAutoAction(gated, state, { wakeEnabled: true });
    assert.equal(opted.kind, "wake", "goal + opt-in → 唤醒");
    assert.equal(opted.gate?.id, "r1");
    const guide = wdSummary([], {
      play: "guide",
      gates: [{ id: "r1", gate: "plan", rev: 1, title: "T" }],
    });
    assert.equal(
      nextAutoAction(guide, state, { wakeEnabled: true }).kind,
      "none",
      "guide 模式门禁只能人点菜单，永不唤醒",
    );
    assert.match(buildGateWakeMessage("run", { id: "r1", gate: "plan", rev: 1, title: "T" }), /approve_gate|reject_gate/);
  });

  it("collectJobs 保持原序且排除 delivered；波唤醒消息点名 ygg_run_lanes（不越界到 gate）", () => {
    const state = initialAutoPilotState();
    const summary = wdSummary([
      wdJob({ id: "r3", stage: "plan" }),
      wdJob({ id: "r1", stage: "development", subagentStatus: "delivered:p2(a)" }),
      wdJob({ id: "r2", stage: "development" }),
    ]);
    assert.deepEqual(
      collectJobs(summary, state).map((job) => job.id),
      ["r3", "r2"],
      "保持 summary.jobs 原序，delivered 排除",
    );
    const message = buildWaveWakeMessage("run-wd", collectJobs(summary, state));
    assert.match(message, /ygg_run_lanes/, "唤醒指令让主代理调 ygg_run_lanes");
    assert.doesNotMatch(message, /gate\.approve/, "唤醒消息绝不指示批门禁");
    assert.equal(AUTO_WAVE_CUSTOM_TYPE, "ygg-auto-wave");
  });
});

describe("watchdog: 失败预算（per-job 停车 / 全局自关）", () => {
  it("同一键连续 2 次失败停车；成功一次清停车并清零全局计数", () => {
    let state = initialAutoPilotState();
    const key = "r1:1:plan";
    assert.equal(WAVE_FAILURES_PARK, 2, "预算常数钉住：per-job 连续 2 次整波失败即停车");
    for (let i = 1; i < WAVE_FAILURES_PARK; i += 1) {
      state = recordWaveOutcome(state, [{ key, ok: false }]);
      assert.equal(key in state.parked, true, "失败未达阈值也已计数");
    }
    state = recordWaveOutcome(state, [{ key, ok: false }]);
    assert.equal(state.parked[key], WAVE_FAILURES_PARK, "达到阈值：停车表就位");
    assert.equal(state.waveCount, WAVE_FAILURES_PARK);
    state = recordWaveOutcome(state, [{ key, ok: true }]);
    assert.equal(key in state.parked, false, "成功清停车");
    assert.equal(state.consecutiveWaveFailures, 0, "成功清零全局连续全败");
  });

  it(`全局连续 ${WAVE_FAILURES_AUTO_OFF} 波全败 → 自关（enabled=false）`, () => {
    let state = initialAutoPilotState();
    for (let i = 0; i < WAVE_FAILURES_AUTO_OFF; i += 1) {
      state = recordWaveOutcome(state, [{ key: `r1:${i + 1}:plan`, ok: false }]);
    }
    assert.equal(state.enabled, false, "三连全败 → 看门狗自关");
    assert.equal(state.consecutiveWaveFailures, 3);
    state = recordWaveOutcome(state, [{ key: "r9:1:plan", ok: true }]);
    assert.equal(state.enabled, true, "自关后再次结算（外部重启场景）恢复可运行");
  });

  it("空波（无作业落地）不累计全败：竞态空转不算失败", () => {
    const state = recordWaveOutcome(initialAutoPilotState(), []);
    assert.equal(state.consecutiveWaveFailures, 0, "outcomes 为空 → 不算全败");
    assert.equal(state.enabled, true);
  });
});

describe("watchdog: 间隔与唤醒开关解析（观测面 fail-safe）", () => {
  it("clampAutoIntervalMs：非法 → 默认；越界钳位 1s-60s", () => {
    assert.equal(clampAutoIntervalMs(undefined), DEFAULT_AUTO_INTERVAL_MS);
    assert.equal(clampAutoIntervalMs("abc"), DEFAULT_AUTO_INTERVAL_MS);
    assert.equal(clampAutoIntervalMs("-5"), DEFAULT_AUTO_INTERVAL_MS);
    assert.equal(clampAutoIntervalMs(0), DEFAULT_AUTO_INTERVAL_MS);
    assert.equal(clampAutoIntervalMs(500), MIN_AUTO_INTERVAL_MS, "低于 1s 钳到 1s");
    assert.equal(clampAutoIntervalMs("120000"), MAX_AUTO_INTERVAL_MS, "高于 60s 钳到 60s");
    assert.equal(clampAutoIntervalMs(9000), 9000, "合法值原样");
  });

  it("parseAutoWake 只认 on（默认关）", () => {
    assert.equal(parseAutoWake(undefined), false);
    assert.equal(parseAutoWake(""), false);
    assert.equal(parseAutoWake("true"), false);
    assert.equal(parseAutoWake("on"), true);
    assert.equal(parseAutoWake(" ON "), true, "大小写与空白不敏感");
  });
});

describe("watchdog: tick 壳（互斥/水位/心跳/波票）", () => {
  it("会话在忙跳过（与手发共享同一把会话闸 = 单飞互斥）；波票未消费不再发新波", async () => {
    const summary = wdSummary([wdJob({ id: "r1", stage: "plan" })]);
    const harness = makeWatchdog(summary);
    harness.wd.state.enabled = true;

    harness.setIdle(false); // 手发编排/唤醒回合进行中
    const busy = await harness.wd.tick();
    assert.equal(busy.kind, "none", "忙 → 跳过本 tick");
    assert.match(busy.reason, /会话在忙/);
    assert.equal(harness.sent.length, 0, "忙时不发任何消息");

    harness.setIdle(true);
    const t0 = 1_000_000;
    const first = await harness.wd.tick(t0);
    assert.equal(first.kind, "wave");
    assert.equal(harness.sent.length, 1, "空闲且可收 → 发一张波票");
    assert.equal(harness.sent[0]!.triggerTurn, true, "波票是 triggerTurn:true 的自唤醒");
    assert.match(harness.sent[0]!.content, /ygg_run_lanes/);

    harness.setSummary(wdSummary([wdJob({ id: "r2", stage: "plan" })]));
    const inGrace = await harness.wd.tick(t0 + 1_000);
    assert.equal(inGrace.kind, "none", "宽限期内（等自唤醒回合起跑）不再发");
    const second = await harness.wd.tick(t0 + 21_000);
    assert.equal(second.kind, "none", "宽限超时且波票仍未消费 → 不再发新波");
    assert.match(second.reason, /波票未消费|已有未消费/);
    assert.equal(harness.sent.length, 1, "fail-closed：一张票卡住即停发");
  });

  it("硬水位不派任何消息（看门狗不是第三个压缩入口）；软水位照派", async () => {
    const summary = wdSummary([wdJob({ id: "r1", stage: "plan" })]);
    const hard = makeWatchdog(summary, { pressureLevel: "hard_brake" });
    hard.wd.state.enabled = true;
    const hardDecision = await hard.wd.tick();
    assert.equal(hardDecision.kind, "none");
    assert.match(hardDecision.reason, /hard_brake/);
    assert.equal(hard.sent.length, 0, "硬水位零派发");

    const soft = makeWatchdog(summary, { pressureLevel: "soft_brake" });
    soft.wd.state.enabled = true;
    const softDecision = await soft.wd.tick();
    assert.equal(softDecision.kind, "wave", "软水位照跑（落地后由既有刹车面交接）");
    assert.equal(soft.sent.length, 1);
  });

  it("每 tick 恰好一次 summary()——daemon 存活心跳（重启由 daemon.ts 既有单次重试自愈）", async () => {
    const harness = makeWatchdog(wdSummary([]));
    harness.wd.state.enabled = true;
    await harness.wd.tick();
    await harness.wd.tick();
    assert.equal(harness.summaryCalls(), 2, "每 tick 一次 summary（顺带 drain + 存活探测）");
  });

  it("start/stop 走可注入时钟；全局三连全败自关时停表", () => {
    let started = 0;
    let cleared = 0;
    const harness = makeWatchdog(wdSummary([]));
    const wd = new AutoWatchdog({
      client: { async summary() { return wdSummary([]); } } as never,
      sender: { sendMessage() {} },
      runId: "run-clock",
      isIdle: () => true,
      wakeEnabled: false,
      timer: {
        set: () => {
          started += 1;
          return { native: null };
        },
        clear: () => {
          cleared += 1;
        },
      },
      logger: () => {},
    });
    wd.start();
    assert.equal(started, 1, "start 装定时器");
    assert.equal(wd.state.enabled, true);
    wd.stop("测试");
    assert.equal(cleared, 1, "stop 清定时器");
    assert.equal(wd.state.enabled, false);
    assert.equal(harness.logs.length, 0, "独立实例互不干扰");
  });

  it("recordWave 三连全败 → 自关并停表（fail-closed 上报）", async () => {
    const harness = makeWatchdog(wdSummary([wdJob({ id: "r1", stage: "plan" })]));
    harness.wd.start();
    harness.wd.recordWave([{ key: "r1:1:plan", ok: false }]);
    harness.wd.recordWave([{ key: "r1:1:plan", ok: false }]);
    harness.wd.recordWave([{ key: "r1:1:plan", ok: false }]);
    assert.equal(harness.wd.state.enabled, false, "三连全败自关");
    assert.match(harness.logs.join("\n"), /自关/, "自关原因进日志（诚实上报）");
    const after = await harness.wd.tick();
    assert.equal(after.kind, "none", "自关后 tick 不再动作");
  });
});

describe("watchdog: 波次日志与交接单段（全量 narrative，不截断）", () => {
  it("buildWaveLogMarkdown 逐作业逐回执落盘，长行原样保留；停点判定进文档", () => {
    const longLine = "[LANE PROBE] ".padEnd(200, "x");
    const markdown = buildWaveLogMarkdown({
      runId: "run-wd",
      wave: 3,
      reason: "r1:plan",
      result: {
        jobs: [
          {
            id: "r1",
            stage: "plan",
            rev: 1,
            lane: 1,
            branch: "feature/r1-t",
            base: "ac1ee3e",
            worktreePath: "/tmp/wt",
            artifactPath: "artifacts/r1-plan.md",
            brief: "b",
            report: "r",
            archived: { ok: true },
            receipts: [
              { label: "worktree_claim", receipt: receipt(["[LANE CLAIMED]", longLine]) },
              { label: "proposal.submit", receipt: receipt(["[PROPOSAL SUBMITTED] r1|1|plan"]) },
            ],
            expectedGate: "plan",
          },
        ],
        gates: [{ id: "r1", gate: "plan", rev: 1, title: "T" }],
        narrative: [],
        summary: null,
        contextBraked: false,
      } as never,
    });
    assert.match(markdown, /# 自动收波波次日志 0003/);
    assert.match(markdown, /### worktree_claim/);
    assert.ok(markdown.includes(longLine), "长 narrative 行必须逐字保留（永不截断）");
    assert.match(markdown, /## 停点判定/);
    assert.match(markdown, /r1 停在 `plan`/);
  });

  it("waveLogPath 四位序号；交接单第 6 节看门狗段：未启用写明、启用带停车表", () => {
    assert.equal(waveLogPath(1), "artifacts/auto-wave-0001.md");
    assert.equal(waveLogPath(27), "artifacts/auto-wave-0027.md");

    const off = buildHandoffMarkdown({
      runId: "run-wd",
      summary: wdSummary([]),
      now: new Date(0),
    });
    assert.match(off, /## 6\. 看门狗（自动收波）/, "加性第 6 节存在");
    assert.match(off, /未启用/, "缺省如实写未启用");

    const on = buildHandoffMarkdown({
      runId: "run-wd",
      summary: wdSummary([]),
      now: new Date(0),
      watchdog: {
        enabled: true,
        intervalMs: 5_000,
        waveCount: 4,
        consecutiveWaveFailures: 1,
        parked: [{ key: "r1:1:plan", failures: 2 }],
        lastReason: "已发收波唤醒（第 4 波）：r1:plan",
        lastWaveLog: "artifacts/auto-wave-0004.md",
      },
    });
    assert.match(on, /运行中 · 间隔 5000ms · 已触发波次 4/);
    assert.match(on, /artifacts\/auto-wave-0004\.md/);
    assert.match(on, /r1:1:plan/, "停车表逐行可见");
    assert.match(on, /## 7\. 压缩后从这里继续/, "下一步节顺延为第 7 节");
  });
});
