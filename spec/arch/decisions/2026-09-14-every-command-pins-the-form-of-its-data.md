---
id: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
status: active
governs: product
realized: src/command_data.rs::generated_command_data_schemas_are_current
supersedes: []
superseded-by: null
establishes:
  - CTR.WIRE.VERSION-DATA
  - CTR.WIRE.BOOTSTRAP-DATA
  - CTR.WIRE.CONFIG-INIT-DATA
  - CTR.WIRE.TOOLS-DOWNLOAD-DATA
  - CTR.WIRE.INIT-DATA
  - CTR.WIRE.EXTENSIONS-DATA
  - CTR.WIRE.EXTENSIONS-INVENTORY-DATA
  - CTR.WIRE.BUILD-DATA
  - CTR.WIRE.LOAD-DATA
  - CTR.WIRE.TEST-DATA
  - CTR.WIRE.DUMP-DATA
  - CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA
  - CTR.WIRE.INFOBASE-DUMP-DATA
  - CTR.WIRE.INFOBASE-RESTORE-DATA
  - CTR.WIRE.CONVERT-DATA
  - CTR.WIRE.MAKE-DATA
  - CTR.WIRE.SYNTAX-DATA
  - CTR.WIRE.LAUNCH-DATA
  - CTR.WIRE.REFUSAL-DATA
  - CTR.WIRE.MCP-REFUSAL-DATA
  - CTR.WIRE.COMMAND-ENVELOPE
changes: [CTR.WIRE.COMMAND-ENVELOPE]
---

# Каждая команда закрепляет форму своего `data`

**Решение.** Конверт закрепляет оболочку ответа и про `data` не говорит ничего: там
лежит предмет команды, и у каждой команды он свой. Поэтому форму `data` закрепляет
отдельный контракт на каждую форму — со своей схемой в `docs/schemas/command-data/`,
своей версией и своей проверкой. Схема порождается из типа, который эту форму
сериализует, и лежит файлом: расхождение типа с файлом валит проверку свежести,
расхождение файла с живым ответом — проверку на настоящем прогоне. Состав полей в
схемах закрыт, поэтому добавленное поле — тоже смена формы. Соответствие «команда —
её формы» опубликовано вместе со схемами: клиент видит в конверте только `command`.

**Почему.** До этого решения `data` описывался в схеме конверта как `true` — то есть
«что угодно». Самая большая часть ответа не удерживалась ничем: поле можно было
переименовать, сделать необязательным или убрать, и ни одна проверка бы не упала.
Обратная совместимость по конверту при этом выглядела соблюдённой, а ломался ровно
тот кусок, который потребитель разбирает.

**Цена.** Форм больше, чем команд: команда отвечает разными формами на чтение и на
изменение, а отказ до диспетчеризации печатает общую для всех. Записей в реестре стало
на двадцать больше, и каждую правку ответа теперь надо проводить через артефакт и
версию. Два типа ответа, которые раньше собирались на месте как произвольный JSON
(отказ CLI и отказ адаптера MCP), пришлось сделать типами.

**Не затрагивает.** Оболочку конверта: `ok`, `command`, `duration_ms`, `warnings`,
`steps` и `error` остаются в [`CTR.WIRE.COMMAND-ENVELOPE`](../contracts/CTR.WIRE.COMMAND-ENVELOPE.md)
и меняются вместе с ней.
