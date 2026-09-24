---
id: INV.WIRE.A-SESSION-RECEIPT-NAMES-ITS-ENDPOINT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/284
---

# Квитанция сессии называет точку входа

Там, где операция шла через сессию агента, квитанция исполнителя несёт `provider.endpoint`
с режимом — `managed` или `attached` — и адресом. Сегодня у квитанции только
`selected`, `origin` и `skipped`.
