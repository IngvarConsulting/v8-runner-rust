---
id: DEC.2026-09-21.OLD-NAMES-ARE-HIDDEN-SYNONYMS-FOR-ONE-CYCLE
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [INV.CLI.A-HIDDEN-SYNONYM-IS-ABSENT-FROM-HELP, INV.CLI.A-SYNONYM-ANSWERS-UNDER-THE-NEW-NAME, INV.CONFIG.A-KEY-SYNONYM-IS-MARKED-DEPRECATED-IN-THE-SCHEMA]
changes: [CTR.CONFIG.V8PROJECT-SCHEMA]
---

# Прежние имена живут один цикл скрытыми синонимами

**Решение.** Прежние имена принимаются ровно один цикл выпуска как скрытые синонимы: в
справке их нет, ответ называет новое имя — поле `command` конверта и квитанция прежнего
не знают, — команда выполняется под новым именем. Команды: `config init` → `init`,
`bootstrap` → `clone`, `build` → `push`, `dump` → `pull`, `infobase configuration
export` → `download`, `syntax` → `check`, `load` → `upload`, `load --mode merge` →
`upload --mode combine`. Ключи команд: `test --no-build` → `--no-push`, `build
--full-rebuild` → `push --full`, `--discard-uncommitted` → `--force`. Ключи
конфигурации: `infobase:` → `infobases.origin`; `providers.build`, `providers.dump`,
`providers.init`, `providers.load`, `providers.infobase.configuration.export` →
`providers.push`, `providers.pull`, `providers.infobase.create`, `providers.upload`,
`providers.download`; `build.partialLoadThreshold` → `push.partialLoadThreshold`.
Прежний `infobase:` принимается и в проектном файле — с предупреждением о переезде в
местный слой; прежний и новый ключ в одном конфиге — отказ валидации. Ключи, у которых
пары нет: `--source-set <имя>` — синоним позиционного набора; `--state working` — без
ключа, `--state database` → `--state db`; `--mode incremental|partial` — без ключа,
режим решает память; `--mode full` — отказ с именем `pull --force`, потому что
молчаливое отображение обошло бы сторожа. Сверх сайта: прежний ключ остаётся в
опубликованной схеме с пометкой `deprecated` — схема обязана совпадать с моделью и
молчать о принимаемом ключе не вправе. Единственное имя без синонима — `init`: под ним
живёт другая команда, и вызов прежнего `init` в проекте с базой в конфиге отказывает и
называет `infobase create`. Через цикл синонимы снимаются отдельной задачей с датой.

**Почему.** Сценарии сборки и чужие конфигурации переживают выпуск: ломать их одним днём
нельзя, а держать два имени вечно — значит держать два словаря.

**Не затрагивает.** `extensions delete` и его судьбу в `push --delete`: это предмет
расширений.

Источник: [`cli.html#synonyms`](../../../docs/site/cli.html#synonyms).
