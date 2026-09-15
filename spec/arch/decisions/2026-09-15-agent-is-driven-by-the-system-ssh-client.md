---
id: DEC.2026-09-15.AGENT-IS-DRIVEN-BY-THE-SYSTEM-SSH-CLIENT
status: active
governs: product
realized: tests/cli_dump_agent.rs::managed_agent_dumps_through_the_system_ssh_client_and_reads_the_result_from_disk
supersedes: [DEC.2026-09-14.AGENT-SPEAKS-JSON-WITHOUT-A-PTY]
superseded-by: null
establishes: [INV.PLATFORM.AGENT-READINESS-IS-AUTHENTICATION, INV.PLATFORM.AGENT-SESSION-OPENS-IN-JSON-MODE]
---

# Агентом правит системный `ssh`: без псевдотерминала, ответы в JSON

**Решение.** Раннер не носит SSH-клиента в себе. Сессию к агенту открывает
системный `ssh` (`tools.designer_agent.ssh` или первый из `PATH`) с ключом `-T`,
пароль уходит через `SSH_ASKPASS` при `SSH_ASKPASS_REQUIRE=force`, а пустой логин
базы без пользователей передаётся как есть через `-l ''`. Команды пишутся по одной
в открытый stdin; первой идёт `options set --show-prompt=no --output-format=json`,
после неё каждый ответ — один JSON-массив, и его границу даёт разбор, а не поиск
приглашения. Итог команды решают `type` и закрытое множество `error-type`;
`message` переносится как улика. Отказы до первого ответа тоже структурны: «там
никто не слушает» говорит TCP-соединение (ошибку типизирует ОС), а любая неудача
`ssh` после принятого соединения — один типизированный отказ с кодом выхода и
stderr как уликой; текст `ssh` («Permission denied», «Connection refused») решений
не принимает. Управляемый агент поднимается как
`1cv8 DESIGNER <база> /AgentMode /AgentPort … /AgentListenAddress 127.0.0.1
/AgentSSHHostKeyAuto|/AgentSSHHostKey … /AgentBaseDir <workPath>/agent/base` и
гасится командой `common shutdown`.

**Почему.** Предшественник предполагал собственный SSH-клиент в процессе и назвал
это ценой. Владелец выбрал системный клиент: OpenSSH с `SSH_ASKPASS_REQUIRE`
(8.4+) есть на каждой машине CI, у него своя криптография и свой цикл обновлений,
а раннеру остаётся ровно одна забота — протокол агента. Замер 13.09.2026 показал,
что с псевдотерминалом агент рвёт сессию, а без JSON-режима приглашение
приклеивается к первому ответу; оба факта унаследованы.

**Цена.** Пароль проходит через окружение дочернего `ssh` и askpass-скрипт, а не
через сокет в процессе; хост-ключ агента не проверяется (`StrictHostKeyChecking=no`)
— он либо сгенерирован платформой на этой же машине, либо назван пользователем в
`attach`. Windows-путь (`askpass.cmd`, `UserKnownHostsFile=NUL`) написан по
документации OpenSSH и вживую не проверен.
