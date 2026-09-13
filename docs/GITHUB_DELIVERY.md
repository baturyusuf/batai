# Rust-native GitHub delivery

Batai separates local Git from GitHub collaboration. `WorktreeManager` owns branches, worktrees, status, diffs and commits. `GitHubService` uses the authenticated official `gh` CLI with structured arguments and JSON output for repository identity, issues, pull requests, reviews, checks and merges. Every remote call is bound to the detected `owner/repository`; no shell interpolation or token extraction is used.

## Lifecycle

```text
GitHub issue ↔ Batai task
                    ↓
agent worktree → commit → push → pull request
                                   ↓
                         internal review + CI
                                   ↓
                         SHA-bound merge gate
                                   ↓
                         merged → safe cleanup
```

Issue links are unique by provider, repository, entity type and number, so repeated imports reuse the same task. A delivery checkpoint stores repository identity, base/base SHA, branch, worktree, commits, issue, typed PR/review/CI state, merge approval binding and last sync time. Delivery is a sub-state: a coding task whose policy requires delivery pauses in review after code execution instead of claiming completion at “code written”.

Branches use `batai/<agent>/<task>` and are created from the detected repository default branch unless the task explicitly names a base. A coding agent cannot deliver from the default branch. Commit messages include `Batai-Task` and `Batai-Agent` trailers without inventing a human author; existing Git author configuration is not modified.

Pull-request bodies contain observable summary, validation commands, task, agent and base information. Provider transcripts and private reasoning are excluded. Internal Batai review and GitHub review requests remain separate. `CHANGES_REQUESTED` returns the delivery to changes-ready until the bounded retry budget is exhausted. CI uses `PENDING`, `PASS`, `FAIL`, `CANCELLED` and `UNKNOWN`; unknown is never treated as pass.

CI refresh uses the durable scheduler. It starts conservatively after PR creation, coalesces one pending job per task and backs off while checks are pending or GitHub is unavailable. Audit/events are emitted only for meaningful state transitions.

## Merge safety

The default merge policy is `GOD_APPROVAL`; `MANUAL` keeps merging on GitHub, and `AUTOMATIC_SAFE` requires both an explicit delivery policy and the separately disabled-by-default project governance flag. Batai requires approved review plus CI pass. An approval is bound to the exact PR HEAD SHA. A new commit clears the prior approval and prior CI result. `gh pr merge --match-head-commit` supplies a second server-side stale-head guard. Conflicts or unsatisfied repository policy block delivery rather than forcing a merge.

## Remote recovery

GitHub is not part of SQLite or Git transactions. Before `CREATE_ISSUE`, `PUSH_BRANCH`, `CREATE_PR`, `UPDATE_PR`, `MERGE_PR` or `CLOSE_ISSUE`, Batai records a durable remote operation with a stable idempotency key. Phases are `PREPARED`, `REMOTE_APPLIED`, `LOCAL_APPLIED`, `COMMITTED` and `NEEDS_REVIEW`.

At startup Batai compares unfinished operations with remote truth. If a PR exists for the exact repository/branch, it adopts it instead of creating a duplicate. If GitHub already reports the PR merged, Batai forward-completes the local checkpoint and task. If the side effect cannot be proven, it becomes `NEEDS_REVIEW`; Batai does not replay blindly. External title/body edits are harmless, while head/base/state changes are reconciled as lifecycle changes.

## Authentication and official contract

`gh auth status` is normalized as `NOT_INSTALLED`, `AUTH_REQUIRED`, `AVAILABLE` or `ERROR`. Batai never reads or persists the GitHub token and guides the user through `gh auth login`. The command contract was checked against the official GitHub CLI manual on 2026-09-12, including JSON fields for `repo view`, `issue view`, `pr view`, `pr checks`, and `--match-head-commit` for `pr merge`.

Real mutations are opt-in only. Automated tests use a local bare Git remote and a deterministic fake `gh` transport. The real draft-PR smoke runs only with `BATAI_REAL_GITHUB_E2E=1`, an explicit `BATAI_REAL_GITHUB_E2E_ROOT`, the exact `BATAI_REAL_GITHUB_E2E_REPOSITORY` slug and a safety-marker file containing the documented acknowledgement. It creates an isolated branch/draft PR, closes it, deletes the remote branch and removes the clean worktree. The current project repository is never selected implicitly and normal CI never sets this gate.
