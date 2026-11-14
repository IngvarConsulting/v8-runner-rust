---
id: INV.PLATFORM.A-MANAGED-APACHE-IS-PRIVATE
check:
  - src/platform/apache.rs::a_private_configuration_names_only_its_loopback_port_and_files
  - src/platform/service.rs::a_linux_listener_is_identified_by_its_socket
---

# Частный Apache запускается отдельно

Конфиг, журнал и PID-файл лежат под `localPath`. Порт вне динамического диапазона.
Apache слушает loopback, его модуль взят из выпуска кластера. Занятый порт ведёт
к выбору другого порта, занятый конфиг — к отказу с PID. Системные команды
управления Apache не используются: раннер останавливает и перезапускает
свой экземпляр по записи службы и идентичности процесса.
