---
id: INV.PLATFORM.MANAGED-MODE-REQUIRES-A-LOCAL-ENDPOINT
status: active
governs: product
decision: DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE
check: tests/cli_agent_standalone.rs::launch_keys_do_not_apply_to_a_standalone_server
scope: [platform, config]
---

# Режим с запуском процесса требует локальной точки входа

Раннер поднимает процесс только там, где он вправе это сделать, — на своей машине. Удалённая точка входа в этом режиме отклоняется с названной причиной, а не обрабатывается как локальная.
