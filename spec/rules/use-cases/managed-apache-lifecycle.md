---
id: INV.USE-CASES.MANAGED-APACHE-LIFECYCLE
check:
  - src/use_cases/managed_stand/cluster.rs::cluster_stops_only_the_apache_it_started_as_a_step
  - src/use_cases/managed_stand/cluster.rs::a_failed_apache_takes_down_the_cluster_and_the_postgres_this_start_raised
  - src/use_cases/managed_stand/web.rs::web_start_raises_a_stopped_cluster_with_debugging_then_apache
  - src/use_cases/managed_stand/web.rs::web_start_with_unresponsive_debugger_rolls_back_the_new_cluster_before_apache
  - src/use_cases/managed_stand/web.rs::web_start_rolls_back_when_debugging_disappears_during_apache_start
  - src/use_cases/managed_stand/web.rs::publication_rollback_restores_both_files
  - src/use_cases/managed_stand/web.rs::publish_runs_the_located_webinst_on_the_private_config_and_reloads_apache
  - src/use_cases/managed_stand/web.rs::a_failed_webinst_restores_the_config_and_the_descriptor
  - src/use_cases/managed_stand/web.rs::a_publication_apache_rejects_is_rolled_back_and_apache_works_on_the_old_config
  - src/use_cases/managed_stand/web.rs::both_reload_strategies_activate_the_same_apache
  - src/use_cases/managed_stand/web.rs::webinst_comes_from_the_platform_search_not_from_the_cluster_record
  - src/use_cases/managed_stand/web.rs::status_marks_the_publication_stale_after_the_regport_moves
  - src/use_cases/managed_stand/web.rs::a_preview_with_apache_stopped_names_the_pinned_address_and_writes_nothing
---

# Частный Apache управляемого кластера

При `infobase.web.managed: true` кластер запускает Apache после своего запуска
и останавливает его перед остановкой кластера. Если Apache не поднялся, `cluster start`
гасит кластер и PostgreSQL, поднятые этим запуском. `web start` делает экземпляр
явным для локальной HTTP-отладки. Он поднимает остановленный кластер с отладкой, ждёт ответа `rdbg`
и только затем запускает Apache или принимает готовый экземпляр. Перед успехом он снова проверяет
ответ отладки. При отказе завершает службы, поднятые этой командой. Превью не запускает процессы и не пишет состояние.
`publish` использует `webinst` выпуска кластера, свой конфиг и текущий
адрес кластера, а после него перезагружает Apache. Неудача `webinst` или перезагрузки
возвращает конфиг и `default.vrd`, и Apache работает на прежнем конфиге. Превью
`publish` при остановленном Apache называет закреплённый адрес и ничего не пишет.
После переноса `regport` статус отмечает устаревшую публикацию.
