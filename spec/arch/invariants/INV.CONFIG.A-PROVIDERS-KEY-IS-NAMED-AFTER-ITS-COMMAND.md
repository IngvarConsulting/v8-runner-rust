---
id: INV.CONFIG.A-PROVIDERS-KEY-IS-NAMED-AFTER-ITS-COMMAND
status: planned
governs: product
decision: DEC.2026-09-21.COMMANDS-FOLLOW-THE-GIT-VOCABULARY
check: null
scope: [config]
---

# Ключ выбора исполнителя назван именем команды

Опубликованная схема знает ровно тринадцать ключей `providers.*`, и каждый совпадает с
именем команды словаря: `push`, `pull`, `apply`, `reset`, `upload`, `download`, `diff`,
`make`, `convert`, `extensions`, `infobase.create`, `infobase.dump`, `infobase.restore`;
`providers.publish`, `providers.check` и `providers.sessions` отклоняются.
