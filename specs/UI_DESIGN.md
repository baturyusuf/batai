# Batai Desktop UI Design

## 1. Ana Layout

Sol:
- Projects
- Files
- Git
- Tasks

Orta:
- Editor / Diff / Preview

Sağ:
- Agent Organization
- Chat tabs
- Resource status

Alt:
- Terminal
- Logs
- Events

## 2. Organization View

```text
Director [GPT / High] ●
├── Architect [Claude / High] ●
├── Backend-01 [Codex / Medium] ●
├── Frontend-01 [Claude / Medium] ◐ WAITING_RESOURCE
├── QA-01 [Local / Low] ●
└── Reviewer [Codex / High] ○ IDLE
```

## 3. Agent Card

Göster:
- role
- provider
- model
- reasoning
- status
- current task
- branch/worktree
- usage state
- parent
- token/cost policy

## 4. GOD Console

Ayrı chat:
- Director ile doğal dil
- pending decisions
- override
- pause/resume
- agent reconfiguration

## 5. Decision Review Panel

Her decision:
- question
- Director recommendation
- alternatives
- impact
- confidence
- GOD action

## 6. Task Board

Kolonlar:
- Pending
- Ready
- Running
- Blocked
- Waiting Resource
- Review
- Completed

## 7. Quota Dashboard

Provider bazında:
- account
- status
- reset time
- active agents
- queued tasks
