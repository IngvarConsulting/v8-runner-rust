---
id: CTR.USE-CASES.INFOBASE-OWNER-MARKER
version: 1
artifact: docs/schemas/infobase-owner-marker.schema.json
check:
  - src/use_cases/infobase_owner.rs::generated_owner_marker_schema_is_current
  - src/use_cases/infobase_owner.rs::a_written_marker_passes_its_schema
---

# Форма метки владельца файловой базы

Метка владельца — файл `.<имя каталога базы>.v8-runner.owners.json` рядом с каталогом
файловой базы. Её форма закреплена схемой `docs/schemas/infobase-owner-marker.schema.json`:
`version` — номер формы, `owners` — копии-владельцы. У каждой копии `machine` —
идентификатор машины, который переживает смену имени хоста, `host` — имя хоста на момент
записи для людей, `project` — канонический каталог проекта, `shared` — согласие делить базу,
`since` — когда копия записана. Набор полей закрыт: метку с незнакомым полем раннер не
понимает и не переписывает.

Схема порождается из типов: `UPDATE_OWNER_MARKER_SCHEMA=1 cargo test
generated_owner_marker_schema_is_current`. Метку другой версии раннер не переписывает
(`INV.USE-CASES.A-MARKER-OF-AN-UNKNOWN-VERSION-IS-NOT-REWRITTEN`), поэтому новая версия формы —
событие для всех рабочих копий базы, а не только для одной.

## Пример

```json
{
  "version": 1,
  "owners": [
    {
      "machine": "9f1c0d4be2a37f6815c9e0b4a7d2f3e61b8c5a9047de2f6c3b1a8e5d7c4f0a92",
      "host": "dev-laptop",
      "project": "/work/erp",
      "shared": false,
      "since": "2026-10-06T12:00:00Z"
    }
  ]
}
```
