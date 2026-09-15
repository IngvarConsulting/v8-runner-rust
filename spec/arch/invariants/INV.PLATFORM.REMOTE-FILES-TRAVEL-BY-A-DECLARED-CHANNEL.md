---
id: INV.PLATFORM.REMOTE-FILES-TRAVEL-BY-A-DECLARED-CHANNEL
status: active
governs: product
decision: DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE
check: tests/cli_agent_standalone.rs::a_standalone_server_without_a_declared_channel_is_refused_before_any_session
scope: [platform]
---

# Файлы удалённой цели забираются объявленным каналом

Результат, оставшийся на стороне удалённой цели, переносится объявленным каналом обмена. Чтение чужого пути так, будто файловая система общая, запрещено.
