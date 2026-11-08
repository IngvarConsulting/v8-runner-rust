---
id: INV.USE-CASES.THE-AGENT-LEADS-A-DEFAULT-CHAIN
check:
  - src/domain/capability.rs::the_agent_leads_the_file_and_cluster_chains
  - src/domain/capability.rs::a_whole_push_or_pull_of_a_file_base_leads_with_ibcmd
  - src/config/model.rs::a_file_base_orders_push_and_pull_by_size_without_a_key
  - tests/contract_receipt.rs::every_operation_with_an_executor_answers_with_a_receipt
  - tests/contract_receipt.rs::a_file_base_picks_the_executor_by_the_size_of_the_operation
---

# Агент ведёт цепочки по изменившемуся, `ibcmd` — целые обмены файловой базы

У `push`, `pull` и `download` строка матрицы файловой базы — `agent`, `designer`, `ibcmd`, у
кластера — `agent`, `designer`. У `infobase dump` и `infobase restore` строка файловой базы и
кластера — `agent`, `designer`. Агент ни в одной строке матрицы не помечен экспериментальным.

Без ключа `providers.*` у `push` и `pull` файловой базы порядок выбирает объём операции. Если
каждый набор, который команда загружает или выгружает, идёт целиком, первым идёт `ibcmd`, за
ним остальные в порядке строки; если хоть один набор идёт по изменившемуся или выборкой — строка
как есть. Целиком идут `push --force`, `push --full`, отправка копии базы и набор, разбор
которого не с чем сравнить, а у проекта EDT — только `--force` и `--full`; у `pull` —
`--force` и выгрузка, которую план делает полной поверх каталога. Сужение формой проекта действует и здесь. Ключ `providers.*` объём не переписывает;
у кластера, автономного сервера и при подключённом агенте (`tools.designer_agent.attach`)
порядок строки не меняется. Квитанция умолчания, упорядоченного по объёму, называет причину
полем `reason` — `whole` или `by_changes`; у остальных квитанций поля нет.
