---
id: INV.CONFIG.GENERATED-TARGETS-DO-NOT-COLLIDE-WITH-SOURCES
status: active
governs: product
decision: DEC.2026-04-20.WORKPATH-IS-THE-ONLY-STATE-ROOT
check: [src/config/validate.rs::rejects_edt_source_set_path_overlapping_generated_work_target, src/config/validate.rs::rejects_reserved_source_set_name]
scope: [config]
---

# Порождённые каталоги не пересекаются с исходниками

Пути наборов и порождённые рабочие цели не совпадают; зарезервированные имена рабочих каталогов нельзя использовать как имена наборов.
