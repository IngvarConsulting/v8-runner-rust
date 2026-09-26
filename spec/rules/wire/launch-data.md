---
id: CTR.WIRE.LAUNCH-DATA
version: 4
artifact: docs/schemas/command-data/launch.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
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
