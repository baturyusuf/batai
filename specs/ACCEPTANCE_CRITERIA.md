# MVP Acceptance Criteria

Batai MVP tamamlanmış sayılırsa:

1. Bir Git repo proje olarak açılabiliyor.
2. Director agent tanımlanabiliyor.
3. Director Batai tool çağrısıyla yeni agent oluşturabiliyor.
4. Agent için provider/model/reasoning seçilebiliyor.
5. Agent otomatik worktree alıyor.
6. Director `.batai/tasks/*.json` üzerinden görev atayabiliyor.
7. Batai file watcher taskı fark edip ilgili agentı başlatıyor.
8. İki bağımsız agent paralel çalışabiliyor.
9. Agent completion event oluşturabiliyor.
10. `on_success.start` ile ikinci task Director çağrılmadan tetiklenebiliyor.
11. `notify: director` ile Director otomatik uyandırılabiliyor.
12. Agent BLOCKED durumunda parent/Director bilgilendiriliyor.
13. Agent WAITING_RESOURCE durumuna alınabiliyor.
14. ResourceManager reset sonrası aynı session'ı resume edebiliyor.
15. GOD Director'a doğal dil ile müdahale edebiliyor.
16. GOD kararı Decision Ledger'a kaydediliyor.
17. Agent directives değiştiğinde ilgili agent bilgilendiriliyor.
18. GitHub issue bir taska dönüştürülüp agenta atanabiliyor.
19. Runtime crash sonrası task/session state yeniden yüklenebiliyor.
20. Local model en az bir worker rolünde çalıştırılabiliyor.
