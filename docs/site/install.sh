#!/bin/sh
# Portable release installer. Installs only into the current user's directories.
set -eu
fail() { printf '%s\n' "$*" >&2; exit 1; }
for tool in curl tar awk sed install mktemp grep mv; do command -v "$tool" >/dev/null 2>&1 || fail "Нужна команда: $tool"; done
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) package=v8-runner-macos-aarch64 ;;
  Darwin:x86_64) package=v8-runner-macos-x86_64 ;;
  Linux:x86_64) package=v8-runner-linux-x86_64-musl ;;
  *) fail 'Поддерживаются macOS Apple Silicon/Intel и Linux x86_64.' ;;
esac
if command -v sha256sum >/dev/null 2>&1; then
  digest() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
  digest() { shasum -a 256 "$1" | awk '{print $1}'; }
else fail 'Нужна sha256sum или shasum.'; fi
tmp=$(mktemp -d)
stage=''
trap 'rm -rf "$tmp"; [ -z "$stage" ] || rm -f "$stage"' 0
trap 'exit 1' HUP INT TERM
base=https://github.com/IngvarConsulting/v8-runner-rust/releases
curl -fsSL "$base/latest/download/v8-runner-assets.json" -o "$tmp/manifest.json"
tag=$(sed -n 's/^[[:space:]]*"sourceTag": "\([A-Za-z0-9._-]*\)".*/\1/p' "$tmp/manifest.json")
case "$tag" in v*) ;; *) fail 'Некорректный тег выпуска.' ;; esac
[ -n "$tag" ] || fail 'Не удалось определить выпуск.'
grep -q '"schemaVersion": 2,' "$tmp/manifest.json" || fail 'Неизвестная схема манифеста.'
[ "$(grep -c '"name": "'"$package"'.tar.gz"' "$tmp/manifest.json")" -eq 1 ] || fail 'Нужна ровно одна запись архива.'
# The release generator owns this pretty-printed schema v2. A changed format fails closed.
checksum=$(awk -v name="$package.tar.gz" '
  $0 ~ /"name":/ { found = index($0, "\"" name "\"") > 0 }
  found && /"sha256":/ { split($0,a,"\""); print a[4]; exit }
' "$tmp/manifest.json")
[ ${#checksum} -eq 64 ] || fail 'В манифесте нет SHA-256 архива.'
case "$checksum" in *[!a-f0-9]*) fail 'Некорректная SHA-256.' ;; esac
curl -fsSL "$base/download/$tag/$package.tar.gz" -o "$tmp/archive.tar.gz"
[ "$(digest "$tmp/archive.tar.gz")" = "$checksum" ] || fail 'SHA-256 архива не совпадает. Установка остановлена.'
tar -xzf "$tmp/archive.tar.gz" -C "$tmp" "$package/v8-runner"
binary_checksum=$(awk -v name="$package.tar.gz" '
  /"name":/ { found = index($0, "\"" name "\"") > 0 }
  found && /"sha256":/ { n++; if (n == 2) { split($0,a,"\""); print a[4]; exit } }
' "$tmp/manifest.json")
[ ${#binary_checksum} -eq 64 ] || fail 'Нет SHA-256 бинарника.'
[ "$(digest "$tmp/$package/v8-runner")" = "$binary_checksum" ] || fail 'SHA-256 бинарника не совпадает.'
"$tmp/$package/v8-runner" --version
bin=${V8_RUNNER_INSTALL_DIR:-"$HOME/.local/bin"}
mkdir -p "$bin"
stage=$(mktemp "$bin/.v8-runner.XXXXXX")
install -m 755 "$tmp/$package/v8-runner" "$stage"
mv -f "$stage" "$bin/v8-runner"
stage=''
# Do not rewrite a custom installation path into the shell profile.
if [ "$bin" = "$HOME/.local/bin" ]; then
  user_shell=${SHELL:-}
  case "${user_shell##*/}" in
    zsh) profile="$HOME/.zshrc" ;;
    bash) profile="$HOME/.bashrc"; [ "$(uname -s)" != Darwin ] || profile="$HOME/.bash_profile" ;;
    *) profile=''; printf 'Добавьте %s в PATH вашей оболочки.\n' "$bin" ;;
  esac
  if [ -n "$profile" ]; then
    line='export PATH="$HOME/.local/bin:$PATH"'
    grep -qxF "$line" "$profile" 2>/dev/null || printf '\n%s\n' "$line" >> "$profile"
  fi
fi
"$bin/v8-runner" --version
printf 'Установлено в %s. Откройте новый терминал.\n' "$bin"
