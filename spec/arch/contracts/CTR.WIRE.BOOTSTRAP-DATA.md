---
id: CTR.WIRE.BOOTSTRAP-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/clone.schema.json
producer: src/domain/bootstrap.rs
consumers: [cli]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/cli_bootstrap.rs::bootstrap_json_success_keeps_credentials_in_local_overlay_only]
scope: [wire, cli]
---

# `data` команды `bootstrap`

Команда заводит проект вокруг существующей базы, и форма называет каждый созданный ею
путь: конфиг, локальный слой, `.gitignore` и каталог исходников. Флаг `dumped` отделяет
заведённый проект от заведённого и сразу выгруженного — второе занимает минуты и делается
не всегда.

Живой проверки у формы пока нет: команде нужна настоящая база, а харнесс подставляет
вместо платформы скрипты. Форму держит сверка с типом, который её сериализует.

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
  "duration_ms": 214536,
  "warnings": []
}
```
