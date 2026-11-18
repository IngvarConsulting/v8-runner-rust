Test fixtures used by parser and selected integration tests.

- `parsers/designer_validation.log` maps to `src/parsers/designer_validation.rs`
- `parsers/edt_validation.log` maps to `src/parsers/edt_validation.rs`
- `parsers/junit_report.xml` maps to `src/parsers/junit.rs`
- `parsers/junit_smoke_report.xml` maps to `tests/cli_test.rs`
- `parsers/yaxunit.log` maps to `src/parsers/yaxunit_log.rs` and `tests/cli_test.rs`
- `fake-dbgs.rs` is the source of a fake 1C debug server that `tests/cli_debug_server.rs` compiles with `rustc` and starts through the real service spawn.
- `edt/` stores a deterministic native EDT sample used by `config init` regressions; it mirrors the local Designer fixtures after `DESIGNER -> EDT` convert and keeps only the stable `.project` / `DT-INF/PROJECT.PMF` / `src/**/*.{mdo,bsl,form}` markers needed for tests.

All fixtures are UTF-8 and are intended to hold larger realistic samples, while tiny edge cases stay inline in the individual tests.
