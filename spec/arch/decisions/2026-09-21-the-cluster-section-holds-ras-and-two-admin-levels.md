---
id: DEC.2026-09-21.THE-CLUSTER-SECTION-HOLDS-RAS-AND-TWO-ADMIN-LEVELS
status: active
governs: product
realized:
  - src/config/loader.rs::the_cluster_section_is_read_from_the_local_layer_next_to_the_infobase_user
  - tests/cli_cluster_section.rs::the_three_credential_levels_lie_in_the_local_layer_side_by_side
supersedes: []
superseded-by: null
establishes:
  - CTR.CONFIG.V8PROJECT-SCHEMA
  - INV.CONFIG.A-CLUSTER-SECTION-IS-REJECTED-OUTSIDE-A-CLUSTER-BASE
  - INV.CLI.A-REFUSAL-NAMES-THE-MISSING-CREDENTIAL-LEVEL
changes: [CTR.CONFIG.V8PROJECT-SCHEMA]
---

# Секция `cluster` держит адрес RAS и два уровня администраторов

**Решение.** В секции базы есть секция `cluster`: `ras` — адрес сервера
администрирования; `user` и `password` — администратор кластера; `agent.address`,
`agent.user`, `agent.password` — агент центрального сервера и его администратор. Вместе
с пользователем базы (`user` секции базы) это три уровня учётных данных, и лежат они в
местном слое порознь. Потребители: `infobase create` в кластере с заполненным списком
администраторов — `cluster.user` через `SUsr`/`SPwd` строки создания; `sessions list` и
`terminate` — администратор кластера; `sessions deny` и `allow` — администратор кластера
и пользователь базы. Учётную запись центрального сервера ни одна операция раннера сама
не требует. Отказ называет, какого уровня не хватает.

**Почему.** У платформы три разных администратора, и один пароль на всех — ложь, которая
всплывает первым же отказом без объяснения, чьего пароля не хватило.

**Не затрагивает.** Что делает раннер с сервером администрирования и как поднимает
`ras`, когда адрес не объявлен, — предмет решений о сеансах.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`deployments.html#d-ras-managed`](../../../docs/site/deployments.html#d-ras-managed).
