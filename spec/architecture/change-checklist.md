# Checklist архитектурных изменений

Этот checklist нужен для задач, которые меняют публичный контракт или архитектурные границы `v8-runner`.
Он не заменяет ADR и [архитектурные инварианты](invariants.md), а помогает не забыть обязательную синхронизацию перед merge.

## Изменение MCP public surface

1. Подтвердить, что изменение разрешено действующим решением реестра `spec/arch`; если нет, сначала завести решение и поднять версию `CTR.MCP.PUBLISHED-TOOL-SURFACE`.
2. Синхронизировать список tools и их публичную семантику минимум в:
   - `spec/arch/contracts/CTR.MCP.PUBLISHED-TOOL-SURFACE.md`
   - `spec/arch/README.md`
   - `ARCHITECTURE.md`
   - `README.md`
   - `docs/CAPABILITIES.md`
   - `src/mcp/server.rs`
   - `src/mcp/request.rs`
   - `src/mcp/service.rs`
   - `src/command_envelope.rs`, если меняется machine-readable command payload
3. Добавить или обновить tests для `list_tools`, request DTO, shared envelope payload и business/runtime failure mapping по `DEC.2026-04-20.BUSINESS-FAILURES-ARE-NOT-TRANSPORT-FAULTS`.
4. Явно проверить, что изменение не публикует CLI-only сценарий как MCP tool по умолчанию.

## Новая public CLI/MCP команда, работающая с `workPath`

1. Брать workspace lock на adapter boundary: `src/cli/execute.rs` для CLI или `src/mcp/port.rs` для MCP.
2. Nested orchestration оставлять на explicit unlocked entrypoints только под уже взятым внешним lock.
3. Не считать execution admission, semaphore или HTTP session capacity заменой workspace lock.
4. Добавить regression coverage минимум на:
   - busy workspace conflict;
   - корректный boundary до dispatch в use case;
   - validation-before-lock, если команда имеет раннюю валидацию аргументов.

## Новый public config field, `source-set` type или `infobase` subtree

1. Добавить typed field и нужные `serde` defaults/renames в `src/config/model.rs`.
2. Добавить validation boundary в `src/config/validate.rs`, чтобы unsafe/unsupported combinations отклонялись до platform DSL.
3. Обновить `config init`, round-trip fixtures и публичные примеры (`README.md`, `examples/*`), если поле входит в supported contract.
4. Синхронизировать `spec/arch/README.md`, `ARCHITECTURE.md` и соответствующий ADR, если поле меняет публичный контракт.
5. Добавить regression tests на parse/validation/round-trip и на целевое поведение для новых `source-set`/`infobase` веток.
6. Адрес в поле (`host[:port]`, значение заголовка `Host`, URL) читает только `support::authority`; своё правило поля (например, обязательный порт) добавляется к его ответу, а не к строке. Страж — `tests/architecture_guardrails.rs::a_host_port_record_is_read_in_one_place`.

## Изменение формы ответа команды

1. Найти запись формы: `docs/schemas/command-data/index.json` называет формы каждой
   команды, каждой соответствует контракт `spec/arch/contracts/CTR.WIRE.*-DATA.md`.
2. Править типизированную модель, а не файл схемы: артефакт порождается командой
   `UPDATE_COMMAND_DATA_SCHEMAS=1 cargo test generated_command_data_schemas_are_current`.
3. Поднять `version` контракта и обновить его раздел «Пример»: пример проверяется
   против той же схемы и устаревшим не останется.
4. Новая команда или новая форма существующей — новая запись в таблице
   `src/command_data.rs` и новый контракт; без них форма не опубликована.
5. Проверить `cargo test --test contract_command_data`: живой ответ сверяется с формой,
   и добавленное поле валит проверку так же, как убранное.

## Новая проверка результата внешнего инструмента

1. Назвать структурный ответ, на котором принимается решение: код выхода, обещанный
   артефакт или документированный машинный вывод. Формулировка вывода входом решения
   быть не может — см. [`DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES`](../arch/decisions/2026-09-12-tool-prose-never-decides.md).
2. Если структурного ответа нет, брать факт у вызывающего и называть неизвестность
   отдельным значением, а не выводить её из текста.
3. Прозу инструмента доносить до вызывающего как улику, не интерпретируя.
4. Снять замер на настоящем инструменте и записать его версию рядом с правилом;
   поддельный инструмент в тестах обязан повторять замер, а не собственную выдумку.
5. Проверить `cargo test --test tool_output_contract`: новое решение на тексте валит
   стража, а реестр `PROSE_DEBT` может только сокращаться.
