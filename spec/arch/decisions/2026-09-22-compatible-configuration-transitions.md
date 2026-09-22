---
id: DEC.2026-09-22.COMPATIBLE-CONFIGURATION-TRANSITIONS
status: active
governs: product
realized: tests/cli_configuration_transition.rs::transitions_use_exact_designer_operation_and_extension_for_file_and_cluster
supersedes: []
superseded-by: null
establishes: [CTR.WIRE.APPLY-DATA, CTR.WIRE.RESET-DATA, INV.RUNTIME.SEPARATE-CONFIGURATION-TRANSITIONS]
changes: [CTR.WIRE.APPLY-DATA, CTR.WIRE.RESET-DATA]
---

# Совместимый раннер разделяет загрузку, применение и откат

**Решение.** Для переходного цикла Unica добавить load --no-apply, apply и
reset --force, сохранив поведение старого load без нового ключа. Цель —
основная конфигурация либо явно названное расширение. Исполнитель Designer;
автономная база и управление чужими сеансами не обещаются.

Загрузка без применения не обновляет конфигурацию БД. Apply делает только
обновление БД; reset возвращает рабочую конфигурацию к конфигурации БД,
не означает удаления расширения и требует force без памяти поколений.

Перед выполнением перехода расширения Designer сохраняет выбранное расширение
во временный уникальный CFE. Нулевой код выхода и новый непустой обычный файл
подтверждают наличие цели; отсутствие доказательства запрещает переход.
Проверка не требует ibcmd или DBMS-реквизитов. Preview этой проверки не запускает
и наличие расширения не подтверждает. Основание — замер Designer 8.3.27.2074:
apply отсутствующего расширения вернул 0 без действия, DumpCfg отсутствующего
расширения вернул 1 без артефакта; существующее расширение выгружено в CFE.

Preview новых apply/reset не создаёт workPath и журнал, не берёт lock и не
запускает платформу; это точная граница нового среза относительно
INV.CLI.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT. Выполнение использует существующий lock,
connection builder и политику CriticalNonAbortable. Успех после отложенного
прерывания остаётся успехом с диагностикой. Частичный эффект не стирается
при ошибке следующего этапа. Новые команды не публикуются как MCP tools.
