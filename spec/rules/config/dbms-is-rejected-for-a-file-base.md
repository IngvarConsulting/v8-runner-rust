---
id: INV.CONFIG.DBMS-IS-REJECTED-FOR-A-FILE-BASE
check: [src/config/validate.rs::file_connection_rejects_dbms_contract]
---

# Секция СУБД у файловой базы отклоняется

Указанный доступ к СУБД при файловом подключении — ошибка валидации: конфиг не должен выглядеть серверным, когда база файловая.
