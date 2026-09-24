---
id: INV.CONFIG.A-KEY-SYNONYM-IS-MARKED-DEPRECATED-IN-THE-SCHEMA
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/191
---

# Прежний ключ конфигурации помечен в схеме устаревшим

Каждый ключ, который модель принимает как синоним, присутствует в опубликованной схеме с
`deprecated: true`; ключ, принимаемый моделью и отсутствующий в схеме, — нарушение.
