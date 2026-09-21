---
id: INV.CONFIG.A-CLUSTER-SECTION-IS-REJECTED-OUTSIDE-A-CLUSTER-BASE
status: active
governs: product
decision: DEC.2026-09-21.THE-CLUSTER-SECTION-HOLDS-RAS-AND-TWO-ADMIN-LEVELS
check: src/config/validate.rs::the_cluster_section_is_rejected_for_a_file_base_and_a_standalone_server
scope: [config]
---

# Секция кластера вне кластерной базы отклоняется

Секция `cluster` держит то, что есть только у кластера: адрес сервера администрирования и
его администраторов. У файловой базы сервера, ведущего сеансы, нет; автономным сервером
`ras` не управляет. Поэтому секция `cluster` рядом с `File=` или рядом с секцией
`standalone` — ошибка валидации: конфиг не должен выглядеть кластерным, когда база не в
кластере.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`deployments.html#d-which`](../../../docs/site/deployments.html#d-which),
[`platform.html#t60`](../../../docs/site/platform.html#t60).
