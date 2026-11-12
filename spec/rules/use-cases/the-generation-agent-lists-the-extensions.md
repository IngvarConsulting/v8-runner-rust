---
id: INV.USE-CASES.THE-GENERATION-AGENT-LISTS-THE-EXTENSIONS
check:
  - tests/cli_live_agent_portable.rs::status_deep_lists_the_extensions_in_the_generation_agent_session
  - src/use_cases/extension_inventory.rs::the_open_agent_session_lists_the_extensions_where_the_plan_admits_the_agent
---

# `status --deep` читает состав расширений в сессии агента поколения

Если поколение у `status --deep` спрашивает агент, тот же агент в той же сессии отдаёт и
состав расширений базы. Второй процесс платформы не запускается. Так бывает, если план
`extensions` берёт агента: агент есть в цепочке умолчаний или назначен ключом
`providers.extensions`. Квитанция `extensions.provider` называет агента и точку входа той же
сессии. Исполнители цепочки до агента указаны в `skipped` с причиной. Состав ответа тот же,
что у `ibcmd` той же базы: имя и активность. Если ключ назначил другого исполнителя, состав
читает он.
