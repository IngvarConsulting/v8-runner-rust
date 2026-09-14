---
id: DEC.2026-04-21.EXECUTION-OUTCOME-IS-THE-CANONICAL-RESULT
status: active
governs: product
realized: tests/cli_extensions.rs::extensions_command_streams_stage_before_pipeline_finishes
supersedes: []
superseded-by: null
establishes: [INV.USE-CASES.STEPS-RECORD-SKIPS-AND-DEGRADATION]
---

# Итог сценария имеет одну форму

**Решение.** Runner-подобные сценарии возвращают один доменный итог: статус, шаги с
их состоянием, включая пропуск и деградацию, метрики и сроки, ошибки. Эта форма
одна для CLI и MCP; адаптеры её раскладывают, но не заводят собственную.

**Почему.** Разные формы итога у похожих команд заставляют вызывающего писать
разбор под каждую и мешают увидеть, что шаг был пропущен, а не выполнен.

**Не затрагивает.** Предметные поля: каждая команда добавляет своё к общей форме.
