---
id: INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Службы стенда раннер запускает только на Windows

На Linux и macOS `dbms start`, `cluster start`, `standalone start` и их `restart` отвечают
`environment_unavailable`, не запуская процессов: поведение служб замерено только на Windows.
