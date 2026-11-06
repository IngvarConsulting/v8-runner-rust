---
id: INV.PLATFORM.A-SERVICE-STOP-IS-CONFIRMED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Остановка службы подтверждается концом процесса

Успех `taskkill` без `/F` ничего не доказывает: раннер ждёт конца процесса и только затем снимает его принудительно.
