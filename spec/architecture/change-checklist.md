# Checklist архитектурных изменений

Этот checklist нужен для задач, которые меняют публичный контракт или архитектурные границы `v8-runner`.
Он не заменяет ADR и [архитектурные инварианты](invariants.md), а помогает не забыть обязательную синхронизацию перед merge.

## Изменение MCP public surface

1. Подтвердить, что изменение разрешено текущим ADR; если нет, сначала добавить новый ADR или обновить `ADR-0005`.
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
3. Добавить или обновить tests для `list_tools`, request DTO, shared envelope payload и business/runtime failure mapping по `ADR-0009`.
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

## Новая проверка результата внешнего инструмента

1. Назвать структурный ответ, на котором принимается решение: код выхода, обещанный
   артефакт или документированный машинный вывод. Формулировка вывода входом решения
   быть не может — см. [ADR-0029](../decisions/0029-proza-instrumenta-ne-prinimaet-resheniy.md).
2. Если структурного ответа нет, брать факт у вызывающего и называть неизвестность
   отдельным значением, а не выводить её из текста.
3. Прозу инструмента доносить до вызывающего как улику, не интерпретируя.
4. Снять замер на настоящем инструменте и записать его версию рядом с правилом;
   поддельный инструмент в тестах обязан повторять замер, а не собственную выдумку.
5. Проверить `cargo test --test tool_output_contract`: новое решение на тексте валит
   стража, а реестр `PROSE_DEBT` может только сокращаться.
