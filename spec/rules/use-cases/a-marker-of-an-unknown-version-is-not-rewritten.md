---
id: INV.USE-CASES.A-MARKER-OF-AN-UNKNOWN-VERSION-IS-NOT-REWRITTEN
check:
  - tests/cli_infobase_owner.rs::a_marker_of_an_unknown_version_stops_a_write
  - tests/cli_infobase_owner.rs::a_marker_of_version_one_is_still_read
  - src/use_cases/infobase_owner.rs::a_marker_of_version_one_is_read_without_its_consent
---

# Метку незнакомой версии не переписывают

Метку владельца, версию которой этот раннер не знает (`CTR.USE-CASES.INFOBASE-OWNER-MARKER`;
он пишет версию 2 и читает ещё версию 1, которую писал 0.13.0), команда записи не
переписывает: она отказывает и называет версию метки и версию, которую знает сам. Команда
чтения идёт дальше и говорит об этом. Метку версии 1 раннер читает без её согласия `shared`, и
отказом она не становится. Команды записи и чтения определены в `INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
