---
id: INV.USE-CASES.MAKE-NAMES-A-FAILED-CLEANUP
check: [src/use_cases/artifacts.rs::publication_message_keeps_cleanup_warning_in_result_contract]
---

# `make` называет неудачную уборку

Предупреждение уборки после публикации `make` попадает в сообщение итога и тогда, когда
рядом стоит предупреждение об отложенном прерывании: успех с неубранной копией чистым не
выглядит.
