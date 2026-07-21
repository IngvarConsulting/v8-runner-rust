# Partial dump selector normalization

## Goal

Make the documented `dump --mode partial --object TYPE:NAME` selector work with
Designer, while retaining the already-used `TYPE.NAME` form as an explicitly
documented compatibility input.

## Contract

- `TYPE:NAME` is the canonical public form.
- `TYPE.NAME` is accepted for backward compatibility.
- Both forms are parsed before Designer is launched and emitted to its list file
  only as `TYPE.NAME`.
- A selector has a supported metadata type and one non-empty name. Control
  characters, empty parts, multiple separators, and unknown types are rejected
  as validation errors.
- The structured dump result reports each requested selector alongside its
  normalized platform selector.

## Design

Introduce a small domain value for a parsed partial-dump selector. Its parser
owns grammar validation and exposes a platform rendering method; callers no
longer pass raw selector strings beyond the request boundary. Metadata types are
represented by an exhaustive enum so new supported types require an explicit
mapping to the Designer spelling.

The dump coordinator parses `DumpRequest.objects` once, creates the temporary
Designer list file from normalized selector values, and places the parsed
request/normalized pairs in `DumpResult`. The CLI remains a thin adapter and
serializes the extended result unchanged.

## Error handling and safety

Parsing returns `AppError::Validation`; no invalid selector reaches a platform
process or temporary list file. Production paths use `Result` propagation and
do not use panic/unwrap for input handling. Borrowed `&str` and `&[T]`
interfaces are used where ownership is unnecessary.

## Tests

Use test-first development for:

1. canonical colon-form normalization into the captured Designer list file;
2. dotted legacy-form compatibility;
3. multiple selectors and structured requested/normalized result;
4. invalid type, empty name, extra separator, and control character rejection
   before Designer execution.

Update CLI help, capabilities documentation, and the project skill so they
describe the canonical and compatibility forms consistently.

## Scope exclusions

This change does not alter `IBCMD` partial-dump fallback behavior, source-set
selection, or the platform's metadata model beyond the selector types needed by
the existing public partial-dump contract.
