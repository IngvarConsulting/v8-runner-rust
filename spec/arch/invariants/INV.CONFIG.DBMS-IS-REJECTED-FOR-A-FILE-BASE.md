---
id: INV.CONFIG.DBMS-IS-REJECTED-FOR-A-FILE-BASE
status: active
governs: product
decision: DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT
check: src/config/validate.rs::file_connection_rejects_dbms_contract
scope: [config]
---

# Секция СУБД у файловой базы отклоняется

Указанный доступ к СУБД при файловом подключении — ошибка валидации: конфиг не должен выглядеть серверным, когда база файловая.
