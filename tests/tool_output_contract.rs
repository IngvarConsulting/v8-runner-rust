//! Guard for ADR-0029: a tool's prose is never a decision input.
//!
//! The runner decides on what an external tool guarantees — its exit code, the artefacts
//! it produced, and output it documents as machine-readable. A sentence printed for a
//! human is none of those: the platform may reword it in any release, a patch one
//! included, and the wording differs per interface language. So a literal that reads like
//! prose must never reach a decision about a tool's result.
//!
//! The check reads the syntax, not the data flow: it cannot know which string came from a
//! tool, so it finds every literal that a decision is taken on — a comparison, a `match`
//! arm, or a named constant standing in for one — and asks whether that literal is a
//! structural token or prose. Structural tokens are ASCII without spaces: flags, keys,
//! enum values, paths, versions. Everything else is prose by construction, a single
//! Cyrillic word included.
//!
//! Syntax, not flow, is also the limit: the guard cannot tell a tool's sentence from our
//! own, so [`NOT_TOOL_OUTPUT`] carries the sites where the string is ours by construction.
//!
//! Scope is three layers, and the boundary was measured rather than assumed. Widening the
//! guard to `cli`, `mcp`, `config`, `support` and `domain` adds eleven hits, and none of them
//! is a tool's answer: `support/adapter_input.rs` accepts synonyms of a client mode from the
//! *caller* (tolerance on input is a feature, not a decision on output), while
//! `support/logging.rs` and `cli/execute.rs` match the runner's own text to colour and format
//! it. Those are presentation and input, so the layers stay out; a decision taken on a tool's
//! answer belongs in `platform`, `parsers` or `use_cases` by the layering of ADR-0006 and
//! ADR-0008.
//!
//! One blind spot is deliberate: a regular expression is not inspected. A legitimate Designer
//! parser must carry Cyrillic character classes, because 1C object names are Cyrillic, so a
//! rule over regex literals would fire on the structure of an identifier rather than on prose.
//! Review covers regexes instead — see the checklist entry for a new tool-result check.

mod guardrail_support;

use guardrail_support::{collect_rust_files, production_items};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{BinOp, Expr, ExprBinary, ExprMatch, ExprMethodCall, ItemConst, ItemStatic, Lit, Pat};

/// Layers that read what an external tool printed.
const TOOL_OUTPUT_LAYERS: &[&str] = &["platform", "parsers", "use_cases"];

/// Method names that turn a literal into a verdict about text.
const TEXT_PREDICATES: &[&str] = &[
    "contains",
    "starts_with",
    "ends_with",
    "eq",
    "eq_ignore_ascii_case",
];

/// Sites that still decide on prose, each with the work that removes it.
///
/// The ledger sizes the debt and stops it growing: a new prose decision fails this test,
/// and an entry may only leave the list. Shape is `(path from the repository root, the
/// literal as written)`.
const PROSE_DEBT: &[(&str, &str)] = &[
    // Designer syntax check: severity and the success verdict are read from the platform's
    // sentences. The verdict must come from the exit code; findings keep the text as
    // evidence. Tracked by #87.
    ("src/parsers/designer_validation.rs", "неразрешим"),
    ("src/parsers/designer_validation.rs", "ошиб"),
    ("src/parsers/designer_validation.rs", "ошибок не обнаружено"),
    ("src/parsers/designer_validation.rs", "предупреждение"),
    // EDT validation: the same shape over EDT's own output, whose language EDT decides.
    // Tracked by #87.
    ("src/parsers/edt_validation.rs", "блокир"),
    ("src/parsers/edt_validation.rs", "значит"),
    ("src/parsers/edt_validation.rs", "инф"),
    ("src/parsers/edt_validation.rs", "крит"),
    ("src/parsers/edt_validation.rs", "незнач"),
    ("src/parsers/edt_validation.rs", "ошиб"),
    ("src/parsers/edt_validation.rs", "предупр"),
    ("src/parsers/edt_validation.rs", "триви"),
    // Vanessa Automation log. Tracked by #87.
    ("src/parsers/vanessa_log.rs", "ошибк"),
    // EDT noise line dropped from captured output. Tracked by #87.
    (
        "src/platform/edt.rs",
        "Run '$exception printStackTrace' for error details",
    ),
    // The load compatibility probe, the defect this decision came from: both literals are
    // sentences, and the English pair is not what the platform writes in any language.
    // Tracked by #86.
    (
        "src/use_cases/load_artifact.rs",
        "configuration is not on support",
    ),
    (
        "src/use_cases/load_artifact.rs",
        "extension is not supported",
    ),
    (
        "src/use_cases/load_artifact.rs",
        "Конфигурация 'Расширение конфигурации' недоступна",
    ),
    // Our own prose, not a tool's, but the same disease: an internal signal carried as a
    // sentence instead of a typed value. Tracked by #88.
    ("src/use_cases/artifacts.rs", "critical phase"),
    ("src/use_cases/build_project/helpers.rs", "invalid data"),
    ("src/use_cases/tool_extension.rs", "invalid data"),
];

/// Sites where the literal is ours by construction, so no tool can reword it.
///
/// Both are the header the runner itself writes into a generated `v8project.yaml` and then
/// looks for to avoid writing it twice. Keep this list short and argued: it is the guard's
/// blind spot, not an escape hatch.
const NOT_TOOL_OUTPUT: &[(&str, &str)] = &[
    (
        "src/use_cases/config_init.rs",
        "# yaml-language-server: $schema=",
    ),
    (
        "src/use_cases/tools_download.rs",
        "# yaml-language-server: $schema=",
    ),
];

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A structural token is what a tool documents as machine-readable: a flag, a key, an
/// enum value, a path, a version. A separator — empty or whitespace only — is structural
/// too, because splitting output is not reading its prose.
fn is_structural(literal: &str) -> bool {
    if literal.trim().is_empty() {
        return true;
    }
    literal
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "_-./:=+*@%#$~|&;,()[]{}<>!?'\"\\".contains(ch))
}

#[derive(Default)]
struct DecisionLiterals {
    prose: BTreeSet<String>,
    prose_constants: Vec<(String, String)>,
    constant_decisions: BTreeSet<String>,
}

impl DecisionLiterals {
    fn record(&mut self, literal: &str) {
        if !is_structural(literal) {
            self.prose.insert(literal.to_owned());
        }
    }

    /// A list whose elements are message fragments is prose as data, and then a
    /// single-word element is a decision on prose just as much as a sentence is:
    /// `timeout` next to `access denied` is a fragment of someone's message, not a key.
    /// The alphabet alone cannot tell `timeout` from the key `active`, so the company a
    /// literal keeps decides.
    fn record_literal_list(&mut self, literals: &[String]) {
        if literals.iter().any(|literal| !is_structural(literal)) {
            for literal in literals {
                self.prose.insert(literal.clone());
            }
        }
    }

    fn record_pattern(&mut self, pattern: &Pat) {
        match pattern {
            Pat::Lit(pattern) => {
                if let Lit::Str(literal) = &pattern.lit {
                    self.record(&literal.value());
                }
            }
            Pat::Or(pattern) => {
                for case in &pattern.cases {
                    self.record_pattern(case);
                }
            }
            Pat::Tuple(pattern) => {
                for element in &pattern.elems {
                    self.record_pattern(element);
                }
            }
            Pat::Paren(pattern) => self.record_pattern(&pattern.pat),
            Pat::Reference(pattern) => self.record_pattern(&pattern.pat),
            Pat::TupleStruct(pattern) => {
                for element in &pattern.elems {
                    self.record_pattern(element);
                }
            }
            // A constant in a pattern position decides exactly like a literal would.
            Pat::Path(pattern) => {
                if let Some(segment) = pattern.path.segments.last() {
                    self.constant_decisions.insert(segment.ident.to_string());
                }
            }
            _ => {}
        }
    }

    /// A constant hides its literal from the comparison site, which is how the load probe
    /// kept a sentence as a decision input while reading as structural code.
    fn resolve_constants(&mut self) {
        let named = std::mem::take(&mut self.prose_constants);
        for (name, literal) in named {
            if self.constant_decisions.contains(&name) {
                self.prose.insert(literal);
            }
        }
    }
}

fn literal_of(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(expr) => match &expr.lit {
            Lit::Str(literal) => Some(literal.value()),
            _ => None,
        },
        Expr::Reference(expr) => literal_of(&expr.expr),
        Expr::Group(expr) => literal_of(&expr.expr),
        Expr::Paren(expr) => literal_of(&expr.expr),
        _ => None,
    }
}

fn constant_name_of(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(expr) => expr
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        Expr::Reference(expr) => constant_name_of(&expr.expr),
        Expr::Group(expr) => constant_name_of(&expr.expr),
        Expr::Paren(expr) => constant_name_of(&expr.expr),
        _ => None,
    }
}

/// Literals of a slice or array expression, however it is wrapped.
fn slice_literals(expr: &Expr) -> Vec<String> {
    match expr {
        Expr::Array(array) => array.elems.iter().filter_map(literal_of).collect(),
        Expr::Reference(expr) => slice_literals(&expr.expr),
        Expr::Group(expr) => slice_literals(&expr.expr),
        Expr::Paren(expr) => slice_literals(&expr.expr),
        _ => Vec::new(),
    }
}

impl<'ast> Visit<'ast> for DecisionLiterals {
    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if TEXT_PREDICATES.contains(&call.method.to_string().as_str()) {
            for argument in &call.args {
                if let Some(literal) = literal_of(argument) {
                    self.record(&literal);
                } else if let Some(name) = constant_name_of(argument) {
                    self.constant_decisions.insert(name);
                }
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_binary(&mut self, binary: &'ast ExprBinary) {
        if matches!(binary.op, BinOp::Eq(_) | BinOp::Ne(_)) {
            for side in [binary.left.as_ref(), binary.right.as_ref()] {
                if let Some(literal) = literal_of(side) {
                    self.record(&literal);
                } else if let Some(name) = constant_name_of(side) {
                    self.constant_decisions.insert(name);
                }
            }
        }
        syn::visit::visit_expr_binary(self, binary);
    }

    fn visit_expr_match(&mut self, expr: &'ast ExprMatch) {
        for arm in &expr.arms {
            self.record_pattern(&arm.pat);
        }
        syn::visit::visit_expr_match(self, expr);
    }

    fn visit_item_static(&mut self, item: &'ast ItemStatic) {
        self.record_literal_list(&slice_literals(&item.expr));
        syn::visit::visit_item_static(self, item);
    }

    fn visit_item_const(&mut self, item: &'ast ItemConst) {
        if let Some(literal) = literal_of(&item.expr) {
            if !is_structural(&literal) {
                self.prose_constants.push((item.ident.to_string(), literal));
            }
        }
        // A list of sentences is prose as data: `FATAL_PATTERNS.iter().any(|p|
        // text.contains(p))` decides on every element, and no element ever appears at
        // the comparison site. This is how the benign/fatal classification of `ibcmd`
        // output stayed invisible to the first version of this guard.
        self.record_literal_list(&slice_literals(&item.expr));
        syn::visit::visit_item_const(self, item);
    }
}

fn relative_path(path: &PathBuf) -> String {
    path.strip_prefix(repo_root())
        .expect("path inside repository")
        .to_string_lossy()
        .replace('\\', "/")
}

fn prose_decisions() -> BTreeSet<(String, String)> {
    let excluded = NOT_TOOL_OUTPUT
        .iter()
        .map(|(file, literal)| ((*file).to_owned(), (*literal).to_owned()))
        .collect::<BTreeSet<_>>();
    let mut sites = BTreeSet::new();
    for layer in TOOL_OUTPUT_LAYERS {
        let root = repo_root().join("src").join(layer);
        assert!(root.is_dir(), "missing layer src/{layer}");
        for file in collect_rust_files(&root) {
            let relative = relative_path(&file);
            let mut literals = DecisionLiterals::default();
            for item in production_items(&file) {
                literals.visit_item(&item);
            }
            literals.resolve_constants();
            for literal in literals.prose {
                let site = (relative.clone(), literal);
                if !excluded.contains(&site) {
                    sites.insert(site);
                }
            }
        }
    }
    sites
}

fn render(sites: &[&(String, String)]) -> String {
    sites
        .iter()
        .map(|(file, literal)| format!("  {file}: \"{literal}\""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn tool_prose_never_decides_and_the_declared_debt_only_shrinks() {
    let found = prose_decisions();
    let declared = PROSE_DEBT
        .iter()
        .map(|(file, literal)| ((*file).to_owned(), (*literal).to_owned()))
        .collect::<BTreeSet<_>>();

    let undeclared = found.difference(&declared).collect::<Vec<_>>();
    let stale = declared.difference(&found).collect::<Vec<_>>();

    assert!(
        undeclared.is_empty(),
        "ADR-0029: these decisions read a tool's prose. Decide on the exit code, a produced \
         artefact, or documented machine-readable output instead; if that work is not this \
         change, add the site to PROSE_DEBT with the issue that removes it.\n{}",
        render(&undeclared)
    );
    assert!(
        stale.is_empty(),
        "PROSE_DEBT names sites that no longer exist; remove them so the ledger keeps \
         measuring real debt:\n{}",
        render(&stale)
    );
}

#[test]
fn the_guard_sees_a_sentence_hidden_behind_a_constant_or_a_tuple_arm() {
    // The shapes the first version of this guard missed. Without them the defect that
    // prompted ADR-0029 stayed invisible: a regex over comparisons saw neither the
    // constant nor the literal inside a tuple pattern.
    let source = r#"
        const ABSENT: &str = "Конфигурация 'Расширение' недоступна";
        enum Kind { Configuration, Extension }
        fn classify(kind: Kind, diagnostic: &str, line: &str) -> u8 {
            if line.trim() == ABSENT {
                return 1;
            }
            match (kind, diagnostic) {
                (Kind::Configuration, "configuration is not on support") => 2,
                (Kind::Extension, "extension is not supported") => 3,
                _ => 0,
            }
        }
    "#;
    let file = syn::parse_file(source).expect("parse fixture");
    let mut literals = DecisionLiterals::default();
    for item in &file.items {
        literals.visit_item(item);
    }
    literals.resolve_constants();

    assert!(
        literals
            .prose
            .contains("Конфигурация 'Расширение' недоступна"),
        "a sentence behind a constant must count: {:?}",
        literals.prose
    );
    assert!(
        literals.prose.contains("configuration is not on support"),
        "the first literal of a tuple arm must count: {:?}",
        literals.prose
    );
    assert!(
        literals.prose.contains("extension is not supported"),
        "every or-pattern case must count: {:?}",
        literals.prose
    );
}

#[test]
fn structural_tokens_are_not_prose() {
    for structural in [
        "",
        "\n",
        "--dry-run",
        "configuration_cf",
        "active",
        "yes",
        "8.3.27.2074",
        "src/Configuration.xml",
        "name-prefix",
    ] {
        assert!(is_structural(structural), "{structural:?} is structural");
    }
    for prose in [
        "ошиб",
        "configuration is not on support",
        "Конфигурация 'Расширение конфигурации' недоступна",
        "critical phase",
    ] {
        assert!(!is_structural(prose), "{prose:?} is prose");
    }
}
