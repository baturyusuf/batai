# Task & Event Protocol

## 1. Amaç

Agentlar arası iletişim mümkün olduğunca serbest chat yerine structured task/event sistemi ile yürür.

## 2. Task Dosyaları

Konum:
`.batai/tasks/<task-id>.json`

Bir task:
- objective
- assignee
- dependencies
- acceptance criteria
- inputs
- outputs
- execution policy
- completion action
içerir.

## 3. Event Türleri

- TASK_CREATED
- TASK_ASSIGNED
- TASK_STARTED
- TASK_BLOCKED
- TASK_COMPLETED
- TASK_FAILED
- TASK_CANCELLED
- AGENT_CREATED
- AGENT_READY
- AGENT_RATE_LIMITED
- AGENT_RESUMED
- AGENT_COMPLETED
- DIRECTIVE_UPDATED
- ISSUE_CREATED
- ISSUE_ASSIGNED
- PR_CREATED
- PR_APPROVED
- CI_FAILED
- CI_PASSED
- REVIEW_REQUIRED
- GOD_DECISION_REQUIRED
- CUSTOMER_DECISION_REQUIRED

## 4. File Watcher Akışı

1. Director `.batai/tasks/T-42.json` yazar/değiştirir.
2. Batai watcher değişikliği algılar.
3. JSON schema validation yapılır.
4. runtime.db güncellenir.
5. dependency kontrol edilir.
6. assignee agent READY ise task gönderilir.
7. agent RUNNING olur.

## 5. Completion Chain

Task:
```json
{
  "on_success": {
    "start": ["T-43", "T-44"]
  },
  "on_failure": {
    "notify": "director"
  }
}
```

Bu durumda Director tekrar çağrılmadan T-43 ve T-44 başlayabilir.

## 6. Director Review Gereken Task

```json
{
  "on_success": {
    "notify": "director",
    "reason": "review_required"
  }
}
```

Batai Director session'ını uyandırır ve task summary gönderir.

## 7. Agentlar Arası Handoff

Agent A tamamladığında:
- result artifact oluşturur,
- HANDOFF event oluşturur,
- Batai hedef agentı uyandırır,
- yalnız gerekli artifact/context gönderilir.

## 8. Toplantı

Meeting:
- agenda
- participants
- max rounds
- max answer tokens
- closure owner

Default:
- max participants: 5
- max rounds: 2
- Director closes meeting
