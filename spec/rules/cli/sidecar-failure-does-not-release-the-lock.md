---
id: INV.CLI.SIDECAR-FAILURE-DOES-NOT-RELEASE-THE-LOCK
check: [src/use_cases/workspace_lock.rs::an_unwritable_sidecar_does_not_release_the_lock]
---

# Ошибка записи метаданных не снимает блокировку

Невозможность записать диагностический файл не отменяет владение каталогом и не разрешает параллельный запуск.
