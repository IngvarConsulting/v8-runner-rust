---
id: INV.USE-CASES.DELETING-THE-WORKPATH-KEEPS-STANDS-AND-BASES
check:
  - src/use_cases/managed_stand/cluster.rs::deleting_the_work_path_keeps_the_stand_and_its_base
  - src/platform/service_record.rs::a_stand_service_lives_under_the_local_path
---

# Удаление `workPath` оставляет стенды и базы

`workPath` пересобирается, и его можно удалить. Данные стендов, их записи, закреплённые порты,
реестр кластера и признак созданной базы лежат под `localPath`, поэтому после удаления
`workPath` `dbms status` видит свою СУБД, а `cluster start` поднимает тот же кластер над той же
базой. Файловая база по умолчанию `<localPath>/db` тоже остаётся.

Память о базе живёт в `workPath` и пропадает вместе с ним: следующий `push` отказывает
`untracked_base`, связь восстанавливает `pull --force`.
