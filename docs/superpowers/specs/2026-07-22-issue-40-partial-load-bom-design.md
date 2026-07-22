# Issue #40: UTF-8 BOM для Designer partial-load listFile

## Контекст

При частичной загрузке Designer команда `v8-runner build` передаёт платформе 1С файл через `/LoadConfigFromFiles -partial -listFile`. Сейчас `partial_load::write_list_file` формирует безопасные относительные пути, соединяет их через `CRLF` и записывает UTF-8-строку без BOM. В проверенном окружении платформа 1С отклоняет такой файл, но принимает тот же payload с UTF-8 BOM `EF BB BF`.

## Цель

Гарантировать, что каждый Designer partial-load `listFile` начинается ровно с одного UTF-8 BOM, а остальной контракт остаётся неизменным:

- пути относительны `source_root` и проходят существующие проверки безопасности;
- имена с кириллицей кодируются в UTF-8;
- записи разделены `CRLF`;
- завершающий `CRLF` не добавляется;
- пустой payload представлен файлом, содержащим только BOM.

## Неграницы

- Не менять выбор между partial и full load.
- Не менять расширение BSL-путей связанными XML-файлами.
- Не менять Designer DSL и состав аргументов `/LoadConfigFromFiles`.
- Не менять IBCMD и partial-dump list files: issue относится только к Designer partial load.
- Не исправлять обнаруженную baseline-ошибку компиляции test-binary на Windows в Unix-ориентированных helpers `check_syntax`.
- Не вводить новый общий сериализатор list-файлов или новую зависимость.

## Решение

Точка сериализации остаётся в `src/change_detection/partial_load.rs::write_list_file`.

1. Получить относительные пути через существующий `relative_paths`.
2. Преобразовать пути в строки тем же способом, который используется сейчас.
3. Соединить строки через `\r\n`, не добавляя разделитель после последней записи.
4. Создать единый byte payload: `EF BB BF` и затем UTF-8-байты соединённой строки.
5. Один раз передать payload в `std::fs::write`.

Так BOM не может появиться перед каждой строкой или быть записан дважды. Ошибки преобразования безопасных путей и ошибки файловой системы продолжают возвращаться через существующий `std::io::Result` и обрабатываются текущим механизмом сохранения diagnostic list file.

## Проверки

### Byte-level unit regression

В модуле тестов `src/change_detection/partial_load.rs` создать два существующих файла под временным `source_root`, включая путь с кириллицей. После вызова `write_list_file` прочитать результат через `std::fs::read` и сравнить весь массив байтов с точным ожидаемым значением:

```text
EF BB BF + UTF8(relative_path_1) + 0D 0A + UTF8(relative_path_2)
```

Проверка полного массива одновременно фиксирует единственный BOM, UTF-8, относительность, `CRLF` и отсутствие завершающего `CRLF`. Тест пустого относительного списка обновляется: ожидается BOM-only файл. Существующие проверки выхода за `source_root` остаются без изменений.

### Trusted live Designer regression

В `scripts/test/live-cli-fixture.sh` после первоначального full build и incremental no-op изменить скопированный fixture-файл `CommonModules/ОбщийМодуль1/Ext/Module.bsl`, добавив корректный комментарий BSL. Затем выполнить JSON build только для configuration source-set и проверить:

- команда завершилась успешно;
- соответствующий step успешен;
- mode шага равен partial и содержит положительный `file_count`.

Сценарий запускается существующим trusted happy-path CI на Ubuntu и Windows, где доступен настоящий Designer. Fork и Dependabot без секретов сохраняют текущий soft-skip live-пути.

## Документация

Rustdoc `write_list_file` обновляется и явно фиксирует UTF-8 BOM и `CRLF`. `SKILL/SKILL.md`, ADR и пользовательские команды не меняются, потому что исправление не меняет публичный workflow или конфигурационный контракт.

## Критерии готовности

- `write_list_file` записывает ровно один BOM `EF BB BF` перед содержимым.
- Точный unit-тест покрывает BOM, UTF-8 кириллицу, относительные пути и `CRLF`.
- Пустой payload имеет определённое представление BOM-only.
- Существующие safe-relative-path тесты продолжают проходить в поддерживаемом test-контуре.
- Trusted live Designer выполняет настоящий partial build после изменения BSL-файла.
- Изменение не затрагивает IBCMD, partial dump и выбор partial/full стратегии.

## Известное baseline-ограничение

До изменений `cargo test` на текущем Windows workspace не компилирует общий test-binary из-за `std::os::unix` и `Permissions::set_mode` в тестовом коде `src/use_cases/check_syntax.rs`. Это не связано с issue #40. Проверка исправления должна использовать доступные targeted/CI-контуры, а это ограничение должно быть явно указано в итогах работы.
