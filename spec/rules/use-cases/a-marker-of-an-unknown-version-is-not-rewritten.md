---
id: INV.USE-CASES.A-MARKER-OF-AN-UNKNOWN-VERSION-IS-NOT-REWRITTEN
check:
  - tests/cli_infobase_owner.rs::a_marker_of_an_unknown_version_stops_a_write
---

# Метку незнакомой версии не переписывают

Метку владельца, версия которой не совпадает с версией формы этого раннера
(`CTR.USE-CASES.INFOBASE-OWNER-MARKER`), команда записи не переписывает: она отказывает и
называет версию метки и версию, которую знает сам. Команда чтения идёт дальше и говорит об
этом. Команды записи и чтения определены в `INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
