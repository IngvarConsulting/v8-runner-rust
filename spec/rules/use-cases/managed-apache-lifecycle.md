---
id: INV.USE-CASES.MANAGED-APACHE-LIFECYCLE
check:
  - src/use_cases/managed_stand/web.rs::a_file_infobase_starts_and_publishes_with_webinst_without_a_cluster
  - tests/cli_web.rs::a_named_file_base_can_plan_its_own_apache_without_a_cluster
  - tests/cli_web.rs::a_named_file_base_can_plan_publication_without_source_sets
  - src/use_cases/managed_stand/web.rs::web_start_starts_only_apache_when_the_cluster_is_stopped
  - src/use_cases/managed_stand/cluster.rs::cluster_stops_only_the_apache_it_started_as_a_step
  - src/use_cases/managed_stand/cluster.rs::a_failed_apache_takes_down_the_cluster_and_the_postgres_this_start_raised
  - src/use_cases/managed_stand/web.rs::publication_rollback_restores_both_files
  - src/use_cases/managed_stand/web.rs::publish_runs_the_located_webinst_on_the_private_config_and_reloads_apache
  - src/use_cases/managed_stand/web.rs::a_failed_webinst_restores_the_config_and_the_descriptor
  - src/use_cases/managed_stand/web.rs::a_publication_apache_rejects_is_rolled_back_and_apache_works_on_the_old_config
  - src/use_cases/managed_stand/web.rs::both_reload_strategies_activate_the_same_apache
  - src/use_cases/managed_stand/web.rs::webinst_comes_from_the_platform_search_not_from_the_cluster_record
  - src/use_cases/managed_stand/web.rs::status_marks_the_publication_stale_after_the_regport_moves
  - src/use_cases/managed_stand/web.rs::a_preview_with_apache_stopped_names_the_pinned_address_and_writes_nothing
  - src/use_cases/managed_stand/web.rs::web_debug_preparation_edits_only_the_descriptor_and_reuses_it_on_repeat
---

# Частный Apache именованной базы

Для файловой ИБ `web start` запускает Apache с модулем из каталога выбранного `webinst`. Публикация
указывает абсолютный `File=…` без реквизитов. Запуск и публикация не требуют кластера.

При `infobase.web.managed: true` кластер запускает Apache после своего запуска
и останавливает его перед остановкой кластера. Если Apache не поднялся, `cluster start`
гасит кластер и PostgreSQL, поднятые этим запуском. `web start` делает экземпляр
явным и запускает только Apache. Кластер и его HTTP-отладка запускаются через `cluster start`.
`publish` использует `webinst` выпуска кластера, свой конфиг и текущий
адрес кластера, а после него перезагружает Apache. Неудача `webinst` или перезагрузки
возвращает конфиг и `default.vrd`, и Apache работает на прежнем конфиге. Превью
`publish` при остановленном Apache называет закреплённый адрес и ничего не пишет.
После переноса `regport` статус отмечает устаревшую публикацию.

Явная подготовка HTTP-отладки меняет только элемент `debug` в `default.vrd` и активирует
его тем же механизмом Apache с откатом. Готовая публикация с тем же адресом отладки
не перезагружается. Отметка незавершённой активации убирается после успешной перезагрузки,
а повтор подготовки завершает активацию после прерывания. При обычном `publish` настройка
из `infobase.web.debugger-url` снова входит в дескриптор до активации.
