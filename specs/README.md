# Batai Specification Pack

Batai, farklı LLM sağlayıcıları ve yerel modeller üzerinde çalışan yazılım geliştirme agentlarını tek bir proje organizasyonu içinde yöneten, Director kontrollü, event-driven bir AI developer organization runtime + desktop IDE'dir.

Bu klasör Codex'e geçmeden önce ürünün davranışını, teknik mimarisini, protokollerini ve MVP sınırlarını sabitlemek için hazırlanmıştır.

## Temel ilkeler

1. **Director en üst AI yöneticidir.**
2. **GOD insan operatördür ve en yüksek yetkiye sahiptir.**
3. **Müşteri yalnız ürün davranışını etkileyen kararlarla muhatap olur.**
4. **Agentlar bağımsız chat/session'lardır.**
5. **Her agent ayrı worktree üzerinde çalışır.**
6. **Koordinasyon mümkün olduğunca LLM yerine deterministik Batai runtime tarafından yapılır.**
7. **Director agent başına provider, model, reasoning seviyesi, maliyet politikası ve çalışma kısıtlarını seçebilir.**
8. **Agentlar arası koordinasyon JSON/task/event dosyaları ve Batai event engine üzerinden yürür.**
9. **Token/quota bittiğinde işler park edilir; kaynak yenilendiğinde otomatik resume edilir.**
10. **Repository kalıcı/audit edilebilir proje hafızasıdır; runtime state ayrı SQLite veritabanında tutulur.**

## Dosyalar

- `MASTER_SPEC.md` — tüm sistemin ana spesifikasyonu
- `PRODUCT_SPEC.md` — ürün amacı, kullanıcı rolleri ve davranış
- `ARCHITECTURE.md` — teknik mimari
- `DIRECTOR_SPEC.md` — Director yetkileri ve davranış kuralları
- `GOD_SPEC.md` — GOD müdahale modeli
- `AGENT_MODEL.md` — agent lifecycle, hiyerarşi ve agent factory
- `TASK_EVENT_PROTOCOL.md` — task/event/message protokolü
- `PROVIDER_ADAPTERS.md` — Codex/Claude/ACP/CLI/local/API adapter yapısı
- `RESOURCE_QUOTA_MANAGER.md` — usage/quota yönetimi ve otomatik resume
- `GIT_WORKTREE_MODEL.md` — repo/worktree/issue/PR çalışma modeli
- `PROJECT_MEMORY.md` — kalıcı proje hafızası ve journal yapısı
- `SECURITY_POLICY.md` — yetkiler, sandbox ve approval kapıları
- `UI_DESIGN.md` — masaüstü uygulama ekranları
- `MVP_ROADMAP.md` — geliştirme fazları
- `ACCEPTANCE_CRITERIA.md` — MVP bitmiş sayılma kriterleri
- `CODEX_BOOTSTRAP_PROMPT.md` — ilk Codex görevi
- `schemas/*.json` — makine-okunabilir şemalar
- `examples/*.json` — örnek task/event/agent tanımları
