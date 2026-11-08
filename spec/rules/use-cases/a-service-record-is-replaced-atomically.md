---
id: INV.USE-CASES.A-SERVICE-RECORD-IS-REPLACED-ATOMICALLY
check:
  - src/use_cases/managed_stand/record.rs::a_record_survives_the_round_trip_and_replaces_the_previous_one
  - src/use_cases/managed_stand/mod.rs::a_start_in_progress_is_reported_and_refused
  - src/use_cases/managed_stand/mod.rs::a_crashed_start_with_a_live_postgres_is_taken_over
---

# Запись о службе заменяется атомарно

Читатель видит старую или новую запись целиком. Пока служба запускается, запись несёт состояние `starting`.
