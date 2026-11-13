# Выпуски и проверка

Что публикует каждый выпуск `v8-runner`, как проверить его происхождение и как менялись имена
файлов выпуска.

## Изменения контракта

### 0.17.2

- Предупреждение о записи в базу другой рабочей копии получило машинный код: конверт несёт
  рядом с прежними строками `warnings` новое необязательное поле `warning_details` с
  `code: infobase_of_another_copy`, именем и адресом базы и живыми копиями-владельцами — у
  прогона и у `--dry-run` командной строки и у инструмента MCP (#460). Конверт —
  `CTR.WIRE.COMMAND-ENVELOPE` версии 6, протокол внешней выгрузки —
  `CTR.WIRE.FULL-PULL-PUBLICATION` версии 4; прежние поля не изменились.
- `status` без `--deep` называет у файловой базы `holders` — живых владельцев из метки, без
  запуска платформы; ушедшие копии в список не входят ни с `--deep`, ни без него. У записи
  владельца новое поле `this_machine` (`CTR.WIRE.STATUS-DATA` версии 7) (#460).
- `init` работает с проектом каталога запуска: `--output` вне него — отказ `invalid_argument`
  до записи чего-либо, в объявленном проекте и в новом (#718). Раньше запуск из другого каталога
  успевал перенаправить `origin` в местном слое проекта и отказывал на `.gitignore` каталога запуска.
  Отказ записи `init` теперь возвращает уже записанным файлам прежнее содержимое.

## Архивы

Каждый выпуск публикует **один архив на платформу**. Внутри — бинарник `v8-runner`
(на Windows `v8-runner.exe`), корневой `README.md`, лицензия, уведомление о форке и каталог
`examples/`:

| Архив | Система | Процессор |
| --- | --- | --- |
| `v8-runner-linux-x86_64-musl.tar.gz` | Linux, любой дистрибутив (статическая сборка musl) | Intel/AMD 64 |
| `v8-runner-macos-aarch64.tar.gz` | macOS | Apple Silicon (M1 и новее) |
| `v8-runner-windows-x86_64.zip` | Windows | Intel/AMD 64 |

Mac на Intel не поддерживается с 0.17.2: архива `v8-runner-macos-x86_64.tar.gz` больше нет, соберите
из исходников `cargo build --release`. Отличить Apple Silicon от Intel: `uname -m` отвечает
`arm64` или `x86_64`.

## Манифест и аттестации

Единый `v8-runner-assets.json` schema v2 связывает исходные tag/commit с ролью, target,
размером и SHA-256 всех остальных assets, а для архивов — также с путём и SHA-256 вложенного
бинарника. Все payload assets и манифест имеют GitHub build attestations, которые
подтверждают их происхождение; `license-v8-runner-AGPL-3.0-only.txt` и
`notice-v8-runner-fork.txt` лежат рядом и также входят в манифест. Corresponding Source —
неизменяемый tag того же выпуска.

## Проверка перед использованием

Проверьте выпуск и конкретный бинарник:

```bash
gh release verify v0.7.0 --repo IngvarConsulting/v8-runner-rust
gh release download v0.7.0 --repo IngvarConsulting/v8-runner-rust \
  --pattern v8-runner-assets.json --pattern v8-runner-linux-x86_64-musl.tar.gz
gh release verify-asset v0.7.0 ./v8-runner-assets.json \
  --repo IngvarConsulting/v8-runner-rust
gh release verify-asset v0.7.0 ./v8-runner-linux-x86_64-musl.tar.gz \
  --repo IngvarConsulting/v8-runner-rust
source_commit="$(python3 -c 'import json; print(json.load(open("v8-runner-assets.json"))["release"]["sourceCommit"])')"
for asset in v8-runner-assets.json v8-runner-linux-x86_64-musl.tar.gz; do
  gh attestation verify "$asset" \
    --repo IngvarConsulting/v8-runner-rust \
    --signer-workflow IngvarConsulting/v8-runner-rust/.github/workflows/release.yml \
    --source-ref refs/heads/master --source-digest "$source_commit" \
    --deny-self-hosted-runners
done
```

Для offline-проверки сначала проверьте и перенесите в изолированную среду
`v8-runner-assets.json`, затем сравните SHA-256 нужного файла с соответствующей записью
манифеста. Манифест, скачанный вместе с бинарником без предварительной проверки
`gh release verify-asset`, сам по себе не является корнем доверия.

## Изменения выпусков

### 0.17.2

- Windows: `CTRL_BREAK`, посланный группе раннера (`GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT,
  pid раннера)`), больше не обрывает его консольных потомков (`ibcmd`, `1cedtcli`, `rac`, `git`)
  мимо отмены раннера: они запускаются в своих группах процессов, и снимает их раннер, а
  критическая фаза записи базы доводится до конца (#717).
- Отказ `environment_unavailable` несёт поле `error.layer` — слой окружения, который отказал:
  `infobase_user`, `agent_endpoint`, `agent_start` или `platform`. Поле есть у отказов пути
  агента и у отказа из-за отсутствующей или не той утилиты; без поля слой не определён. Код,
  род и тексты отказов прежние, так что опознание по тексту у прежних выпусков продолжает
  работать, а с этого выпуска его можно заменить чтением поля. У MCP то же поле приходит в
  отказе и в конверте ответа, при коде `runtime_failure`. Конверт — версии 6
  ([правило](../spec/rules/wire/an-environment-refusal-names-its-layer.md), #485).

## История имён файлов выпуска

- С `v0.17.2` архив `v8-runner-macos-x86_64.tar.gz` (macOS на Intel) не собирается: машин
  GitHub с Intel macOS мало, и выпуск ждал их дольше всех остальных шагов. Для Mac на Intel —
  сборка из исходников `cargo build --release`.
- До `v0.11.4` включительно рядом с архивами выкладывались несжатые бинарники
  `v8-runner-darwin-arm64`, `v8-runner-linux-x64` и `v8-runner-win-x64.exe`. Это были байт в
  байт те же файлы, что лежат в архивах, и одна платформа выходила под двумя именами. Начиная
  с `v0.12.0` они не публикуются.
- В `v0.6.x` публиковались отдельные `.sha256` и `.provenance.json`. С `v0.7.0` их заменяет
  единый манифест; имена архивов и юридических файлов при этом не менялись.
