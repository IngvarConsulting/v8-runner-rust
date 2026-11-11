---
id: CTR.WIRE.CLUSTER-START-DATA
version: 1
artifact: docs/schemas/command-data/cluster-start.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/cluster.rs::start_raises_the_agent_then_ras_and_waits_for_rac
  - src/use_cases/managed_stand/cluster.rs::a_busy_regport_moves_the_cluster_to_a_new_registry_over_its_database
---

# `data` команды `cluster start`

`action`: `started`, `already_running` или `planned` в превью. Состояние `state`: `not_running`, `starting`,
`started_by_this_copy`, `foreign` — закреплённый порт слушает чужой процесс, его pid в `occupant_pid`. `pid` — агент
`ragent`, `ras_pid` — сервер администрирования; `connection` — строка соединения с базой кластера (`Srvr=` на `regport`),
`agent` и `ras` — их адреса, `worker_range` — диапазон рабочих процессов, `registry_dir` — каталог реестра. `ready`
говорит, что ответил `rac cluster list`, `registered` — что база создана в этом реестре. Если запуск завёл новый
реестр — по `--new-regport` или сам, потому что закреплённый порт занят чужим процессом
(`INV.USE-CASES.A-BUSY-REGPORT-IS-REFUSED`), — `previous_regport` называет `regport` прежнего реестра, а `connection`,
`agent` и `worker_range` — новые порты. Во втором случае `warnings` конверта называет порт и pid того, кто его занял.
`stale_record` говорит, что запись была, а процесса из неё уже нет.

Адрес кластера живёт только в состоянии стенда: `v8project*.yaml` раннер не правит, адрес у `origin` не пишет. Берите
его из ответа `start` или `status` и не кешируйте.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "started_by_this_copy",
  "action": "started",
  "pid": 6312,
  "ras_pid": 16980,
  "version": "8.5.4.2028",
  "connection": "Srvr=127.0.0.1:46761;Ref=demo",
  "agent": "127.0.0.1:46760",
  "ras": "127.0.0.1:25425",
  "worker_range": "46762:46777",
  "registry_dir": ".local/infobases/origin/managed/cluster/registry-46761",
  "log_path": ".local/infobases/origin/managed/cluster/cluster.log",
  "ready": true,
  "registered": false,
  "duration_ms": 9100
}
```
