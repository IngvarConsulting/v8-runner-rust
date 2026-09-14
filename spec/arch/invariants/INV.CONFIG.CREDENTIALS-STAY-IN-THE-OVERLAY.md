---
id: INV.CONFIG.CREDENTIALS-STAY-IN-THE-OVERLAY
status: active
governs: product
decision: DEC.2026-05-02.LOCAL-OVERLAY-CARRIES-MACHINE-SETTINGS
check: tests/cli_bootstrap.rs::bootstrap_json_success_keeps_credentials_in_local_overlay_only
scope: [config]
---

# Учётные данные попадают только в локальный слой

Команда заведения проекта пишет учётные данные исключительно в некоммитируемый локальный слой.
