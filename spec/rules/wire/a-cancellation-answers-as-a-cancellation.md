---
id: INV.WIRE.A-CANCELLATION-ANSWERS-AS-A-CANCELLATION
check:
  - src/support/error.rs::a_cancellation_is_recognised_through_every_wrapper
  - src/support/error.rs::a_timeout_or_an_unrelated_failure_is_not_a_cancellation
  - src/use_cases/result.rs::every_cancellation_answers_as_cancelled
  - tests/cli_publish.rs::publish_interrupted_after_webinst_started_answers_in_its_form
  - tests/cli_launch.rs::a_wait_ready_interrupted_after_the_client_started_is_a_cancellation
  - tests/architecture_guardrails.rs::a_cancellation_is_classified_only_by_its_owner
---

# Отмена отвечает отменой

Прерывание оператором классифицируется как отмена у любой команды, где бы его ни заметили:
на безопасной точке команды, в снятом процессе, в брошенной команде агента или общей сессии
EDT, в прерванной загрузке по HTTP. Конверт командной строки называет такой отказ родом
`interruption` и кодом `cancelled`; код выхода следует из рода
([INV.CLI.EXIT-CODE-REFLECTS-THE-FAILURE-KIND](../cli/exit-code-reflects-the-failure-kind.md)).
Истёкший предел шага отменой не является и сохраняет свой род.
