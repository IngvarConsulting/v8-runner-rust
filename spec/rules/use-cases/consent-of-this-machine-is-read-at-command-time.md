---
id: INV.USE-CASES.CONSENT-OF-THIS-MACHINE-IS-READ-AT-COMMAND-TIME
check:
  - tests/cli_infobase_owner.rs::a_consent_withdrawn_on_this_machine_stops_every_copy_at_once
  - tests/cli_infobase_owner.rs::an_unreadable_local_layer_of_the_owner_keeps_it_alive
---

# Согласие копий этой машины читается в момент команды

Согласие рабочих копий этой машины раннер читает из их местных слоёв в момент команды.
Копия согласна, если `shared: true` стоит у каждой секции её местного слоя, которая объявляет
базу; местный слой, который не прочитать, — несогласие.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
