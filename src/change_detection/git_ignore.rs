//! Rules of git ignore files that can hide a file the platform writes into a source set (#575).
//!
//! Changes of a set inside a git working tree come from git, and git does not report a file it
//! ignores. A rule that can hide such a file refuses the command
//! (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`). The rules are read and
//! matched here, the way git matches them: the last matching rule decides, a negation lets a
//! path through, a directory that is ignored hides everything below it, rules of a deeper
//! `.gitignore` are stronger than those of a shallower one, `.git/info/exclude` and
//! `core.excludesFile` are the weakest.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::domain::git_worktree::GitIgnoreSource;

/// File extensions a Designer dump writes (#575): the paths a rule is tried on while the set
/// has no memory of the base yet.
const DUMP_EXTENSIONS: &[&str] = &[
    "xml", "bsl", "bin", "txt", "html", "htm", "png", "gif", "jpg", "jpeg", "bmp", "ico", "svg",
    "tif", "tiff", "wmf", "emf",
];

/// How many hidden paths a refusal names.
const NAMED_PATHS: usize = 5;

/// One rule line of an ignore file.
struct Rule {
    /// The line as written, without the line end.
    text: String,
    file: PathBuf,
    /// Line number from 1.
    line: usize,
    /// Directory the rule counts from, from the root of the working tree; empty — the root.
    base: String,
    negated: bool,
    dir_only: bool,
    /// Matched against the path below `base` when it holds a slash, else against the name.
    anchored: bool,
    pattern: Regex,
}

/// The rules of every ignore file in force, weakest first.
pub(crate) struct IgnoreRules {
    rules: Vec<Rule>,
    ignore_case: bool,
}

/// A rule that hides files of a set, with the first of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenBy {
    /// The rule as written.
    pub rule: String,
    /// The ignore file that holds it.
    pub file: PathBuf,
    /// Its line in that file, from 1.
    pub line: usize,
    /// The first hidden paths below the set root, `/`-separated; the root itself is `.`.
    pub paths: Vec<String>,
}

impl IgnoreRules {
    /// Reads every source; a file that is absent or unreadable holds no rules, as for git.
    pub(crate) fn read(sources: &[GitIgnoreSource], ignore_case: bool) -> Self {
        let mut rules = Vec::new();
        for source in sources {
            let Ok(bytes) = std::fs::read(&source.file) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            for (index, line) in text.split('\n').enumerate() {
                if let Some(rule) =
                    Rule::parse(line, &source.file, index + 1, &source.base, ignore_case)
                {
                    rules.push(rule);
                }
            }
        }
        Self { rules, ignore_case }
    }

    /// The rule that hides the first of `candidates` (paths below the set root at `root`,
    /// from the root of the working tree) or the root itself, with up to [`NAMED_PATHS`] of
    /// the paths it hides. `None`: no rule hides any of them.
    pub(crate) fn hiding(
        &self,
        root: &str,
        candidates: impl IntoIterator<Item = String>,
    ) -> Option<HiddenBy> {
        let mut directories = HashMap::new();
        if !root.is_empty() {
            if let Some(rule) = self.verdict(root, true, &mut directories) {
                return Some(self.hit(rule, vec![".".to_owned()]));
            }
        }
        let mut found: Option<(usize, Vec<String>)> = None;
        for candidate in candidates {
            let path = if root.is_empty() {
                candidate.clone()
            } else {
                format!("{root}/{candidate}")
            };
            let Some(rule) = self.verdict(&path, false, &mut directories) else {
                continue;
            };
            match &mut found {
                None => found = Some((rule, vec![candidate])),
                Some((first, paths)) if *first == rule => {
                    paths.push(candidate);
                    if paths.len() >= NAMED_PATHS {
                        break;
                    }
                }
                Some(_) => {}
            }
        }
        found.map(|(rule, paths)| self.hit(rule, paths))
    }

    fn hit(&self, rule: usize, paths: Vec<String>) -> HiddenBy {
        let rule = &self.rules[rule];
        HiddenBy {
            rule: rule.text.clone(),
            file: rule.file.clone(),
            line: rule.line,
            paths,
        }
    }

    /// The rule that ignores `path`: a parent directory that is ignored first, then the last
    /// rule that matches the path itself, unless that one is a negation.
    fn verdict(
        &self,
        path: &str,
        is_dir: bool,
        directories: &mut HashMap<String, Option<usize>>,
    ) -> Option<usize> {
        if let Some((parent, _)) = path.rsplit_once('/') {
            let parent_verdict = match directories.get(parent) {
                Some(verdict) => *verdict,
                None => {
                    let verdict = self.verdict(parent, true, directories);
                    directories.insert(parent.to_owned(), verdict);
                    verdict
                }
            };
            if parent_verdict.is_some() {
                return parent_verdict;
            }
        }
        let last = self
            .rules
            .iter()
            .enumerate()
            .rev()
            .find(|(_, rule)| rule.matches(path, is_dir, self.ignore_case))?;
        (!last.1.negated).then_some(last.0)
    }
}

impl Rule {
    fn parse(raw: &str, file: &Path, line: usize, base: &str, ignore_case: bool) -> Option<Self> {
        let text = raw.strip_suffix('\r').unwrap_or(raw);
        if text.is_empty() || text.starts_with('#') {
            return None;
        }
        let mut pattern = trim_unescaped_trailing_spaces(text);
        let negated = pattern.starts_with('!');
        if negated {
            pattern = &pattern[1..];
        }
        let dir_only = pattern.ends_with('/') && !pattern.ends_with("\\/");
        let pattern = pattern
            .strip_suffix('/')
            .filter(|_| dir_only)
            .unwrap_or(pattern);
        if pattern.is_empty() {
            return None;
        }
        let anchored = pattern.contains('/');
        let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
        let regex = Regex::new(&format!(
            "{}^{}$",
            if ignore_case { "(?i)" } else { "" },
            glob_to_regex(pattern)
        ))
        .ok()?;
        Some(Self {
            text: text.to_owned(),
            file: file.to_path_buf(),
            line,
            base: base.to_owned(),
            negated,
            dir_only,
            anchored,
            pattern: regex,
        })
    }

    fn matches(&self, path: &str, is_dir: bool, ignore_case: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        let below = if self.base.is_empty() {
            path
        } else {
            match strip_dir_prefix(path, &self.base, ignore_case) {
                Some(below) => below,
                None => return false,
            }
        };
        if self.anchored {
            self.pattern.is_match(below)
        } else {
            self.pattern
                .is_match(below.rsplit('/').next().unwrap_or(below))
        }
    }
}

/// `path` below the directory `dir`, both from the root of the working tree.
fn strip_dir_prefix<'a>(path: &'a str, dir: &str, ignore_case: bool) -> Option<&'a str> {
    let head = path.get(..dir.len())?;
    let same = if ignore_case {
        head.eq_ignore_ascii_case(dir)
    } else {
        head == dir
    };
    (same && path.as_bytes().get(dir.len()) == Some(&b'/')).then(|| &path[dir.len() + 1..])
}

/// Trailing spaces end a pattern unless a backslash escapes them.
fn trim_unescaped_trailing_spaces(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 && text.as_bytes()[end - 1] == b' ' {
        if end >= 2 && text.as_bytes()[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    &text[..end]
}

/// A git wildcard pattern as a regular expression over a `/`-separated path: `*` and `?` stay
/// inside one component, `**` between slashes crosses any number of them.
fn glob_to_regex(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut regex = String::new();
    let mut i = 0;
    while i < chars.len() {
        let at_start = i == 0 || chars[i - 1] == '/';
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') && at_start => {
                let end = i + 2;
                if end == chars.len() {
                    regex.push_str(".*");
                    i = end;
                } else if chars[end] == '/' {
                    regex.push_str("(?:.*/)?");
                    i = end + 1;
                } else {
                    regex.push_str("[^/]*");
                    i = end;
                }
            }
            '*' => {
                regex.push_str("[^/]*");
                i += 1;
                while chars.get(i) == Some(&'*') {
                    i += 1;
                }
            }
            '?' => {
                regex.push_str("[^/]");
                i += 1;
            }
            '[' => match chars[i + 1..].iter().position(|c| *c == ']') {
                // `[]` as the first character of a class is a literal bracket.
                Some(close) if close > 0 || chars.get(i + 2).is_some() => {
                    let close = if close == 0 {
                        chars[i + 2..].iter().position(|c| *c == ']').map(|p| p + 1)
                    } else {
                        Some(close)
                    };
                    match close {
                        Some(close) => {
                            let class: String = chars[i + 1..i + 1 + close].iter().collect();
                            regex.push('[');
                            let class = match class.strip_prefix(['!', '^']) {
                                Some(rest) => {
                                    regex.push('^');
                                    rest.to_owned()
                                }
                                None => class,
                            };
                            for c in class.chars() {
                                if matches!(c, '\\' | '[' | ']' | '&' | '~') {
                                    regex.push('\\');
                                }
                                regex.push(c);
                            }
                            regex.push(']');
                            i += close + 2;
                        }
                        None => {
                            regex.push_str("\\[");
                            i += 1;
                        }
                    }
                }
                _ => {
                    regex.push_str("\\[");
                    i += 1;
                }
            },
            '\\' if i + 1 < chars.len() => {
                regex.push_str(&regex::escape(&chars[i + 1].to_string()));
                i += 2;
            }
            c => {
                regex.push_str(&regex::escape(&c.to_string()));
                i += 1;
            }
        }
    }
    regex
}

/// The paths a dump of a configuration writes, as samples below a set root: every folder of
/// the dump layout ([`crate::change_detection::partial_load::dump_layout_folders`]) with an
/// element, its `Ext` folder and a file of every extension a dump writes. A set without
/// memory of the base is checked against them.
pub(crate) fn dump_samples() -> Vec<String> {
    let (top, nested) = crate::change_detection::partial_load::dump_layout_folders();
    let mut samples = Vec::new();
    for extension in DUMP_EXTENSIONS {
        samples.push(format!("Configuration.{extension}"));
        samples.push(format!("Ext/Sample.{extension}"));
        for folder in &top {
            samples.push(format!("{folder}/Sample.{extension}"));
            samples.push(format!("{folder}/Sample/Ext/Sample.{extension}"));
            for inner in &nested {
                samples.push(format!("{folder}/Sample/{inner}/Sample.{extension}"));
                samples.push(format!(
                    "{folder}/Sample/{inner}/Sample/Ext/Sample.{extension}"
                ));
            }
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::{dump_samples, IgnoreRules};
    use crate::domain::git_worktree::GitIgnoreSource;
    use std::fs;

    fn rules(files: &[(&str, &str)]) -> (tempfile::TempDir, IgnoreRules) {
        let dir = tempfile::tempdir().expect("tempdir");
        let sources = files
            .iter()
            .map(|(base, text)| {
                let folder = dir.path().join(base);
                fs::create_dir_all(&folder).expect("folder");
                let file = folder.join(".gitignore");
                fs::write(&file, text).expect("rules");
                GitIgnoreSource {
                    file,
                    base: (*base).to_owned(),
                }
            })
            .collect::<Vec<_>>();
        let rules = IgnoreRules::read(&sources, false);
        (dir, rules)
    }

    fn memory(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn a_rule_that_meets_no_dumped_file_hides_nothing() {
        let (_dir, rules) = rules(&[("", "*.log\n")]);
        assert_eq!(rules.hiding("src/cf", dump_samples()), None);
        assert_eq!(
            rules.hiding(
                "src/cf",
                memory(&["Configuration.xml", "Ext/ParentConfigurations.bin"])
            ),
            None
        );
    }

    #[test]
    fn an_unanchored_rule_hides_a_remembered_file_at_any_depth() {
        let (_dir, rules) = rules(&[("", "*.bin\n")]);
        let hidden = rules
            .hiding(
                "src/cf",
                memory(&["Configuration.xml", "Ext/ParentConfigurations.bin"]),
            )
            .expect("hidden");
        assert_eq!(hidden.rule, "*.bin");
        assert_eq!(hidden.line, 1);
        assert_eq!(hidden.paths, ["Ext/ParentConfigurations.bin"]);
    }

    #[test]
    fn an_unanchored_directory_rule_hides_the_folders_of_a_dump() {
        let (_dir, rules) = rules(&[("", "# comment\n\nCommonModules/\n")]);
        let hidden = rules
            .hiding("src/cf", memory(&["CommonModules/Common/Ext/Module.bsl"]))
            .expect("hidden");
        assert_eq!((hidden.rule.as_str(), hidden.line), ("CommonModules/", 3));
        assert!(rules.hiding("src/cf", dump_samples()).is_some());
    }

    #[test]
    fn without_memory_the_rule_is_tried_on_the_files_a_dump_writes() {
        let (_dir, bsl) = rules(&[("", "*.bsl\n")]);
        assert!(bsl.hiding("src/cf", dump_samples()).is_some());
        let (_dir, log) = rules(&[("", "*.log\n")]);
        assert_eq!(log.hiding("src/cf", dump_samples()), None);
    }

    #[test]
    fn a_rule_anchored_outside_the_root_hides_nothing() {
        let (_dir, rules) = rules(&[("", "/build/\n/docs/*.xml\n")]);
        assert_eq!(rules.hiding("src/cf", dump_samples()), None);
    }

    #[test]
    fn a_rule_anchored_above_the_root_hides_the_root() {
        let (_dir, rules) = rules(&[("", "/src/\n")]);
        let hidden = rules
            .hiding("src/cf", memory(&["Configuration.xml"]))
            .expect("root");
        assert_eq!(hidden.paths, ["."]);
    }

    #[test]
    fn a_negation_lets_the_set_through() {
        let (_dir, rules) = rules(&[("", "/*\n!/src/\n*.bsl\n!*.bsl\n")]);
        assert_eq!(rules.hiding("src/cf", dump_samples()), None);
    }

    #[test]
    fn a_rule_of_a_gitignore_inside_the_set_counts_from_its_folder() {
        let (_dir, rules) = rules(&[("src/cf/Catalogs", "Items/\n"), ("other", "*.xml\n")]);
        let hidden = rules
            .hiding(
                "src/cf",
                memory(&[
                    "Catalogs/Items/Ext/ObjectModule.bsl",
                    "Documents/Items/Ext/ObjectModule.bsl",
                ]),
            )
            .expect("hidden");
        assert_eq!(hidden.paths, ["Catalogs/Items/Ext/ObjectModule.bsl"]);
    }

    #[test]
    fn the_version_file_rule_hides_nothing_the_runner_reads() {
        let (_dir, rules) = rules(&[("", "ConfigDumpInfo.xml\n")]);
        assert_eq!(rules.hiding("src/cf", dump_samples()), None);
        assert_eq!(rules.hiding("src/cf", memory(&["Configuration.xml"])), None);
    }

    #[test]
    fn a_double_star_crosses_folders() {
        let (_dir, rules) = rules(&[("", "src/**/Ext/*.bin\n")]);
        assert!(rules
            .hiding("src/cf", memory(&["Catalogs/A/Ext/Picture.bin"]))
            .is_some());
    }
}
