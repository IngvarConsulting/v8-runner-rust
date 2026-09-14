---
id: INV.CLI.LOCK-CONFLICT-IS-ONE-ERROR
status: active
governs: product
decision: DEC.2026-04-20.A-COMMAND-OWNS-THE-WORKPATH-EXCLUSIVELY
check: tests/cli_build.rs::build_text_workspace_lock_conflict_prints_single_error
scope: [cli]
---

# Занятый каталог даёт одну понятную ошибку

Столкновение с чужой блокировкой печатается одним сообщением, а не потоком неудач вложенных шагов.
