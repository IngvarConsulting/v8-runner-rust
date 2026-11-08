---
id: INV.PLATFORM.RAS-STOPS-APART-FROM-RAGENT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/538
---

# `ras` останавливается отдельно от `ragent`

`ras` переживает конец `ragent`, поэтому у него своя запись, и `cluster stop` останавливает его отдельным шагом.
