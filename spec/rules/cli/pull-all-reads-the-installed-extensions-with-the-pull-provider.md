---
id: INV.CLI.PULL-ALL-READS-THE-INSTALLED-EXTENSIONS-WITH-THE-PULL-PROVIDER
check:
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
  - tests/cli_pull_all.rs::ibcmd_lists_the_installed_extensions
  - tests/cli_dump_agent.rs::pull_all_through_the_agent_reads_the_installed_extensions
---

# `pull --all` читает состав базы исполнителем `pull`

Какие расширения установлены в базе, `pull --all` спрашивает у выбранного исполнителя
`pull` его вызовом списка: пакетный Конфигуратор — `/DumpDBCfgList -AllExtensions`, `ibcmd` —
`config extension list`, агент — `config extensions properties get --all-extensions`. Ответ
читается по структуре — имя на строку, поле `name`, запись JSON, — а не по тексту сообщений.
