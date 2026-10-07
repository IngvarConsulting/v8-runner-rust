---
id: INV.USE-CASES.A-DOWNLOAD-THROUGH-THE-GATE-EXPORTS-THE-MAIN-CONFIGURATION
check: [tests/cli_agent_standalone.rs::a_download_through_the_gate_exports_the_main_configuration]
---

# Через SSH-шлюз `download` отдаёт основную конфигурацию

Когда `download` без расширения на автономном сервере исполняет агентский shell его
SSH-шлюза — строки прямого шлюза нет или Конфигуратор не готов, — раннер шлёт шлюзу `config dump-cfg --file=<путь на стороне цели>` без `--extension` и
забирает пакет объявленным каналом обмена. Ответ называет предметом основную конфигурацию
(`subject.kind: main`) в рабочем состоянии, а опубликованный `.cf` — то, что выгрузил шлюз.
