---
id: INV.USE-CASES.MANAGED-APACHE-LIFECYCLE
check:
  - tests/cli_web.rs::web_status_of_a_declared_cluster_writes_nothing
  - src/use_cases/managed_stand/cluster.rs::cluster_stops_only_the_apache_it_started_as_a_step
  - src/use_cases/managed_stand/web.rs::publication_rollback_restores_both_files
---

# Частный Apache управляемого кластера

При `infobase.web.managed: true` кластер запускает Apache после своего запуска
и останавливает его перед остановкой кластера. `web start` делает экземпляр
явным. `publish` использует `webinst` выпуска кластера, свой конфиг и текущий
адрес кластера. После переноса `regport` статус отмечает устаревшую публикацию.
