---
id: CTR.WIRE.BOOTSTRAP-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/clone.schema.json
producer: src/domain/bootstrap.rs
consumers: [cli]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/cli_bootstrap.rs::bootstrap_json_success_keeps_credentials_in_local_overlay_only, tests/cli_bootstrap.rs::clone_preview_names_the_project_it_would_write_and_writes_nothing]
scope: [wire, cli]
---

# `data` команды `bootstrap`

Команда заводит проект вокруг существующей базы, и форма называет каждый созданный ею
путь: конфиг, локальный слой, `.gitignore` и каталог исходников. Флаг `dumped` отделяет
заведённый проект от заведённого и сразу выгруженного — второе занимает минуты и делается
не всегда.

`provider_dispatched` отвечает на другой вопрос: запускалась ли платформа. `false` приходит
и под превью, и при отказе раньше выбора исполнителя. Поле есть всегда, поэтому отсутствие
запуска не выводится из отсутствия значения. Два флага не сводятся к одному: упавшая
выгрузка отвечает `dumped: false` при `provider_dispatched: true`, а превью — обоими `false`.

Под превью пути называют то, что **было бы** записано: ни одного из четырёх на диске нет.
Проверки настроек при этом те же, что у боевого прогона, и утилита выгрузки найдена —
отсутствие платформы отказывает до одобрения плана.

Квитанции выбора исполнителя форма не несёт: `clone` в семью
`DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE` не входит ни боевым прогоном, ни превью.
Найденную утилиту превью называет словами в `message`.

## Пример

```json
{
  "ok": true,
  "path": "v8project.yaml",
  "local_path": "v8project.local.yaml",
  "gitignore_path": ".gitignore",
  "source_dir": "src/cf",
  "dump_target_path": "src/cf",
  "dumped": true,
  "provider_dispatched": true,
  "duration_ms": 214536
}
```

Превью той же команды:

```json
{
  "ok": true,
  "path": "v8project.yaml",
  "local_path": "v8project.local.yaml",
  "gitignore_path": ".gitignore",
  "source_dir": "src/cf",
  "dump_target_path": "src/cf",
  "dumped": false,
  "provider_dispatched": false,
  "duration_ms": 5,
  "message": "would dump Full into 'src/cf' via /opt/1cv8/x86_64/8.3.27.1000/1cv8; nothing written"
}
```
