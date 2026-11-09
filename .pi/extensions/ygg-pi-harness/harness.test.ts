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
  buildLaneProbeArgs,
  buildProposalSubmitPayload,
  buildResumeDeltaPrompt,
  buildSubagentBrief,
  buildWorktreeClaimArgs,
  COMMIT_NUDGE_TEXT,
  ensureBriefVerificationAndFooter,
  extractAgentId,
  laneProbeNeedsCommit,
  orchestrateActiveJobs,
  parseLaneProbeNarrative,
  parseSubagentOutput,
  pickFreeLane,
  readLaneProbeFromGit,
  requirementNumber,
  resolveResumeId,
  resumeReason,
  sanitizeSegment,
  slugify,
  UNKNOWN_LANE_PROBE,
  worktreePathFor,
  type OrchestratorDaemon,
  type SubagentRequest,
} from "./orchestrator.ts";
import {
  buildCompactInstructions,
  buildHandoffMarkdown,
  buildResumeMessage,
  evaluateContextPressure,
  formatPercent,
  HANDOFF_ARTIFACT_PATH,
  HANDOFF_RESUME_CUSTOM_TYPE,
  isStableCheckpoint,
  stableCheckpointReason,
  writeHandoffToVfs,
  type HandoffWrite,
} from "./compaction.ts";

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

    // ② 制品入 VFS
    assert.equal(daemon.vfs.length, 1);
    assert.equal(daemon.vfs[0].path, "artifacts/r2-development.md");
    assert.match(daemon.vfs[0].content, /cargo test --workspace: ok/);

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
      ["artifacts/r1-plan.md", "artifacts/r2-plan.md"],
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
    assert.equal(daemon.vfs[0].content, "# 落盘版方案");
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
    assert.equal(daemon.vfs.length, 2, "失败的作业也要归档一条（写明失败原因）");
    assert.match(daemon.vfs.find((entry) => entry.path.includes("r1"))?.content ?? "", /崩了/);
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
    assert.match(unknown, /先 `step map\.show` 重新取一次现场/);
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
