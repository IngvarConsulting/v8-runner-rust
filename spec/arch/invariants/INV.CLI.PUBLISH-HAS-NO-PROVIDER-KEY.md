---
id: INV.CLI.PUBLISH-HAS-NO-PROVIDER-KEY
status: active
governs: product
decision: DEC.2026-09-14.RUNNER-PUBLISHES-TO-A-WEB-SERVER
check: tests/cli_publish.rs::publish_rejects_a_provider_override_because_there_is_no_choice
scope: [config]
---

# У публикации нет выбора исполнителя

Ключ переопределения провайдера для публикации отклоняется: развилки у операции нет.
