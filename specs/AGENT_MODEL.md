# Agent Model

## 1. Agent = Session + Role + Tools + Context + Budget

Agent tamamen serbest prompttan yaratılmaz.

Temel agent template'leri:
- SoftwareEngineer
- Architect
- Reviewer
- QAEngineer
- DevOpsEngineer
- DataEngineer
- MLEngineer
- Designer
- Researcher

Director template seçer ve konfigüre eder.

## 2. AgentConfig

Alanlar:
- id
- name
- role_template
- parent_agent_id
- provider
- model
- reasoning_effort
- auth_mode
- workspace/worktree
- allowed_paths
- tools
- constraints
- token budget
- max turns
- lifetime
- status

## 3. Lifecycle

```text
CREATED
→ INITIALIZING
→ READY
→ RUNNING
→ BLOCKED / WAITING_RESOURCE / PAUSED
→ RUNNING
→ COMPLETED
→ TERMINATED
```

Failure:
```text
RUNNING
→ FAILED
→ RETRY_PENDING
→ RUNNING
```

## 4. Worker Kısıtları

Worker:
- müşteriyle konuşmaz,
- ürün kapsamını genişletmez,
- gereksiz research yapmaz,
- kendi scope dışındaki modülleri değiştirmez,
- yeni agent yaratmaz,
- kritik ürün kararı vermez,
- blocked olduğunda parent'a structured event yollar.

## 5. Varsayılan Organizasyon

```text
Director
├── Architect
├── Lead / Worker
├── Reviewer
└── QA
```

Küçük projede:
```text
Director
└── Fullstack Worker
```

Büyük projede:
```text
Director
├── Backend Lead
│   ├── Worker
│   └── Worker
├── Frontend Lead
├── Architect
├── Reviewer
└── QA
```
