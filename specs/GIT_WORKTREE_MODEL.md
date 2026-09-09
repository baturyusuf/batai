# Git / Worktree / Issue Model

## 1. Temel Kural

Her kod yazan agent ayrı Git worktree ve branch kullanır.

Örnek:
```text
repo/
worktrees/
├── backend-agent-01
├── frontend-agent-02
└── qa-agent-03
```

## 2. Branch Naming

`batai/<agent-id>/<task-id>`

Örnek:
`batai/backend-01/TASK-142`

## 3. Merge

Agent doğrudan main'e push etmez.

Akış:
- code
- tests
- commit
- PR
- reviewer
- QA/CI
- merge

## 4. GitHub Issue

Issue yalnız:
- cross-agent blocker,
- persistent defect,
- architecture conflict,
- user-visible bug,
- deferred technical debt
için açılır.

Küçük lokal problem issue olmaz.

## 5. Issue Routing

Agent issue açabilir.
Director:
- assignee belirler,
- task oluşturur,
- Batai target agentı uyandırır.

## 6. Worktree + Shared State

Git worktree'ler birbirlerinin uncommitted `.batai` değişikliklerini anında göremez.

Bu nedenle:
- runtime state = merkezi SQLite
- repo state = commitlenebilir audit trail

Batai task/event değişikliklerini merkezi proje root üzerinden yönetir.
