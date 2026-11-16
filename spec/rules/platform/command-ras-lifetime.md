---
id: INV.PLATFORM.COMMAND-RAS-LIFETIME
check:
  - src/platform/locator.rs::rac_candidates_follow_the_configured_mask_in_version_order
  - src/use_cases/cluster_admin.rs::the_cluster_is_selected_by_server_and_manager_port
  - src/use_cases/cluster_admin.rs::a_loopback_target_matches_the_reported_local_machine_name
  - src/use_cases/cluster_admin.rs::a_foreign_listener_prevents_the_rac_probe
  - src/use_cases/cluster_admin.rs::command_ras_discards_a_mismatched_candidate_before_trying_the_exact_version
  - src/use_cases/cluster_admin.rs::cancellation_during_ras_readiness_removes_the_child_before_return
  - src/platform/process.rs::only_a_parent_tied_process_may_omit_its_stderr_log
---

# Временный RAS принадлежит одной команде

Сначала используется объявленный `infobase.cluster.ras`, затем RAS работающего стенда
раннера по проверенной записи службы. Для внешнего кластера без RAS команда поднимает
`ras cluster` на свободном порту и проверяет владельца слушателя, а затем получает список
кластеров через `rac` из того же каталога версии. К работе допускается только
пара, подтвердившая совместимость с агентом ответом `rac cluster list`. Хост и порт
менеджера из строки соединения обязаны однозначно совпасть с одной из строк списка,
даже если кластер в списке один.

Временный RAS работает до возврата из команды, под её замком при исполнении.
В Windows его снимает Job, в Linux сигнал завершения родителя. В этих системах
завершение раннера снимает вложенный процесс без записи на диск и последующего запуска
команды. Превью делает тот же запрос без замков и файлов состояния. Для временного RAS
нужен отдельный процесс CLI, которому принадлежит его политика завершения потомков. В прочих
контекстах и на других системах указывают `infobase.cluster.ras`.
