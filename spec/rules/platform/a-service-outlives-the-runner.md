---
id: INV.PLATFORM.A-SERVICE-OUTLIVES-THE-RUNNER
check:
  - src/use_cases/managed_stand/mod.rs::a_service_that_would_not_survive_is_refused_as_the_environment
  - src/platform/service.rs::a_hidden_service_writes_to_its_log_and_gets_a_real_pid
  - src/platform/service.rs::a_job_that_kills_on_close_and_forbids_leaving_refuses_the_service
  - tests/architecture_guardrails.rs::a_service_starts_only_through_the_process_executor_mode
---

# Служба переживает выход раннера

Служба запускается вне Job раннера, со своей скрытой консолью и без наследованных дескрипторов родителя, вывод
идёт в журнал под `workPath`. Если раннер сам работает в Job с kill-on-close, служба отделяется
`CREATE_BREAKAWAY_FROM_JOB`. Job этого не разрешает — отказ `environment_unavailable` до запуска службы.
