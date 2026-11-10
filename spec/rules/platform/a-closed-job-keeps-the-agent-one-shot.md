---
id: INV.PLATFORM.A-CLOSED-JOB-KEEPS-THE-AGENT-ONE-SHOT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/589
---

# В Job без права выхода агент остаётся разовым

Бывает, что раннер запущен в Job с kill-on-close без права выхода (без
`JOB_OBJECT_LIMIT_BREAKAWAY_OK`). Сторож тогда не может выйти из Job
(`INV.PLATFORM.A-SERVICE-OUTLIVES-THE-RUNNER`), а закрытие Job сняло бы и агента. В таком Job
раннер ведёт себя так:

- обычная команда не отказывает, а работает с разовым агентом: в квитанции `kept: false` и
  `reason: job` (`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`);
- `agent hold` отказывает с кодом `live_agent_job` и платформу не запускает.

Под Unix ограничения нет: сторож выходит из группы процессов и из сеанса того, кто его
вызвал (`setsid`), так что отмена всей группы вызвавшего его не задевает.
