---
id: INV.CONFIG.GENERATED-TARGETS-DO-NOT-COLLIDE-WITH-SOURCES
check:
  - src/config/validate.rs::rejects_edt_source_set_path_overlapping_generated_work_target
  - src/config/validate.rs::rejects_reserved_source_set_name
---

# Порождённые каталоги не пересекаются с исходниками

Пути наборов и порождённые рабочие цели не совпадают; зарезервированные имена рабочих каталогов нельзя использовать как имена наборов.
