# Yggdrasill · mod 文案契约 · beats-lanes
> **id** copy/lanes · **幕** all · **触发** 改 `beats-lanes.json` 前
> **读不到代价** silent-drift
> **ver** c0aeb4

> **定位**：`beats-lanes.json` 36 键的句式知识权威：泳道账本、池注册、认领释放与 probe。
> **上游**：[copy/README.md](README.md) · **运行时投影**：`.ygg/mods/agent-seed/data/texts/beats-lanes.json`
> **维护**：改文案先改本表再同步 mod；键增删/改域触发索引、prompt-surface 与键数同批更新。

| 键 | 句式模板（逐字） | 变量 | 协议归属 |
|---|---|---|---|
| `conflict_radar` | [CONFLICT RADAR] {{first}} ({{lane}}) and {{second}} ({{peer}}) both modified {{file}} | first, lane, second, peer, file | 叙述 |
| `conflict_radar_note` | overlapping file: {{file}} | file | 叙述 |
| `lane_bad_k` | a lane number accepts only 1..{{max}}, got `{{k}}`. | max, k | 叙述 |
| `lane_branch_hint` | [HINT] {{lane}} carries no branch: run lane.claim first (payload "lane\|req_id\|branch\|base\|clean\|pool", branch = feature/xxxx-semantic) so the worktree claim can bind a real branch; dispatching without one only records the ledger. | lane | 叙述 |
| `lane_branch_invalid` | [LANE BRANCH INVALID] branch "{{branch}}" violates ^feature/[a-z0-9]{4}-<semantic>$ (total <=48; semantic: lowercase alnum/dash, >=3 consecutive letters, not pure digits, not 8-16 pure hex). | branch | 叙述 |
| `lane_claim_bound` | {{id}} already holds {{lane}}; a second lane is not allowed. | id, lane | 叙述 |
| `lane_claim_closed` | {{id}} is {{status}} — terminal requirements cannot claim a lane. | id, status | 叙述 |
| `lane_claim_dirty` | {{lane}} claim refused: `clean` must be "1". The main agent runs `git status --porcelain` in the target worktree first (must be empty except `.lane`); if dirty, pick another lane or handle the changes — the mod cannot run git, it only accepts this assertion. | lane | 叙述 |
| `lane_claim_occupied` | {{lane}} is already owned by {{id}} — pick another lane or wait for its release. | lane, id | 叙述 |
| `lane_claim_usage` | lane.claim "lane\|req_id\|branch\|base\|clean\|pool" (lane 1..{{max}}; pool optional = registered named pool, default pool when omitted; clean must be "1") | max | 叙述 |
| `lane_claimed` | [LANE CLAIMED] {{lane}} ← {{id}} "{{title}}" \| branch {{branch}} \| base {{base}} \| since #{{since}} \| clean=1 (the main agent confirmed a clean tree) | lane, id, title, branch, base, since | 叙述 |
| `lane_field_none` | - | — | 叙述 |
| `lane_ledger_all_free` |   all {{total}} lanes free | total | 叙述 |
| `lane_ledger_all_free_pool` |   all {{pool}} lanes free ({{total}}) | pool, total | 叙述 |
| `lane_ledger_head` | -- lane ledger: {{used}}/{{total}} owned (branch / base / clean are recorded by lane.claim, since = claim serial, clean = the main agent's clean-tree assertion) -- | used, total | 叙述 |
| `lane_ledger_head_pool` | -- lane ledger[{{pool}}]: {{used}}/{{total}} owned (branch / base / clean are recorded by lane.claim, since = claim serial, clean = the main agent's clean-tree assertion) -- | pool, used, total | 叙述 |
| `lane_ledger_none` |   free: {{lanes}} | lanes | 叙述 |
| `lane_ledger_used` |   {{lane}} ← {{id}} ({{stage}}) \| branch {{branch}} \| base {{base}} \| since #{{since}} \| clean {{clean}} | lane, id, stage, branch, base, since, clean | 叙述 |
| `lane_pool_bad_label` | a pool label accepts only ^[a-z][a-z0-9-]{0,23}$ (reserved: seq, pools), got `{{pool}}`. | pool | 叙述 |
| `lane_pool_bad_segment` | lane.pool usage: 'label\|path' — path must be non-empty, without \| ; newline, no leading/trailing spaces. | — | 叙述 |
| `lane_pool_path_missing` | pool `{{pool}}` has no registered path — run lane.pool "{{pool}}\|<local checkout path>" first. | pool, pool | 叙述 |
| `lane_pool_registered` | [LANE POOL] {{pool}} -> {{path}} \| {{state}} ({{count}} named pools) | pool, path, state, count | 叙述 |
| `lane_pool_unknown` | pool `{{pool}}` is not registered — run lane.pool "{{pool}}\|<local checkout path>" first. | pool, pool | 叙述 |
| `lane_probe_bad_lane` | [LANE PROBE REJECTED] lane={{lane}}: out of range 1..{{max}} \| nothing was recorded | lane, max | 叙述 |
| `lane_probe_failed` | [LANE PROBE FAILED] {{lane}}: commits={{commits}} clean={{clean}} | lane, commits, clean | 叙述 |
| `lane_probe_recorded` | [LANE PROBE] {{lane}}: commits={{commits}} clean={{clean}} head={{head}} files={{files}} | lane, commits, clean, head, files | 叙述 |
| `lane_probe_truncated` | [LANE PROBE TRUNCATED] {{call_id}}: the file list was cut at the host output cap - the conflict radar is skipped for this probe | call_id | 叙述 |
| `lane_release_idle` | none (it was already free) | — | 叙述 |
| `lane_release_usage` | lane.release "lane\|pool" (1..{{max}}; pool optional = registered named pool, default pool when omitted) | max | 叙述 |
| `lane_released` | [LANE RELEASED] {{lane}} → free \| previous owner {{id}} | lane, id | 叙述 |
| `probe_rejected` | only an empty payload, "0" or "full" is accepted, got `{{got}}`; usage: {{usage}} | got, usage | 叙述 |
| `worktree_claim_refused` | [WORKTREE CLAIM REFUSED] {{lane}}: {{reason}} \| the lane ledger was not touched | lane, reason | 叙述 |
| `worktree_claimed` | [WORKTREE CLAIMED] {{lane}} <- {{req}} \| branch {{branch}} \| base {{base}} \| clean=1 | lane, req, branch, base | 叙述 |
| `worktree_release_idle` | [WORKTREE RELEASED] {{lane}} -> free (it was already idle) | lane | 叙述 |
| `worktree_release_refused` | [WORKTREE RELEASE REFUSED] {{reason}} \| the lane ledger was not touched | reason | 叙述 |
| `worktree_released` | [WORKTREE RELEASED] {{lane}} -> free | lane | 叙述 |
