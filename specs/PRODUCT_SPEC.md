# Batai Product Specification

## 1. Ürün Tanımı

Batai, farklı AI sağlayıcıları ve yerel LLM'ler üzerinde çalışan yazılım geliştirme agentlarını tek bir proje içinde organize eden masaüstü uygulamadır.

Her agent:
- bağımsız bir chat/session'dır,
- belirli bir role sahiptir,
- belirli bir model/provider ile çalışır,
- ayrı bir Git worktree kullanır,
- görev ve ilerlemelerini Batai protokolüyle raporlar.

Director:
- müşteri talebini analiz eder,
- gerekli organizasyonu kurar,
- agentları oluşturur,
- model ve reasoning seviyelerini seçer,
- görevleri dağıtır,
- sonuçları değerlendirir,
- gerekirse yeni görev/agent oluşturur,
- maliyet ve kaynak kullanımını optimize eder.

GOD:
- insan operatördür,
- Director üzerinde en yüksek yetkiye sahiptir,
- proje başlangıcındaki kritik kararları onaylar,
- doğal dil ile herhangi bir anda müdahale edebilir.

## 2. Hedef Kullanım

Örnek müşteri talebi:
> Bir fitness uygulaması geliştir.

Batai:
1. Director chat'ini başlatır.
2. Director requirement analizi yapar.
3. Ürün davranışını etkileyen bilinmezlikleri çıkarır.
4. GOD review yapılır.
5. Gerekirse müşteri soruları oluşturulur.
6. Director teknik planı oluşturur.
7. Director gerekli agent organizasyonunu kurar.
8. Agentlar worktree'lerde çalışır.
9. Batai task/event dosyalarını izler.
10. Agentlar birbirini veya Director'u otomatik tetikler.
11. Review/QA tamamlanır.
12. Çıktı teslim edilir.

## 3. Roller

### Customer
- ürün talebini verir,
- yalnız ürün davranışını etkileyen kararlarla muhatap olur,
- teknik detaylara zorlanmaz.

### GOD
- tüm projelerde en yüksek insan yetkisidir,
- Director kararlarını değiştirebilir,
- global/project/task scope policy verebilir.

### Director
- customer-facing tek AI yöneticidir,
- organizasyonu yönetir,
- teknik kararların çoğunu kendisi verir,
- customer question gate uygular,
- Batai control tools kullanır.

### Lead
- belirli alanı koordine eder,
- doğrudan müşteriyle konuşmaz,
- Director tarafından atanır.

### Worker
- dar kapsamlı implementasyon görevlerini yapar,
- müşteriyle konuşmaz,
- ürün stratejisi üretmez.

### Reviewer / QA
- bağımsız doğrulama yapar,
- developer'ın kendi çıktısını onaylamaz.

## 4. Ürün Farklılaştırıcıları

1. Director-managed heterogeneous AI team
2. Agent başına dinamik provider/model/reasoning seçimi
3. Subscription/local/free/premium model karışımı
4. Event-driven, düşük-token koordinasyon
5. JSON/task dosyası değişikliğiyle agent tetikleme
6. Quota dolunca park etme ve otomatik resume
7. Persistent organizational memory
8. GOD müdahalesi
9. Worktree izolasyonu
10. Runtime state + repository audit trail ayrımı
