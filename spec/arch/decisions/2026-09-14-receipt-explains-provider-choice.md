---
id: DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE
status: active
governs: product
realized: tests/contract_receipt.rs::every_operation_with_an_executor_answers_with_a_receipt
supersedes: []
superseded-by: null
establishes: [CTR.WIRE.INIT-DATA, CTR.WIRE.BUILD-DATA, CTR.WIRE.DUMP-DATA, CTR.WIRE.LOAD-DATA, CTR.WIRE.EXTENSIONS-DATA, CTR.WIRE.EXTENSIONS-INVENTORY-DATA, CTR.WIRE.SYNTAX-DATA, CTR.WIRE.MAKE-DATA, CTR.WIRE.PUBLISH-DATA]
changes: [CTR.WIRE.INIT-DATA, CTR.WIRE.BUILD-DATA, CTR.WIRE.DUMP-DATA, CTR.WIRE.LOAD-DATA, CTR.WIRE.EXTENSIONS-DATA, CTR.WIRE.EXTENSIONS-INVENTORY-DATA, CTR.WIRE.SYNTAX-DATA, CTR.WIRE.MAKE-DATA, CTR.WIRE.PUBLISH-DATA]
---

# Квитанция объясняет выбор провайдера, а не предлагает его

**Решение.** У всех операций одна форма: `provider.selected`, `provider.origin`
(`default` или `override` с именем файла), `provider.skipped[]` с причиной у
каждого пропущенного и `provider.endpoint` с режимом и адресом там, где исполнение
шло через сессию. Неиспользованные альтернативы не перечисляются.

**Почему.** Перечень доступных кандидатов — приглашение выбирать, а выбор
вызывающему не принадлежит. Объяснить принятое решение нужно, предложить другое —
нет.

**Не затрагивает.** Запрет выдумывать развилку, которой нет: с матрицей развилка
настоящая, и в квитанцию она входит как объяснение, а не как меню.
