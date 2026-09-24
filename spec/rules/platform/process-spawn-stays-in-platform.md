---
id: INV.PLATFORM.PROCESS-SPAWN-STAYS-IN-PLATFORM
check: [tests/architecture_guardrails.rs::raw_process_spawn_apis_stay_inside_platform_layer]
---

# Запуск процессов живёт только в слое платформы

Прямые вызовы запуска процессов допускаются лишь внутри `platform`. Слой сценариев процессов не порождает.
