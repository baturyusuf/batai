# Recovery Model

Batai does **not** claim a distributed ACID transaction across the filesystem and SQLite. These stores cannot share one atomic commit. Batai instead uses a durable operation journal, idempotent steps, deterministic forward recovery, safe rollback and human review when the observed state is ambiguous.

## Journal boundary

The journal protects mutations that need consistency across declarative project state under `.batai` and runtime state in SQLite. Current operation types cover agent create/update, reporting and organizational relationship changes, policy changes, decision-bound mutations and task/worktree metadata assignment. Ordinary single-table runtime updates do not create journal noise. The record structure is extensible for future GitHub lifecycle operations.

Each record contains a stable operation ID and type, actor and target, organization revision before/after, timestamps, current phase, affected files and database entities, preimage and intended fingerprints, optional bounded file preimages, decision/task correlation, failure details and recovery disposition. Secrets and provider credentials are excluded.

## Phases

The successful path is:

1. `PREPARED`
2. `APPLYING_FILES`
3. `FILES_APPLIED`
4. `APPLYING_DB`
5. `DB_APPLIED`
6. `COMMITTED`

Recovery can use `ROLLBACK_REQUIRED`, `ROLLING_BACK`, `ROLLED_BACK`, `RECOVERY_REQUIRED` or `NEEDS_REVIEW`. A cross-store mutation cannot write its target files or SQLite entities until the `PREPARED` record is durable.

Organization files are serialized first, bounded preimages and SHA-256 fingerprints are recorded, and writes use a same-directory temporary file, flush and replace/backup sequence. Related SQLite entity changes and the database phase transition are committed in one SQLite transaction where possible. Audit/event emission follows the durable state changes.

## Startup reconciliation

Before the runtime accepts normal organization mutations, it lists unfinished operations and reconciles each one:

- `PREPARED` with unchanged state safely aborts and rolls back.
- Intended files present but SQLite missing forward-completes the deterministic database step.
- Intended files and database state present with `DB_APPLIED` completes only the journal commit marker.
- A file or organization revision changed by an external actor is never blindly overwritten; the operation becomes `NEEDS_REVIEW`.
- Interrupted same-directory replacement can recover from the recorded temporary/backup paths.

Forward completion is preferred only when the intended state is deterministic and fingerprints match. Rollback is available only while the preimage and current state make it safe. UI actions are `Retry/complete`, `Accept current state` and `Rollback`; each action revalidates state rather than acting as a force switch.

## Approval restart boundary

Provider approval records use a separate state machine and table. A live Codex JSON-RPC request belongs to one App Server process/session, thread, turn and request ID. If Batai restarts, that live channel no longer exists, so unfinished approvals become `ORPHANED` and the task moves to review. A later GOD click cannot execute the old request.

## Audit and retention

The journal answers how an interrupted operation can recover. Audit answers who attempted what and what outcome a person should see. Normal success is audited as applied; recovery records `RECOVERED`, `ROLLED_BACK` or `RECOVERY_REQUIRED`. Journal rows are queryable with bounded UI exposure. This release does not aggressively compact committed rows; future retention may compact journal mechanics without deleting audit history.

## Tested failures

Fault-injection tests reopen the runtime after failure before files, after files/before SQLite, after SQLite/before the commit marker and during a Windows-style replace. Tests also verify temporary-file cleanup, database/file rollback after a pre-SQLite failure and external fingerprint conflicts that require review without overwriting user changes.
