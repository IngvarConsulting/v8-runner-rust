---
id: INV.USE-CASES.A-FAILED-PUBLICATION-RESTORES-THE-TARGET
check:
  - src/support/fs.rs::publish_file_atomically_restores_backup_when_publish_fails
  - src/support/fs.rs::replace_file_restores_original_bytes_when_stage_disappeared
---

# Неудачная публикация файла возвращает прежнюю цель

Если опубликовать файл не удалось, откат выполняется на деле: цель снова держит прежнее
содержимое.
