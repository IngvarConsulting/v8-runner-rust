---
id: CTR.WIRE.LAUNCH-DATA
version: 5
artifact: docs/schemas/command-data/launch.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_launch.rs::an_epf_wait_interrupted_after_the_client_started_answers_in_its_form
  - src/use_cases/client_mcp_readiness.rs::retains_last_tools_observation_after_reinitialize_failure_until_timeout
  - src/use_cases/client_mcp_readiness.rs::initialize_failure_before_any_tools_observation_reports_endpoint_error
  - src/use_cases/client_mcp_readiness.rs::newer_incomplete_tools_observation_replaces_the_previous_one
  - src/use_cases/client_mcp_readiness.rs::fresh_complete_tools_after_reinitialize_establish_readiness
  - src/use_cases/client_mcp_readiness.rs::cancellation_after_reinitialize_failure_keeps_observation_and_cancel_message
---

# `data` команды `launch`

Запуск клиента отчитывается тем, что именно будет запущено: программа и полный список
аргументов в `plan`, найденный исполняемый файл и то, откуда взялась платформа
(`platform_resolution.source`). Секреты в `plan` не попадают — это общее правило вывода, а
не свойство этой команды.

`pid` есть только у настоящего запуска; у превью он пуст, и `provider_dispatched: false`
говорит о том же вторым полем — вызывающему не приходится выводить факт запуска из
отсутствия значения.

**Что изменила версия 4.** Ожидание `--wait-for-exit`, прерванное уже после старта
клиента, отвечает этой формой: `ok: false`, `provider_dispatched: true` и
`external_epf_wait` с `exit_code: null` и `timed_out: false` — клиент не вышел, и срок не
истёк. Прежде такой отказ отвечал общей формой отказа.

`via` называет, каким из двух адресов цели открыта база: `connection` — административным,
`web` — клиентским. Поле есть у каждого режима, а не только у тонкого клиента, где есть
выбор: иначе его отсутствие пришлось бы толковать. `url` заполнен там, где адрес
клиентский, и пароль из userinfo в нём замаскирован.

## Наблюдение MCP

При ожидании MCP готовность подтверждает только свежий успешный `tools/list` с нужными инструментами. После неудачного ожидания `tools` и `missing_tools` описывают последний успешный ответ в этом ожидании: ошибки повторного `initialize` или `tools/list` сохраняют это диагностическое наблюдение и сообщение о недостающих инструментах. Новый успешный список заменяет предыдущий. Сохранённый список не доказывает текущую доступность endpoint и не даёт `ok: true`; отмена сохраняет собственный исход и сообщение. Если успешного списка ещё не было, сообщается ошибка доступности endpoint.

## Пример

```json
{
  "ok": true,
  "mode": "designer",
  "pid": null,
  "binary": "/opt/1cv8/bin/1cv8",
  "via": "connection",
  "platform_resolution": {
    "path": "/opt/1cv8/bin/1cv8",
    "version": null,
    "source": "explicit",
    "installation_root": "/opt/1cv8"
  },
  "provider_dispatched": false,
  "plan": {
    "program": "/opt/1cv8/bin/1cv8",
    "args": [
      "DESIGNER",
      "/DisableStartupDialogs",
      "/IBConnectionString",
      "File=build/ib"
    ]
  },
  "message": "Previewed конфигуратор via /opt/1cv8/bin/1cv8; client process not dispatched"
}
```
