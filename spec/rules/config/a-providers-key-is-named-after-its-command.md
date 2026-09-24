---
id: INV.CONFIG.A-PROVIDERS-KEY-IS-NAMED-AFTER-ITS-COMMAND
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/201
---

# Ключ выбора исполнителя назван именем команды

Опубликованная схема знает ровно тринадцать ключей `providers.*`, и каждый совпадает с
именем команды словаря: `push`, `pull`, `apply`, `reset`, `upload`, `download`, `diff`,
`make`, `convert`, `extensions`, `infobase.create`, `infobase.dump`, `infobase.restore`;
`providers.publish`, `providers.check` и `providers.sessions` отклоняются.
