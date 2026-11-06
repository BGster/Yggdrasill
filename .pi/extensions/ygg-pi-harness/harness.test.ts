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
} from "./daemon.ts";
import { checkToolCallFirewall, checkBatchEntrypoint, checkPathText } from "./firewall.ts";
import {
  artifactPathFor,
  branchNameFor,
  buildLaneProbeArgs,
  buildProposalSubmitPayload,
  buildSubagentBrief,
  buildWorktreeClaimArgs,
  orchestrateActiveJobs,
  pickFreeLane,
  requirementNumber,
  sanitizeSegment,
  slugify,
  worktreePathFor,
  type OrchestratorDaemon,
  type SubagentRequest,
} from "./orchestrator.ts";

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

function summaryWith(jobs: SummaryJob[], gates: { id: string; gate: string; rev: number; title: string }[] = []): RunSummary {
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

// 让 tsc/编辑器知道这些类型确实被用到（运行时无副作用）。
export type _Types = [RunSummary, StepReceipt, SummaryJob, SubagentRequest, OrchestratorDaemon];
