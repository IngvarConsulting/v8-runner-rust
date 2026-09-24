---
id: INV.CLI.LOCK-CONFLICT-IS-ONE-ERROR
check: [tests/cli_build.rs::build_text_workspace_lock_conflict_prints_single_error]
---

# Занятый каталог даёт одну понятную ошибку

Столкновение с чужой блокировкой печатается одним сообщением, а не потоком неудач вложенных шагов.
