---
id: INV.PLATFORM.PROCESS-SPAWN-STAYS-IN-PLATFORM
status: active
governs: process
decision: DEC.2026-04-20.PLATFORM-DSL-STAYS-OUT-OF-ORCHESTRATION
check: tests/architecture_guardrails.rs::raw_process_spawn_apis_stay_inside_platform_layer
scope: [platform, use-cases]
---

# Запуск процессов живёт только в слое платформы

Прямые вызовы запуска процессов допускаются лишь внутри `platform`. Слой сценариев процессов не порождает.
