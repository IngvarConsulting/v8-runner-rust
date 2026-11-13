# Выпуски и проверка

Что публикует каждый выпуск `v8-runner`, как проверить его происхождение и как менялись имена
файлов выпуска.

## Архивы

Каждый выпуск публикует **один архив на платформу**. Внутри — бинарник `v8-runner`
(на Windows `v8-runner.exe`), корневой `README.md`, лицензия, уведомление о форке и каталог
`examples/`:

| Архив | Система | Процессор |
| --- | --- | --- |
| `v8-runner-linux-x86_64-musl.tar.gz` | Linux, любой дистрибутив (статическая сборка musl) | Intel/AMD 64 |
| `v8-runner-macos-aarch64.tar.gz` | macOS | Apple Silicon (M1 и новее) |
| `v8-runner-macos-x86_64.tar.gz` | macOS | Intel |
| `v8-runner-windows-x86_64.zip` | Windows | Intel/AMD 64 |

Отличить Apple Silicon от Intel: `uname -m` отвечает `arm64` или `x86_64`.

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

## История имён файлов выпуска

- До `v0.11.4` включительно рядом с архивами выкладывались несжатые бинарники
  `v8-runner-darwin-arm64`, `v8-runner-linux-x64` и `v8-runner-win-x64.exe`. Это были байт в
  байт те же файлы, что лежат в архивах, и одна платформа выходила под двумя именами. Начиная
  с `v0.12.0` они не публикуются.
- В `v0.6.x` публиковались отдельные `.sha256` и `.provenance.json`. С `v0.7.0` их заменяет
  единый манифест; имена архивов и юридических файлов при этом не менялись.
