---
id: INV.USE-CASES.IBCMD-EXPORTS-A-PACKAGE-IN-A-THROWAWAY-BASE
check:
  - tests/cli_convert.rs::convert_a_package_file_to_xml_exports_it_in_a_throwaway_base
  - src/platform/ibcmd.rs::config_export_file_uses_config_mode_and_the_target_context
  - src/platform/ibcmd.rs::only_the_package_export_is_single_threaded
---

# `convert` пакета в XML через `ibcmd` идёт во временной базе раннера

Файл пакета `.cf` или `.cfe` `ibcmd` разбирает в XML вызовом `config export --threads=1 --file=<пакет>
<каталог>` — тем же, которым `extensions` читает применённое расширение. Вызов несёт
подключение к базе, и это временная база раннера под `workPath` со своим каталогом данных
`--data`, созданная `infobase create`; база проекта не выбирается и не открывается. XML
ложится в промежуточный каталог рядом с целью и публикуется заменой каталога, база
убирается после прогона. Живой замер этой последовательности —
`INV.USE-CASES.CONVERT-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM`.

Разбор пакета идёт в один поток (`--threads=1`): в несколько потоков он отказывает на годных
пакетах (#476). Выгрузка из базы этого ключа не несёт
(`INV.PLATFORM.AN-IBCMD-EXPORT-FROM-A-BASE-KEEPS-PLATFORM-THREADS`).
