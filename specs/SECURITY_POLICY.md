# Security & Permission Policy

## 1. Least Privilege

Her agent yalnız gerekli:
- file paths
- tools
- repo
- environment
erişimine sahip olur.

## 2. Default Forbidden

Worker:
- main branch direct write
- production deploy
- production DB destructive operation
- secret read
- customer contact
- agent creation
- global policy change

## 3. Approval Required

- production deploy
- destructive migration
- secret rotation
- cloud resource deletion
- permanent agent creation
- global policy modification

Default approver: GOD.

## 4. Sandbox

Kod yazan agent:
- worktree
- process sandbox
- optional container
kullanır.

## 5. Secret Storage

Secrets `.batai` repo içine yazılmaz.
OS keychain / secure store kullanılır.

## 6. Audit

Her:
- task assignment
- agent creation
- model/provider change
- GOD override
- merge
- deployment
audit event olarak kaydedilir.
