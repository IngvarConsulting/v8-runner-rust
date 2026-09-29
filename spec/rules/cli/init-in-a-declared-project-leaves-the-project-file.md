---
id: INV.CLI.INIT-IN-A-DECLARED-PROJECT-LEAVES-THE-PROJECT-FILE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/329
---

# `init` в готовом проекте не трогает проектный файл

В проекте, где `v8project.yaml` уже есть, `init` без `--force` его не трогает. Без
`--infobase` он объявляет `origin` базой `File=build/ib`, если `origin` ещё нет. С
`--infobase <адрес>` он отдаёт `origin` новый адрес, а прежнюю секцию сохраняет под именем
`upstream` вместе с учётными данными; ответ называет заменённый адрес. Адрес, который уже
стоит в `origin`, ничего не меняет. `--force` переписывает только проектный файл: с местным
слоем `init` поступает так же, как без него.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
