---
id: INV.CLI.LOCK-CONFLICT-IS-ONE-ERROR
check:
  - tests/cli_build.rs::build_text_workspace_lock_conflict_prints_single_error
  - tests/cli_infobase.rs::restore_on_a_busy_workspace_prints_one_envelope
---

# Занятый каталог даёт одну понятную ошибку

Столкновение с чужой блокировкой печатается одним сообщением, а не потоком неудач вложенных шагов.
