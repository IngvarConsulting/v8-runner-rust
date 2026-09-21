---
id: DEC.2026-04-21.INFOBASE-SECTION-OWNS-CONNECTION-AND-CREDENTIALS
status: superseded
governs: product
realized: tests/cli_bootstrap.rs::bootstrap_json_success_keeps_credentials_in_local_overlay_only
supersedes: []
superseded-by: DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT
establishes: [INV.CONFIG.DBMS-IS-REJECTED-FOR-A-FILE-BASE, INV.CONFIG.TOP-LEVEL-CONNECTION-IS-NOT-ACCEPTED]
---

# Подключение и учётные данные базы живут в одной секции

**Решение.** Строка подключения и учётные данные информационной базы лежат в секции
`infobase`; отдельных ключей верхнего уровня для них нет. Там же объявляется
доступ к СУБД для серверной базы. Учётные данные СУБД не заменяют учётных данных
информационной базы: это разные пользователи разных систем.

**Почему.** Рассыпанные по корню ключи не показывали, что описывают одну и ту же
базу, и позволяли собрать конфиг, где строка подключения говорит про одну базу, а
учётные данные — про другую.

**Не затрагивает.** Вид цели: он объявляется отдельно.
