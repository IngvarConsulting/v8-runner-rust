---
id: INV.USE-CASES.A-FAILED-ROLLBACK-IS-NAMED
status: active
governs: product
decision: DEC.2026-04-21.FULL-REPLACEMENT-PUBLISHES-THROUGH-STAGING
check: [src/support/fs.rs::publish_file_atomically_reports_when_publish_and_rollback_both_fail, src/support/fs.rs::replace_file_rollback_failure_is_typed_as_uncertain]
scope: [use-cases]
---

# Неудачный откат называет себя

Если вернуть прежнюю цель не удалось, ошибка несёт контекст отката: человек должен узнать, что цель требует ручной проверки, из ответа, а не из журнала.
