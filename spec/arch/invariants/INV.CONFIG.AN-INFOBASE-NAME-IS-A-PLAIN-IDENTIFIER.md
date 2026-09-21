---
id: INV.CONFIG.AN-INFOBASE-NAME-IS-A-PLAIN-IDENTIFIER
status: active
governs: product
decision: DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT
check: [tests/cli_infobases.rs::an_infobase_name_is_a_plain_identifier, src/config/schema.rs::the_infobase_synonym_is_deprecated_and_the_map_keys_are_identifiers]
scope: [config]
---

# Имя базы — простой идентификатор

Ключ карты `infobases`, не подходящий под `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`, отвергается
валидацией с указанием ключа; каталог памяти строится из имени без преобразований,
поэтому вывести его за пределы `workPath/infobases/` нельзя.
