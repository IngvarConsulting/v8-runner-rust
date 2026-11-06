---
id: INV.CLI.A-SERVICE-STATUS-HAS-NO-PREVIEW
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# У `status` служб нет превью

`status` у `dbms`, `cluster` и `standalone` ничего не запускает, поэтому `--dry-run` у него отвергается.
