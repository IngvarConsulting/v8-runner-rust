---
id: DEC.2026-09-14.AGENT-SPEAKS-JSON-WITHOUT-A-PTY
status: superseded
governs: product
realized: null
supersedes: []
superseded-by: DEC.2026-09-15.AGENT-IS-DRIVEN-BY-THE-SYSTEM-SSH-CLIENT
establishes: [INV.PLATFORM.AGENT-READINESS-IS-AUTHENTICATION, INV.PLATFORM.AGENT-SESSION-OPENS-IN-JSON-MODE]
---

# Агентская сессия идёт без псевдотерминала и отвечает JSON

**Решение.** Раннер подключается к агентскому shell по SSH без запроса
псевдотерминала, первой командой ставит `options set --show-prompt=no
--output-format=json` и разбирает ответы как JSON-массивы. Решение принимается по
полю `type` и закрытому множеству `error-type`; текст `message` переносится как
улика и входом решения не является.

**Почему.** Замер 13.09.2026: с запросом псевдотерминала агент отвечает «PTY
allocation request failed» и рвёт сессию; в текстовом режиме границу ответа
пришлось бы искать по приглашению, а в JSON-режиме приглашения нет вовсе. Закрытое
множество `error-type` — единственная часть ответа, которую документация называет
машинной.

**Цена.** Клиент SSH нужен свой, в процессе: внешний `ssh` требует свежего
OpenSSH для неинтерактивной передачи пароля, а пустое имя пользователя принимает
только через отдельный ключ.
