---
id: INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/577
---

# Живого агента берут только по его ключу

Ключ живого агента состоит из хеша строки подключения базы, версии платформы, пользователя
базы, `workPath`, версии раннера и учётной записи ОС. Команда берёт живого агента, только
если совпали все части ключа. В открытом виде ключ не хранится: в строке подключения может
быть пароль.

Поскольку `workPath` входит в ключ, у каждой рабочей копии свой агент. Его `AgentBaseDir`,
текущий каталог и ключ хоста лежат в её `workPath`
(`INV.PLATFORM.A-MANAGED-AGENT-RUNS-IN-ITS-OWN-DIRECTORY`). Команду и агента связывает
блокировка рабочего каталога, под которой команда держит агента
(`INV.PLATFORM.AGENT-SESSION-LIVES-WITH-THE-LOCK`).
