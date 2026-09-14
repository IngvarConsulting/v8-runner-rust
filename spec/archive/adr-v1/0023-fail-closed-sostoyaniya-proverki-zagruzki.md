# ADR-0023: Fail-closed состояния проверки загрузки артефактов

- Статус: `accepted`
- Дата: `2026-09-02`
- Связанные решения: [ADR-0008](0008-derzhat-platformennye-backend-dsl-otdelno-ot-orchestration.md), [ADR-0009](0009-razdelit-business-i-transport-runtime-failures.md), [ADR-0010](0010-razdelit-cli-output-dlya-cheloveka-i-ai-agenta.md), [ADR-0016](0016-edinyy-executionoutcome-i-pipeline-steps-dlya-runner-like-stsenariev.md), [ADR-0022](0022-universalnyy-mehanizm-podgotovki-rasshireniy-i-client-mcp-extension.md)

## Контекст

Перед `load` и `merge` runner проверяет совместимость артефакта через Designer.
Для первой загрузки отсутствующего расширения платформа 8.3.27.2130 возвращает
ненулевой код и точную строку `/Out`:

```text
Конфигурация 'Расширение конфигурации' недоступна
```

Прежняя классификация склеивала stdout, stderr и `/Out`, затем искала отдельные
слова. Такое правило неоднозначно: сообщение об отсутствующем расширении и
несвязанная ошибка доступа могли вместе разрешить изменяющий `/LoadCfg`.

## Решение

1. `CompatibilityState` имеет четыре публичных JSON-значения:
   `supported`, `not_supported`, `absent`, `unknown`.
2. `absent` означает доказанное отсутствие target в ИБ, а не несовместимость.
   `load + absent` разрешает первую установку; `merge + absent` отклоняется с
   рекомендацией сначала выполнить `load`.
3. Ненулевой probe классифицируется положительно только по закрытому allowlist
   целых чистых диагностик. Подстроки из разных каналов не комбинируются.
4. Новая ветка `absent` принимается только для зафиксированной русской строки в
   текущем `/Out`, при пустых stdout/stderr, без interruption, ошибки чтения и
   дополнительных строк. Неподтверждённый перевод не добавляется.
5. Любой иной ненулевой результат остаётся `unknown`, возвращается как platform
   failure по ADR-0009 и не разрешает `/LoadCfg` или `/MergeCfg`.
6. Матрица `(mode, compatibility state)` перечисляется исчерпывающе; новый
   вариант enum не может получить разрешение через default arm.
7. Предыдущий `/Out` удаляется до запуска Designer. Любая ошибка удаления,
   кроме `NotFound`, останавливает команду до spawn.

## Совместимость

`absent` расширяет machine-readable контракт в функциональном релизе `v0.6.0`.
Потребители, которые исчерпывающе разбирают `compatibility_state`, должны
добавить новый вариант. Сводить `absent` к `not_supported` нельзя: это возвращает
неоднозначность для AI-агента и скрывает правильный следующий шаг.

## Reintroduction guard

- Root cause: положительная классификация по независимым подстрокам из разных
  каналов и повторное использование недоказанного `/Out`.
- Single owner: `classify_probe_failure` и helpers рядом с ним в
  `src/use_cases/load_artifact.rs`; очисткой `/Out` владеет `DesignerDsl::run`.
- Detection: cross-platform unit-тесты проверяют закрытый allowlist, BOM/CRLF и
  противоречивые diagnostics; execute/CLI tests проверяют отсутствие apply при
  mixed output, read error и stale log, а также порядок probe -> load -> update.

## Последствия

1. Неизвестная локаль или новая формулировка платформы сначала безопасно
   блокирует изменение ИБ.
2. Новый вариант добавляется только после wire capture реального вывода и
   отдельного regression-теста.
3. `/DumpDBCfgList` может заменить текстовый probe после фиксации его формата на
   поддерживаемых версиях платформы, но само наличие команды этого не доказывает.

