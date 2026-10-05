---
id: INV.USE-CASES.VANESSA-DEFAULTS-TO-THE-GITHUB-LATEST-RELEASE
check:
  - tests/cli_tools_download.rs::tools_download_vanessa_takes_the_latest_release_without_prerelease_flag
---

# Vanessa по умолчанию берёт latest-выпуск GitHub

Без ключа `--prerelease` команда `tools download vanessa` берёт выпуск, который GitHub
отдаёт как `releases/latest`; pre-release туда не попадает, даже если его версия
больше (#160).
