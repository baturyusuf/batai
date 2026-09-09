# BATAI MASTER SPECIFICATION

## Executive Summary

Batai, yazılım geliştirme projeleri için farklı sağlayıcıların ve yerel LLM'lerin aynı organizasyon içinde çalışmasını sağlayan bir masaüstü agent runtime + IDE'dir.

Batai'nin temel farklılaştırıcısı, kullanıcıların tek tek agentları manuel yönetmesi değil; **Director agentın Batai'nin control-plane fonksiyonlarını kullanarak organizasyonu kurması ve yönetmesidir.**

Director:
- hangi agentın oluşturulacağına,
- hangi provider/modelin kullanılacağına,
- reasoning seviyesine,
- agent bütçesine,
- local/subscription/API tercihine,
- görev bağımlılıklarına,
- hangi sonuçların kendisine dönmesi gerektiğine
karar verir.

Batai'nin kendisi LLM değildir. Deterministik orchestration sağlar.

## Temel Organizasyon

```text
GOD
 │
 ▼
Director
 │
 ├── Architect
 ├── Lead(s)
 │    └── Worker(s)
 ├── Reviewer
 └── QA
```

Customer Director ile konuşur. Worker'lar müşteriyle konuşmaz.

## Temel Akış

```text
Customer Request
→ Director Requirement Analysis
→ Unknown / Decision Analysis
→ GOD Review Gate
→ Customer Questions if needed
→ Technical Plan
→ Organization Plan
→ Agent Creation
→ Task DAG
→ Event-driven Execution
→ Review / QA
→ Delivery
```

## Batai Control Plane

- AuthorityRouter
- DirectorController
- AgentManager
- AgentFactory
- PolicyEngine
- TaskEngine
- EventEngine
- Scheduler
- ResourceManager
- MeetingManager
- SessionManager
- GitManager
- GitHubManager
- ContextBuilder
- DecisionLedger
- AuditLog

## Ekonomik Optimizasyon

Director her görev için "en güçlü model" değil, "minimum yeterli intelligence" seçer.

Örnek:
- deterministic tool → LLM yok
- basit görev → local/free model
- normal coding → orta model
- kritik coding → güçlü coding model
- mimari/review → frontier/high reasoning

Koordinasyon:
- started/finished,
- dependency,
- timer,
- resume,
- task dispatch
gibi mekanik olaylarda LLM kullanılmaz.

## Event-driven Model

Director task JSON oluşturur.
Batai değişikliği algılar.
Batai agentı uyandırır.
Agent çalışır.
Agent completion event üretir.
Batai:
- diğer taskları otomatik tetikler
veya
- Director'u uyandırır.

Timer/insan koordinasyonuna ihtiyaç minimuma iner.

## Repository Contract

```text
.batai/
├── PROJECT.md
├── organization.json
├── policies.json
├── tasks/
├── decisions/
├── agents/
├── journals/
├── handoffs/
└── reports/
```

Runtime transient state Git dışında SQLite'tadır.

## Worktree

Her coding agent ayrı worktree:
- conflict izolasyonu,
- paralel geliştirme,
- ayrı branch,
- bağımsız test,
- PR tabanlı merge.

## Quota Recovery

Agent limit dolunca:
- session saklanır,
- task WAITING_RESOURCE olur,
- reset zamanı/scheduler kaydedilir,
- provider tekrar uygun olduğunda aynı session resume edilir,
- task kaldığı yerden devam eder.

## MVP Focus

İlk sürümde tam IDE yeniden yazılmayacak.
- Monaco
- xterm.js
- Git CLI
- Tauri
üzerinden yeterli developer yüzeyi sağlanacak.

Asıl ürün:
- Director orchestration
- multi-provider sessions
- task/event engine
- worktree manager
- quota scheduler
- GOD controls
