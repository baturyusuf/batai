# Resource & Quota Manager

## 1. Amaç

Agentların provider limitlerini izlemek, limit dolduğunda görevleri güvenli şekilde park etmek ve uygun olduğunda otomatik devam ettirmek.

## 2. Usage Status

- AVAILABLE
- LOW
- DEGRADED
- RATE_LIMITED
- UNKNOWN
- AUTH_REQUIRED
- OFFLINE

## 3. WAITING_RESOURCE Akışı

```text
RUNNING
→ RATE_LIMITED
→ save session
→ save task checkpoint
→ WAITING_RESOURCE
→ scheduler
→ availability check
→ AVAILABLE
→ resume_session
→ continue task
```

## 4. Reset Bilgisi

Provider reset time veriyorsa:
- exact scheduler job oluştur.

Vermiyorsa:
- düşük frekanslı availability check yap.

## 5. Otomatik Continue

Agent resume edildiğinde:
- aynı session mümkünse resume edilir,
- task id ve son checkpoint gönderilir,
- gereksiz tüm conversation replay edilmez.

## 6. Maliyet Politikası

Her agent için:
- preferred cost tier
- max premium calls
- max reasoning
- local preferred
- subscription preferred
- API fallback allowed
tutulur.

## 7. Resource Manager Deterministik Olmalı

Quota polling/scheduling için LLM çağrısı yapılmaz.
