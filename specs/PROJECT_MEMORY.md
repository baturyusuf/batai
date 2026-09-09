# Project Memory & Organizational Memory

## 1. Amaç

Agentlar birbirlerinin uzun chat geçmişlerini okumadan ortak proje bilgisini kullanabilmelidir.

## 2. Repository Memory

```text
.batai/
├── PROJECT.md
├── organization.json
├── policies.json
├── decisions/
├── tasks/
├── agents/
│   └── <agent-id>/
│       ├── ROLE.md
│       └── DIRECTIVES.md
├── journals/
├── handoffs/
└── reports/
```

## 3. Agent Journal

Agent yalnız şu eventlerde journal yazar:
- TASK_STARTED
- DECISION_MADE
- INTERFACE_CHANGED
- BLOCKED
- TASK_COMPLETED
- HANDOFF

Journal kısa ve structured olmalıdır.

## 4. Director Directives

Her agent için:
`.batai/agents/<id>/DIRECTIVES.md`

İçerik:
- scope
- must
- must not
- priorities
- temporary constraints

Dosya değişirse Batai DIRECTIVE_UPDATED event üretir.

## 5. ADR

Teknik kararlar:
`.batai/decisions/ADR-XXX.md`

Amaç:
- aynı teknik konunun tekrar tekrar reasoning tüketmesini önlemek.

## 6. Context Builder

Her agent yalnız şu bağlamı alır:
- ROLE
- DIRECTIVES
- current TASK
- ilgili ADR
- ilgili interface/contract
- gerekli source files
- relevant recent events

Tüm müşteri konuşması gönderilmez.
