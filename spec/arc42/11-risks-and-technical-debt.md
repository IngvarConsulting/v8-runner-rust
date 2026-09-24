## 11. Риски и технический долг

- Реестр правил уже введён, но его нужно поддерживать синхронно с кодом и публичной документацией.
- Публичная и внутренняя документация могут расходиться, если их не обновлять вместе с кодом.
- Общий shared interactive EDT-path теперь вынесен в `platform`, но остаётся риск регрессии к третьему публичному execution path, если новые EDT-сценарии начнут обходить общий actor/manager или документация/tests перестанут держать правила `DEC.2026-04-20.EDT-RUNS-ONE-SHOT-OR-IN-ONE-SHARED-SESSION`.
- `pull format=EDT` теперь зависит от внутреннего Designer snapshot под `workPath/designer/<source-set>`; новые изменения не должны подменять этот reverse-sync path командой `convert` или обходить staged publication target-каталога.
- Поддержка `IBCMD` остаётся уже, чем поддержка Designer.
- Provisioning contract из `DEC.2026-04-22.INIT-ENSURES-A-SERVER-INFOBASE-THROUGH-IBCMD` реализован только для исполнителя `ibcmd`; `designer` по-прежнему пропускает server infobase create step и это остаётся документированным ограничением.
- Шаг без собственного предела автоматически не обрывается (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`): зависший процесс платформы заканчивает оператор. Шаги со своим пределом — EDT, ожидание внешней обработки, тестовый прогон — обрываются как прежде. Сторож по росту файла `/Out` не сделан: он требует замера на настоящей платформе.
- MCP running cancellation/timeout с detached completion считается переходным механизмом до terminal-state semantics из `DEC.2026-04-20.CANCELLATION-COUNTS-ONLY-AFTER-A-TERMINAL-STATE`.
- `ExecutionOutcome<T>` is now canonical for `test`, `artifacts`, and `upload` domain results; the remaining risk is future reintroduction of duplicated result fields outside adapter projections.
- Система сильно зависит от локальных внешних инструментов и корректности окружения, что ограничивает герметичное тестирование.
- Многошаговые сценарии вроде `push` по нескольким `source-set` намеренно не являются атомарными.
- Workspace lock является local advisory lock и не защищает от некорректной семантики блокировок на сетевых файловых системах или от команд на разных машинах.
- Переименование `source-set.name` меняет runtime identity и может сбросить persisted state; это нужно явно подсвечивать в документации и release notes.
- Если новые архитектурные границы не фиксировать [правилами](../rules/README.md), AI-агенты или новые контрибьюторы могут переинтерпретировать важные решения как случайные детали реализации.
