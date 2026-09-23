---
id: INV.CONFIG.A-PROVIDERS-KEY-IS-NAMED-AFTER-ITS-COMMAND
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/270
---

# Ключ выбора исполнителя назван именем команды

Опубликованная схема будет знать ровно тринадцать ключей `providers.*`, и каждый совпадёт
с именем команды словаря: `push`, `pull`, `apply`, `reset`, `upload`, `download`, `diff`,
`make`, `convert`, `extensions`, `infobase.create`, `infobase.dump`, `infobase.restore`;
`providers.publish`, `providers.check` и `providers.sessions` отклоняются.

Замер 23.09.2026: схема знает шестнадцать ключей прежнего словаря — `build`, `dump`,
`load`, `syntax`, `init`, `infobase.configuration.export` среди них, а `apply`, `reset`,
`diff` и `convert` отсутствуют. Разрыв и критерии его устранения — в `gap`.
