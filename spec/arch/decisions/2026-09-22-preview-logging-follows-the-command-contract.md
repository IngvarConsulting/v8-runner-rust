---
id: DEC.2026-09-22.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT
status: active
governs: product
realized: [tests/contract_previews.rs::every_preview_leaves_a_line_in_the_action_log, tests/cli_configuration_transition.rs::apply_reset_preview_does_not_dispatch_or_create_work_path]
supersedes: [DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED]
superseded-by: null
establishes: [INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY, INV.CLI.LAUNCH-PREVIEW-NAMES-PROGRAM-AND-ARGS, INV.CLI.PREVIEW-DISPATCHES-NOTHING, INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP, INV.CLI.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT]
changes: [INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY]
---

# Журнал превью определяется контрактом команды

**Решение.** Сохраняется граница прежнего превью: проверка запроса и поиск
утилиты проходят до возврата плана; платформа не запускается, цели, артефакты
и состояние обнаружения изменений не создаются. Правила именования запуска
и отказа при отсутствии платформы остаются прежними.

Замена прежнего решения ограничена журналом новых команд apply/reset:
их preview не создаёт workPath или журнал. Остальные команды сохраняют
прежнее журналирование превью. Универсальное утверждение
INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY выводится из обращения; различие областей
закрепляет INV.CLI.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT.

**Почему.** Новые отдельные переходы конфигурации требуют чтения плана без
записи файлов, а менять наблюдаемое поведение старых команд совместимый
релиз не должен.
