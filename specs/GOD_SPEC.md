# GOD Control Specification

## 1. Tanım

GOD, Batai sistemindeki en yüksek yetkili insan rolüdür.

Authority precedence:
1. System safety/policy
2. GOD
3. Approved customer requirements
4. Director
5. Leads
6. Workers

## 2. GOD Komut Scope'ları

### Global Policy
Tüm projelerde geçerli.
Örnek:
- production deploy GOD approval gerektirir.

### Project Policy
Tek projede geçerli.
Örnek:
- bu projede PostgreSQL kullanılacak.

### Task Instruction
Tek görevlik.
Örnek:
- TASK-142 yeniden yapılacak.

## 3. GOD Yetkileri

- requirement değiştirme
- technical constraint koyma
- agent ekleme/kaldırma
- hierarchy değiştirme
- model/provider değiştirme
- reasoning seviyesini değiştirme
- task pause/cancel/retry
- kalite eşiğini değiştirme
- bütçe belirleme
- Director kararını override etme
- müşteriyle sorulacak soruları düzenleme

## 4. GOD Mesaj İşleme

GOD doğal dil ile Director'a yazar.

Batai mesaj metadata'sına:
```json
{
  "source": "GOD",
  "authority": 100
}
```
ekler.

Director mesajı:
- policy,
- project decision,
- task instruction,
- question,
- review request
olarak sınıflandırır.

## 5. Çatışma

Müşteri ve GOD kararı çelişirse:
- GOD kararı effective decision olur,
- Decision Ledger'a kayıt edilir,
- Director müşteri gereksiniminde uyumsuzluk varsa GOD'a bildirir.
