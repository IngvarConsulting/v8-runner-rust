---
id: INV.PLATFORM.DIRECT-IBCMD-REFUSES-A-FILE-BASE-SERVED-BY-IBSRV
check:
  - tests/cli_ibcmd_server_guard.rs::a_running_ibsrv_blocks_direct_download_and_extension_inventory
  - tests/cli_ibcmd_server_guard.rs::a_running_ibsrv_for_another_base_leaves_direct_download_available
  - tests/cli_ibcmd_server_guard.rs::an_ibsrv_over_the_same_data_file_under_another_directory_blocks_download
---

# Прямой `ibcmd` отказывает, если файловую базу обслуживает локальный `ibsrv`

На Windows и Linux перед прямым вызовом `ibcmd` к файловой базе раннер ищет доступный
пользователю раннера локальный процесс `ibsrv` с однозначным абсолютным `--db-path` или `--database-path`.
Сравнивается физический каталог или файл `1Cv8.1CD`. На Linux аргумент относится к тому же пространству
монтирования и корню файловой системы. Совпадение даёт отказ `infobase_busy` с PID до запуска
утилиты. Сервер остаётся работать. Проверка применяется и к вложенным вызовам;
`--dry-run` по-прежнему не запускает платформу.

Общая точка запуска `IbcmdDsl::run_with` проверяет файловое подключение независимо от
команды и выбора исполнителя. Регрессии через явный `download`, стандартный
`extensions list` и вложенный шаг `upload .cfe` фиксируют обход этой точки.

Процесс другого пользователя, аргументы или образ которого закрыты, и недоступный перечень
процессов проверку не останавливают (#780).

Снимок доступных аргументов подтверждает совпадение в момент проверки. Допуск прямого
доступа по окну обслуживания и подтверждённой топологии выполняется отдельно.
