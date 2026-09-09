# Batai Repository Instructions for Coding Agents

Read `specs/MASTER_SPEC.md` and `docs/IMPLEMENTATION_STATUS.md` before modifying architecture.

## Rules

- Prefer deterministic orchestration to LLM calls.
- Keep provider-specific code under `src/providers/`.
- Keep core state/task/event logic provider-neutral.
- Never hard-code personal credentials, tokens or subscription data.
- Every coding agent should work in a dedicated Git worktree in production use.
- New task/event protocol fields must remain backward compatible or include a migration.
- Add or update tests for every behavior change.
- Do not expand into market research, legal analysis or unrelated product features.

## Test gate

Run:

```bash
npm test
```

All tests must pass before handoff.
