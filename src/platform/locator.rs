use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::support::path::{nearest_existing_canonical_path, normalize_windows_verbatim_path};

/// Executable-oriented platform utility identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UtilityType {
    /// `1cv8`
    V8,
    /// `1cv8c`
    V8C,
    /// `ibcmd`
    Ibcmd,
    /// `1cedtcli`
    EdtCli,
}

impl UtilityType {
    /// Executable filename for the current platform.
    pub fn executable_name(self) -> &'static str {
        match self {
            Self::V8 => executable_name_for("1cv8"),
            Self::V8C => executable_name_for("1cv8c"),
            Self::Ibcmd => executable_name_for("ibcmd"),
            Self::EdtCli => executable_name_for("1cedtcli"),
        }
    }

    /// Returns `true` for regular 1C platform binaries.
    pub fn is_platform(self) -> bool {
        !matches!(self, Self::EdtCli)
    }
}

impl fmt::Display for UtilityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.executable_name())
    }
}

/// Exact 4-part 1C platform version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlatformVersion {
    /// Major version component.
    pub major: u32,
    /// Minor version component.
    pub minor: u32,
    /// Patch version component.
    pub patch: u32,
    /// Build version component.
    pub build: u32,
}

impl PlatformVersion {
    /// Parse a strict `major.minor.patch.build` string.
    pub fn parse_strict(value: &str) -> Option<Self> {
        let parts = value
            .split('.')
            .map(str::trim)
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;

        if parts.len() != 4 {
            return None;
        }

        Some(Self {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
            build: parts[3],
        })
    }
}

impl fmt::Display for PlatformVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.major, self.minor, self.patch, self.build
        )
    }
}

/// 1C platform version requirement from configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlatformVersionRequirement {
    /// Major version component.
    pub major: u32,
    /// Minor version component.
    pub minor: u32,
    /// Optional patch version component.
    pub patch: Option<u32>,
    /// Optional build version component.
    pub build: Option<u32>,
}

impl PlatformVersionRequirement {
    /// Parse `major.minor`, `major.minor.patch`, or `major.minor.patch.build`.
    pub fn parse(value: &str) -> Option<Self> {
        let parts = value
            .split('.')
            .map(str::trim)
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;

        if !(2..=4).contains(&parts.len()) {
            return None;
        }

        Some(Self {
            major: parts[0],
            minor: parts[1],
            patch: parts.get(2).copied(),
            build: parts.get(3).copied(),
        })
    }

    /// Returns `true` when a discovered platform version satisfies this requirement.
    pub fn matches(&self, version: &PlatformVersion) -> bool {
        self.major == version.major
            && self.minor == version.minor
            && self
                .patch
                .map(|patch| patch == version.patch)
                .unwrap_or(true)
            && self
                .build
                .map(|build| build == version.build)
                .unwrap_or(true)
    }
}

impl fmt::Display for PlatformVersionRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.patch, self.build) {
            (Some(patch), Some(build)) => {
                write!(f, "{}.{}.{}.{}", self.major, self.minor, patch, build)
            }
            (Some(patch), None) => write!(f, "{}.{}.{}", self.major, self.minor, patch),
            (None, None) => write!(f, "{}.{}", self.major, self.minor),
            (None, Some(_)) => unreachable!("build cannot be parsed without patch"),
        }
    }
}

/// EDT discovery version parsed leniently from numeric tokens.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdtVersion {
    /// Numeric tokens extracted from a path or directory name.
    pub parts: Vec<u32>,
}

impl EdtVersion {
    /// Parse a version from any string that contains numeric tokens.
    pub fn parse_lenient(value: &str) -> Option<Self> {
        let parts: Vec<u32> = value
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u32>().ok())
            .collect();

        if parts.is_empty() {
            None
        } else {
            Some(Self { parts })
        }
    }
}

/// Parsed utility version metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UtilityVersion {
    /// Exact 1C platform version.
    Platform(PlatformVersion),
    /// Lenient EDT discovery version.
    Edt(EdtVersion),
}

/// Resolution behavior for configured platform installation hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlatformResolutionPolicy {
    /// Do not enforce `tools.platform.version` for a configured platform path.
    #[default]
    Lenient,
    /// Enforce `tools.platform.version` inside a configured platform path boundary.
    Strict,
}

/// Inputs used to construct a locator with OS-default discovery roots.
#[derive(Debug, Clone, Default)]
pub struct LocatorOptions {
    /// Configured platform executable or installation boundary.
    pub platform_hint: Option<PathBuf>,
    /// Optional platform version prefix or exact build.
    pub platform_version: Option<PlatformVersionRequirement>,
    /// Whether a configured platform path enforces `platform_version`.
    pub platform_policy: PlatformResolutionPolicy,
    /// Configured EDT executable or installation hint.
    pub edt_hint: Option<PathBuf>,
    /// Optional EDT discovery version.
    pub edt_version: Option<EdtVersion>,
}

/// Typed origin of a resolved executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionSource {
    /// The configured utility or installation hint.
    Explicit,
    /// An operating-system-specific default installation root.
    DefaultRoot,
    /// A directory captured from `PATH` when the locator was created.
    Path,
}

/// Resolved utility path together with parsed version information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtilityLocation {
    /// Utility kind that was resolved.
    pub utility: UtilityType,
    /// Absolute path to the executable.
    pub path: PathBuf,
    /// Parsed version metadata if it could be derived from the path.
    pub version: Option<UtilityVersion>,
    /// Origin used to discover the executable.
    pub source: ResolutionSource,
    /// Canonical root shared by sibling executables from this installation.
    pub installation_root: PathBuf,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LocatorError {
    #[error("utility '{0}' was not found")]
    NotFound(UtilityType),
    #[error("utility '{utility}' was not found inside strict platform boundary '{boundary}'")]
    StrictBoundaryNotFound {
        utility: UtilityType,
        boundary: PathBuf,
    },
    #[error(
        "utility '{utility}' at '{}' has unknown platform version; required {required}",
        path.display()
    )]
    UnknownVersion {
        utility: UtilityType,
        path: PathBuf,
        required: PlatformVersionRequirement,
    },
    #[error(
        "utility '{utility}' at '{}' has platform version {found}; required {required}",
        path.display()
    )]
    VersionMismatch {
        utility: UtilityType,
        path: PathBuf,
        required: PlatformVersionRequirement,
        found: PlatformVersion,
    },
    #[error(
        "utility '{utility}' is missing from pinned platform installation '{}'",
        installation_root.display()
    )]
    MissingSibling {
        utility: UtilityType,
        installation_root: PathBuf,
    },
}

#[derive(Debug, Clone)]
struct Candidate {
    path: PathBuf,
    version: Option<UtilityVersion>,
    source: ResolutionSource,
}

#[derive(Debug, Clone)]
struct CanonicalCandidate {
    path: PathBuf,
    version: Option<UtilityVersion>,
    source: ResolutionSource,
    installation_root: PathBuf,
}

#[derive(Debug, Clone)]
struct PinnedPlatformInstallation {
    root: PathBuf,
    source: ResolutionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileHintSiblingResolution {
    Lexical,
    CanonicalInstallation,
}

/// Stateful utility locator with per-instance cache.
pub struct Locator {
    platform_hint: Option<PathBuf>,
    platform_version: Option<PlatformVersionRequirement>,
    platform_policy: PlatformResolutionPolicy,
    edt_hint: Option<PathBuf>,
    edt_version: Option<EdtVersion>,
    cache: HashMap<(UtilityType, Option<String>), UtilityLocation>,
    platform_roots: Vec<PathBuf>,
    edt_roots: Vec<PathBuf>,
    path_roots: Vec<PathBuf>,
    pinned_platform: Option<PinnedPlatformInstallation>,
}

impl Locator {
    /// Build a locator using default OS-specific search roots.
    pub fn new(options: LocatorOptions) -> Self {
        Self {
            platform_hint: options.platform_hint,
            platform_version: options.platform_version,
            platform_policy: options.platform_policy,
            edt_hint: options.edt_hint,
            edt_version: options.edt_version,
            cache: HashMap::new(),
            platform_roots: default_platform_roots(),
            edt_roots: default_edt_roots(),
            path_roots: captured_path_roots(),
            pinned_platform: None,
        }
    }

    /// Resolve an executable path for the requested utility.
    pub fn locate(&mut self, utility: UtilityType) -> Result<UtilityLocation, LocatorError> {
        let cache_key = (utility, self.version_requirement_string(utility));

        if let Some(cached) = self.cache.get(&cache_key).cloned() {
            if let Some(revalidated) = self.revalidate_cached_location(utility, &cached) {
                return Ok(revalidated);
            }
            self.cache.remove(&cache_key);
        }

        let selected = if utility.is_platform() {
            self.locate_platform(utility)?
        } else {
            self.locate_edt(utility)?
        };
        self.cache.insert(cache_key, selected.clone());
        Ok(selected)
    }

    fn revalidate_cached_location(
        &self,
        utility: UtilityType,
        cached: &UtilityLocation,
    ) -> Option<UtilityLocation> {
        if cached.utility != utility {
            return None;
        }
        if utility.is_platform()
            && self.platform_hint.is_some()
            && self.platform_policy == PlatformResolutionPolicy::Lenient
        {
            return None;
        }

        let candidate = Candidate {
            path: cached.path.clone(),
            version: cached.version.clone(),
            source: cached.source,
        };
        let current = match (utility, self.platform_policy, self.pinned_platform.as_ref()) {
            (
                UtilityType::V8 | UtilityType::V8C | UtilityType::Ibcmd,
                PlatformResolutionPolicy::Strict,
                Some(pinned),
            ) => select_pinned_candidate(
                utility,
                vec![candidate],
                self.effective_platform_version_requirement(),
                &pinned.root,
            ),
            (
                UtilityType::V8 | UtilityType::V8C | UtilityType::Ibcmd,
                PlatformResolutionPolicy::Strict,
                None,
            ) => {
                let boundary = self.platform_hint.as_deref().map(strict_candidate_boundary);
                select_candidate(
                    utility,
                    vec![candidate],
                    self.effective_platform_version_requirement(),
                    boundary.as_deref(),
                )
            }
            (
                UtilityType::V8 | UtilityType::V8C | UtilityType::Ibcmd,
                PlatformResolutionPolicy::Lenient,
                Some(_) | None,
            ) => select_candidate(
                utility,
                vec![candidate],
                self.effective_platform_version_requirement(),
                None,
            ),
            (
                UtilityType::EdtCli,
                PlatformResolutionPolicy::Lenient | PlatformResolutionPolicy::Strict,
                Some(_) | None,
            ) => select_edt_candidate(vec![candidate], utility, None),
        }?;

        (current == *cached).then_some(current)
    }

    #[cfg(test)]
    pub(crate) fn with_roots(
        platform_hint: Option<PathBuf>,
        platform_version: Option<PlatformVersionRequirement>,
        edt_hint: Option<PathBuf>,
        edt_version: Option<EdtVersion>,
        platform_roots: Vec<PathBuf>,
        edt_roots: Vec<PathBuf>,
    ) -> Self {
        Self::with_search_roots(
            platform_hint,
            platform_version,
            PlatformResolutionPolicy::Lenient,
            edt_hint,
            edt_version,
            platform_roots,
            edt_roots,
            Vec::new(),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_search_roots(
        platform_hint: Option<PathBuf>,
        platform_version: Option<PlatformVersionRequirement>,
        platform_policy: PlatformResolutionPolicy,
        edt_hint: Option<PathBuf>,
        edt_version: Option<EdtVersion>,
        platform_roots: Vec<PathBuf>,
        edt_roots: Vec<PathBuf>,
        path_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            platform_hint,
            platform_version,
            platform_policy,
            edt_hint,
            edt_version,
            cache: HashMap::new(),
            platform_roots,
            edt_roots,
            path_roots,
            pinned_platform: None,
        }
    }

    fn version_requirement_string(&self, utility: UtilityType) -> Option<String> {
        if utility.is_platform() {
            self.effective_platform_version_requirement()
                .map(ToString::to_string)
        } else {
            self.edt_version.as_ref().map(|version| {
                version
                    .parts
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            })
        }
    }

    fn effective_platform_version_requirement(&self) -> Option<&PlatformVersionRequirement> {
        if self.platform_hint.is_some() && self.platform_policy == PlatformResolutionPolicy::Lenient
        {
            None
        } else {
            self.platform_version.as_ref()
        }
    }

    fn locate_platform(&mut self, utility: UtilityType) -> Result<UtilityLocation, LocatorError> {
        if self.platform_policy == PlatformResolutionPolicy::Strict {
            if let Some(pinned) = self.pinned_platform.as_ref() {
                return select_pinned_candidate(
                    utility,
                    pinned_platform_candidates(utility, pinned),
                    self.effective_platform_version_requirement(),
                    &pinned.root,
                )
                .ok_or_else(|| LocatorError::MissingSibling {
                    utility,
                    installation_root: pinned.root.clone(),
                });
            }
        }

        let direct_explicit_candidates = self
            .platform_hint
            .as_deref()
            .map(|hint| {
                explicit_direct_candidates(
                    hint,
                    utility,
                    match self.platform_policy {
                        PlatformResolutionPolicy::Lenient => FileHintSiblingResolution::Lexical,
                        PlatformResolutionPolicy::Strict => {
                            FileHintSiblingResolution::CanonicalInstallation
                        }
                    },
                )
            })
            .unwrap_or_default();
        let strict_boundary = match self.platform_policy {
            PlatformResolutionPolicy::Lenient => None,
            PlatformResolutionPolicy::Strict => {
                self.platform_hint.as_deref().map(strict_candidate_boundary)
            }
        };
        let versioned_explicit_candidates = self
            .platform_hint
            .as_deref()
            .filter(|hint| hint.is_dir())
            .map(|hint| {
                platform_candidates_any_version(
                    utility,
                    std::slice::from_ref(&hint.to_path_buf()),
                    ResolutionSource::Explicit,
                )
            })
            .unwrap_or_default();

        if self.platform_hint.is_some() {
            let mut explicit_candidates = direct_explicit_candidates;
            explicit_candidates.extend(versioned_explicit_candidates);
            let required = self.effective_platform_version_requirement();
            if let Some(location) = select_candidate(
                utility,
                explicit_candidates.clone(),
                required,
                strict_boundary.as_deref(),
            ) {
                self.pin_platform(&location);
                return Ok(location);
            }
            return match self.platform_policy {
                PlatformResolutionPolicy::Strict => Err(strict_resolution_error(
                    utility,
                    self.platform_hint.as_deref(),
                    explicit_candidates,
                    required,
                    strict_boundary.as_deref(),
                )),
                PlatformResolutionPolicy::Lenient => Err(LocatorError::NotFound(utility)),
            };
        }

        let mut candidates = platform_candidates_any_version(
            utility,
            &self.platform_roots,
            ResolutionSource::DefaultRoot,
        );
        candidates.extend(path_candidates(utility, &self.path_roots));
        let location = select_candidate(
            utility,
            candidates,
            self.effective_platform_version_requirement(),
            None,
        )
        .ok_or(LocatorError::NotFound(utility))?;
        self.pin_platform(&location);
        Ok(location)
    }

    fn pin_platform(&mut self, location: &UtilityLocation) {
        if self.platform_policy != PlatformResolutionPolicy::Strict || self.platform_hint.is_none()
        {
            return;
        }
        self.pinned_platform = Some(PinnedPlatformInstallation {
            root: location.installation_root.clone(),
            source: location.source,
        });
    }

    fn locate_edt(&self, utility: UtilityType) -> Result<UtilityLocation, LocatorError> {
        let direct_explicit = self
            .edt_hint
            .as_deref()
            .map(|hint| {
                explicit_direct_candidates(hint, utility, FileHintSiblingResolution::Lexical)
            })
            .unwrap_or_default();
        if let Some(location) = select_edt_candidate(direct_explicit, utility, None) {
            return Ok(location);
        }

        let versioned_explicit = match self.edt_hint.as_ref() {
            Some(hint) if hint.is_dir() => edt_candidates_any_version(
                utility,
                std::slice::from_ref(hint),
                ResolutionSource::Explicit,
            ),
            Some(_) | None => Vec::new(),
        };
        if let Some(location) =
            select_edt_candidate(versioned_explicit, utility, self.edt_version.as_ref())
        {
            return Ok(location);
        }

        let mut candidates =
            edt_candidates_any_version(utility, &self.edt_roots, ResolutionSource::DefaultRoot);
        candidates.extend(path_candidates(utility, &self.path_roots));
        select_edt_candidate(candidates, utility, self.edt_version.as_ref())
            .ok_or(LocatorError::NotFound(utility))
    }
}

fn compare_versions(
    left: Option<&UtilityVersion>,
    right: Option<&UtilityVersion>,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(UtilityVersion::Platform(a)), Some(UtilityVersion::Platform(b))) => a.cmp(b),
        (Some(UtilityVersion::Edt(a)), Some(UtilityVersion::Edt(b))) => a.cmp(b),
        (Some(UtilityVersion::Platform(_)), Some(UtilityVersion::Edt(_)))
        | (Some(UtilityVersion::Edt(_)), Some(UtilityVersion::Platform(_))) => {
            std::cmp::Ordering::Equal
        }
        (Some(UtilityVersion::Platform(_)) | Some(UtilityVersion::Edt(_)), None) => {
            std::cmp::Ordering::Greater
        }
        (None, Some(UtilityVersion::Platform(_)) | Some(UtilityVersion::Edt(_))) => {
            std::cmp::Ordering::Less
        }
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn explicit_direct_candidates(
    hint: &Path,
    utility: UtilityType,
    sibling_resolution: FileHintSiblingResolution,
) -> Vec<Candidate> {
    if hint.is_file() {
        let canonical_hint = canonical_boundary(hint);
        let target_name = utility.executable_name();
        let file_name_matches = hint
            .file_name()
            .map(|name| executable_component_matches(name, target_name))
            .unwrap_or(false);

        let path = if file_name_matches {
            canonical_hint
        } else {
            let sibling_base = match sibling_resolution {
                FileHintSiblingResolution::Lexical => hint,
                FileHintSiblingResolution::CanonicalInstallation => canonical_hint.as_path(),
            };
            match sibling_base.parent() {
                Some(parent) => parent.join(target_name),
                None => return Vec::new(),
            }
        };
        return vec![candidate_from_path(
            path,
            utility,
            ResolutionSource::Explicit,
        )];
    }

    if hint.is_dir() {
        return direct_candidates(hint, utility, ResolutionSource::Explicit);
    }

    Vec::new()
}

fn direct_candidates(
    root: &Path,
    utility: UtilityType,
    source: ResolutionSource,
) -> Vec<Candidate> {
    [
        root.join("bin").join(utility.executable_name()),
        root.join(utility.executable_name()),
    ]
    .into_iter()
    .map(|path| candidate_from_path(path, utility, source))
    .collect()
}

fn candidate_from_path(path: PathBuf, utility: UtilityType, source: ResolutionSource) -> Candidate {
    Candidate {
        version: infer_version(utility, &path),
        path,
        source,
    }
}

fn platform_candidates_any_version(
    utility: UtilityType,
    roots: &[PathBuf],
    source: ResolutionSource,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let Some(dir_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(version) = PlatformVersion::parse_strict(dir_name) else {
                continue;
            };

            candidates.push(Candidate {
                path: path.join(utility.executable_name()),
                version: Some(UtilityVersion::Platform(version.clone())),
                source,
            });
            candidates.push(Candidate {
                path: path.join("bin").join(utility.executable_name()),
                version: Some(UtilityVersion::Platform(version)),
                source,
            });
        }
    }

    candidates
}

fn edt_candidates_any_version(
    utility: UtilityType,
    roots: &[PathBuf],
    source: ResolutionSource,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let version = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(EdtVersion::parse_lenient)
                .map(UtilityVersion::Edt);

            candidates.push(Candidate {
                path: path.join("1cedt").join(utility.executable_name()),
                version: version.clone(),
                source,
            });
            candidates.push(Candidate {
                path: path.join(utility.executable_name()),
                version,
                source,
            });
        }
    }

    candidates
}

fn edt_version_matches(required: &EdtVersion, candidate: &EdtVersion) -> bool {
    if required.parts.is_empty() {
        return false;
    }

    candidate
        .parts
        .windows(required.parts.len())
        .any(|window| window == required.parts.as_slice())
}

fn path_candidates(utility: UtilityType, roots: &[PathBuf]) -> Vec<Candidate> {
    roots
        .iter()
        .map(|dir| {
            candidate_from_path(
                dir.join(utility.executable_name()),
                utility,
                ResolutionSource::Path,
            )
        })
        .collect()
}

fn captured_path_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    match std::env::current_dir() {
        Ok(current_dir) => normalize_path_roots(roots, &current_dir),
        Err(_) => roots
            .into_iter()
            .filter(|root| root.is_absolute())
            .collect(),
    }
}

fn normalize_path_roots(roots: Vec<PathBuf>, current_dir: &Path) -> Vec<PathBuf> {
    roots
        .into_iter()
        .filter_map(|root| {
            if root.is_absolute() {
                Some(root)
            } else {
                absolutize_relative_path_root(root, current_dir)
            }
        })
        .collect()
}

#[cfg(not(windows))]
fn absolutize_relative_path_root(root: PathBuf, current_dir: &Path) -> Option<PathBuf> {
    Some(current_dir.join(root))
}

#[cfg(windows)]
fn absolutize_relative_path_root(root: PathBuf, current_dir: &Path) -> Option<PathBuf> {
    use std::path::{Component, Prefix};

    fn disk_prefix(path: &Path) -> Option<u8> {
        match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    Some(drive.to_ascii_uppercase())
                }
                Prefix::Verbatim(_)
                | Prefix::UNC(_, _)
                | Prefix::DeviceNS(_)
                | Prefix::VerbatimUNC(_, _) => None,
            },
            Some(Component::RootDir | Component::CurDir | Component::ParentDir)
            | Some(Component::Normal(_))
            | None => None,
        }
    }

    let absolute = if let Some(root_drive) = disk_prefix(&root) {
        if disk_prefix(current_dir) == Some(root_drive) {
            current_dir.join(root.components().skip(1).collect::<PathBuf>())
        } else {
            std::path::absolute(root).ok()?
        }
    } else {
        current_dir.join(root)
    };

    absolute.is_absolute().then_some(absolute)
}

fn pinned_platform_candidates(
    utility: UtilityType,
    pinned: &PinnedPlatformInstallation,
) -> Vec<Candidate> {
    direct_candidates(&pinned.root, utility, pinned.source)
}

fn select_candidate(
    utility: UtilityType,
    candidates: Vec<Candidate>,
    required: Option<&PlatformVersionRequirement>,
    boundary: Option<&Path>,
) -> Option<UtilityLocation> {
    choose_candidate(
        utility,
        canonical_candidates(utility, candidates, boundary),
        required,
    )
}

fn select_pinned_candidate(
    utility: UtilityType,
    candidates: Vec<Candidate>,
    required: Option<&PlatformVersionRequirement>,
    installation_root: &Path,
) -> Option<UtilityLocation> {
    choose_candidate(
        utility,
        canonical_candidates(utility, candidates, Some(installation_root))
            .into_iter()
            .filter(|candidate| candidate.installation_root.as_path() == installation_root),
        required,
    )
}

fn choose_candidate(
    utility: UtilityType,
    candidates: impl IntoIterator<Item = CanonicalCandidate>,
    required: Option<&PlatformVersionRequirement>,
) -> Option<UtilityLocation> {
    candidates
        .into_iter()
        .filter(|candidate| match (required, candidate.version.as_ref()) {
            (Some(required), Some(UtilityVersion::Platform(version))) => required.matches(version),
            (Some(_), Some(UtilityVersion::Edt(_)) | None) => false,
            (None, Some(UtilityVersion::Platform(_)) | Some(UtilityVersion::Edt(_)) | None) => true,
        })
        .max_by(|left, right| compare_versions(left.version.as_ref(), right.version.as_ref()))
        .map(|chosen| UtilityLocation {
            utility,
            path: chosen.path,
            version: chosen.version,
            source: chosen.source,
            installation_root: chosen.installation_root,
        })
}

fn select_edt_candidate(
    candidates: Vec<Candidate>,
    utility: UtilityType,
    required: Option<&EdtVersion>,
) -> Option<UtilityLocation> {
    choose_candidate(
        utility,
        canonical_candidates(utility, candidates, None)
            .into_iter()
            .filter(
                |candidate| match (required, candidate.source, candidate.version.as_ref()) {
                    (
                        None,
                        ResolutionSource::Explicit
                        | ResolutionSource::DefaultRoot
                        | ResolutionSource::Path,
                        _,
                    ) => true,
                    (Some(_), ResolutionSource::Path, _) => true,
                    (
                        Some(required),
                        ResolutionSource::Explicit | ResolutionSource::DefaultRoot,
                        Some(UtilityVersion::Edt(version)),
                    ) => edt_version_matches(required, version),
                    (
                        Some(_),
                        ResolutionSource::Explicit | ResolutionSource::DefaultRoot,
                        Some(UtilityVersion::Platform(_)) | None,
                    ) => false,
                },
            ),
        None,
    )
}

fn canonical_candidates(
    utility: UtilityType,
    candidates: Vec<Candidate>,
    boundary: Option<&Path>,
) -> Vec<CanonicalCandidate> {
    candidates
        .into_iter()
        .filter_map(|mut candidate| {
            if !is_valid_executable(&candidate.path) {
                return None;
            }
            let path =
                normalize_windows_verbatim_path(&std::fs::canonicalize(&candidate.path).ok()?);
            let installation_root = normalize_windows_verbatim_path(
                &std::fs::canonicalize(installation_root_for_executable(&path)).ok()?,
            );
            if boundary.is_some_and(|boundary| {
                !path.starts_with(boundary) || !installation_root.starts_with(boundary)
            }) {
                return None;
            }
            candidate.version = infer_version(utility, &path);
            Some(CanonicalCandidate {
                path,
                version: candidate.version,
                source: candidate.source,
                installation_root,
            })
        })
        .collect()
}

fn strict_resolution_error(
    utility: UtilityType,
    hint: Option<&Path>,
    candidates: Vec<Candidate>,
    required: Option<&PlatformVersionRequirement>,
    boundary: Option<&Path>,
) -> LocatorError {
    let candidates = canonical_candidates(utility, candidates, boundary);
    if let Some(required) = required {
        let mismatch = candidates
            .iter()
            .filter_map(|candidate| match candidate.version.as_ref() {
                Some(UtilityVersion::Platform(found)) if !required.matches(found) => {
                    Some((candidate, found))
                }
                Some(UtilityVersion::Platform(_)) | Some(UtilityVersion::Edt(_)) | None => None,
            })
            .max_by(|(left, left_version), (right, right_version)| {
                left_version
                    .cmp(right_version)
                    .then_with(|| left.path.cmp(&right.path))
            });
        if let Some((candidate, found)) = mismatch {
            return LocatorError::VersionMismatch {
                utility,
                path: candidate.path.clone(),
                required: required.clone(),
                found: found.clone(),
            };
        }

        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.version.is_none())
        {
            return LocatorError::UnknownVersion {
                utility,
                path: candidate.path.clone(),
                required: required.clone(),
            };
        }
    }

    LocatorError::StrictBoundaryNotFound {
        utility,
        boundary: boundary
            .map(Path::to_path_buf)
            .or_else(|| hint.map(canonical_boundary))
            .unwrap_or_default(),
    }
}

fn canonical_boundary(path: &Path) -> PathBuf {
    nearest_existing_canonical_path(path)
        .map(|canonical| normalize_windows_verbatim_path(&canonical))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn strict_candidate_boundary(hint: &Path) -> PathBuf {
    let canonical = canonical_boundary(hint);
    if hint.is_file()
        || canonical
            .file_name()
            .is_some_and(|name| executable_component_matches(name, "bin"))
    {
        installation_root_for_executable(&canonical)
    } else {
        canonical
    }
}

fn installation_root_for_executable(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(path);
    let is_utility_directory = parent.file_name().is_some_and(|name| {
        executable_component_matches(name, "bin") || executable_component_matches(name, "1cedt")
    });
    if is_utility_directory {
        parent.parent().unwrap_or(parent).to_path_buf()
    } else {
        parent.to_path_buf()
    }
}

fn executable_component_matches(actual: &std::ffi::OsStr, expected: &str) -> bool {
    #[cfg(windows)]
    {
        actual
            .to_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
    }

    #[cfg(not(windows))]
    {
        actual == std::ffi::OsStr::new(expected)
    }
}

fn infer_version(utility: UtilityType, path: &Path) -> Option<UtilityVersion> {
    let installation_root = installation_root_for_executable(path);
    let version_text = installation_root.file_name().and_then(|name| name.to_str());
    match utility {
        UtilityType::V8 | UtilityType::V8C | UtilityType::Ibcmd => version_text
            .and_then(PlatformVersion::parse_strict)
            .map(UtilityVersion::Platform),
        UtilityType::EdtCli => version_text
            .and_then(EdtVersion::parse_lenient)
            .map(UtilityVersion::Edt),
    }
}

fn is_valid_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn executable_name_for(base: &'static str) -> &'static str {
    #[cfg(windows)]
    {
        match base {
            "1cv8" => "1cv8.exe",
            "1cv8c" => "1cv8c.exe",
            "ibcmd" => "ibcmd.exe",
            "1cedtcli" => "1cedtcli.exe",
            _ => base,
        }
    }

    #[cfg(not(windows))]
    {
        base
    }
}

fn default_platform_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        vec![
            PathBuf::from(r"C:\Program Files\1cv8"),
            PathBuf::from(r"C:\Program Files (x86)\1cv8"),
        ]
    }

    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/opt/1cv8/x86_64"),
            PathBuf::from("/opt/1cv8/i386"),
            PathBuf::from("/usr/local/1cv8"),
        ]
    }

    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        vec![PathBuf::from("/opt/1cv8")]
    }
}

fn default_edt_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        vec![PathBuf::from(r"C:\Program Files\1C\1CE\components")]
    }

    #[cfg(target_os = "linux")]
    {
        vec![PathBuf::from("/opt/1C/1CE/components")]
    }

    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        vec![PathBuf::from("/opt/1C/1CE/components")]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_path_roots, normalize_windows_verbatim_path, EdtVersion, Locator, LocatorError,
        PlatformResolutionPolicy, PlatformVersion, PlatformVersionRequirement, ResolutionSource,
        UtilityType, UtilityVersion,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    fn touch_executable(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        make_executable(path);
    }

    fn canonical(path: &Path) -> PathBuf {
        normalize_windows_verbatim_path(&path.canonicalize().expect("canonical path"))
    }

    fn touch_versioned_platform_executable(
        root: &Path,
        version: &str,
        utility: UtilityType,
        use_bin_layout: bool,
    ) -> PathBuf {
        let path = if use_bin_layout {
            root.join(version)
                .join("bin")
                .join(utility.executable_name())
        } else {
            root.join(version).join(utility.executable_name())
        };
        touch_executable(&path);
        path
    }

    fn strict_locator(
        hint: PathBuf,
        version: Option<&str>,
        platform_roots: Vec<PathBuf>,
        path_roots: Vec<PathBuf>,
    ) -> Locator {
        Locator::with_search_roots(
            Some(hint),
            version.and_then(PlatformVersionRequirement::parse),
            PlatformResolutionPolicy::Strict,
            None,
            None,
            platform_roots,
            Vec::new(),
            path_roots,
        )
    }

    fn lenient_locator(
        hint: Option<PathBuf>,
        version: Option<&str>,
        platform_roots: Vec<PathBuf>,
        path_roots: Vec<PathBuf>,
    ) -> Locator {
        Locator::with_search_roots(
            hint,
            version.and_then(PlatformVersionRequirement::parse),
            PlatformResolutionPolicy::Lenient,
            None,
            None,
            platform_roots,
            Vec::new(),
            path_roots,
        )
    }

    #[test]
    fn strict_resolution_does_not_fallback_to_default_or_path_roots() {
        let dir = tempdir().expect("tempdir");
        let explicit = dir.path().join("explicit");
        let default_root = dir.path().join("default");
        let path_root = dir.path().join("path");
        fs::create_dir_all(&explicit).expect("explicit root");
        touch_versioned_platform_executable(&default_root, "8.3.25.1234", UtilityType::V8, true);
        touch_executable(&path_root.join(UtilityType::V8.executable_name()));

        let mut locator =
            strict_locator(explicit.clone(), None, vec![default_root], vec![path_root]);

        assert_eq!(
            locator.locate(UtilityType::V8).expect_err("strict failure"),
            LocatorError::StrictBoundaryNotFound {
                utility: UtilityType::V8,
                boundary: canonical(&explicit),
            }
        );
    }

    #[test]
    fn strict_resolution_reports_exact_version_mismatch() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let binary =
            touch_versioned_platform_executable(&root, "8.3.25.9999", UtilityType::V8, true);
        let mut locator = strict_locator(root, Some("8.3.25.1234"), vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::V8).expect_err("mismatch"),
            LocatorError::VersionMismatch {
                utility: UtilityType::V8,
                path: canonical(&binary),
                required: PlatformVersionRequirement::parse("8.3.25.1234").expect("requirement"),
                found: PlatformVersion::parse_strict("8.3.25.9999").expect("version"),
            }
        );
    }

    #[test]
    fn strict_resolution_reports_prefix_version_mismatch() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let binary =
            touch_versioned_platform_executable(&root, "8.3.24.9999", UtilityType::V8C, false);
        let mut locator = strict_locator(root, Some("8.3.25"), vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::V8C).expect_err("mismatch"),
            LocatorError::VersionMismatch {
                utility: UtilityType::V8C,
                path: canonical(&binary),
                required: PlatformVersionRequirement::parse("8.3.25").expect("requirement"),
                found: PlatformVersion::parse_strict("8.3.24.9999").expect("version"),
            }
        );
    }

    #[test]
    fn strict_resolution_rejects_unknown_version_when_requirement_is_configured() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let binary = root.join("bin").join(UtilityType::Ibcmd.executable_name());
        touch_executable(&binary);
        let mut locator = strict_locator(root, Some("8.3"), vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::Ibcmd).expect_err("unknown"),
            LocatorError::UnknownVersion {
                utility: UtilityType::Ibcmd,
                path: canonical(&binary),
                required: PlatformVersionRequirement::parse("8.3").expect("requirement"),
            }
        );
    }

    #[test]
    fn strict_resolution_does_not_infer_version_from_outer_ancestor() {
        let dir = tempdir().expect("tempdir");
        let installation = dir
            .path()
            .join("8.3.25.1234")
            .join("unversioned-installation");
        let binary = installation
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&binary);
        let mut locator = strict_locator(installation, Some("8.3.25.1234"), vec![], vec![]);

        assert_eq!(
            locator
                .locate(UtilityType::V8)
                .expect_err("outer ancestor must not define installation version"),
            LocatorError::UnknownVersion {
                utility: UtilityType::V8,
                path: canonical(&binary),
                required: PlatformVersionRequirement::parse("8.3.25.1234").expect("requirement"),
            }
        );
    }

    #[test]
    fn strict_resolution_selects_highest_version_and_reports_explicit_source() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let older =
            touch_versioned_platform_executable(&root, "8.3.25.1000", UtilityType::V8, false);
        let wanted =
            touch_versioned_platform_executable(&root, "8.3.25.9999", UtilityType::V8, true);
        let mut locator = strict_locator(root, Some("8.3.25"), vec![], vec![]);

        let location = locator.locate(UtilityType::V8).expect("locate");

        assert_ne!(location.path, older);
        assert_eq!(location.path, canonical(&wanted));
        assert_eq!(location.source, ResolutionSource::Explicit);
        assert_eq!(
            location.installation_root,
            canonical(
                wanted
                    .parent()
                    .and_then(Path::parent)
                    .expect("installation root")
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn strict_prefix_selects_highest_across_direct_and_versioned_candidates() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let lower =
            touch_versioned_platform_executable(&root, "8.3.25.1000", UtilityType::V8, true);
        let higher =
            touch_versioned_platform_executable(&root, "8.3.25.9999", UtilityType::V8, true);
        let direct_alias = root.join("bin").join(UtilityType::V8.executable_name());
        fs::create_dir_all(direct_alias.parent().expect("direct parent")).expect("direct parent");
        std::os::unix::fs::symlink(&lower, &direct_alias).expect("direct alias");
        let mut locator = strict_locator(root, Some("8.3.25"), vec![], vec![]);

        let location = locator.locate(UtilityType::V8).expect("highest candidate");

        assert_eq!(location.path, canonical(&higher));
        assert_eq!(
            location.version,
            Some(UtilityVersion::Platform(
                PlatformVersion::parse_strict("8.3.25.9999").expect("version")
            ))
        );
    }

    #[test]
    fn platform_resolution_pins_siblings_to_first_installation() {
        let dir = tempdir().expect("tempdir");
        let explicit_root = dir.path().join("explicit");
        let fallback_root = dir.path().join("fallback");
        let v8 = touch_versioned_platform_executable(
            &explicit_root,
            "8.3.25.1234",
            UtilityType::V8,
            true,
        );
        touch_versioned_platform_executable(&fallback_root, "8.3.25.1234", UtilityType::V8C, true);
        let mut locator = strict_locator(
            explicit_root,
            Some("8.3.25.1234"),
            vec![fallback_root],
            vec![],
        );
        let first = locator.locate(UtilityType::V8).expect("first utility");

        assert_eq!(first.path, canonical(&v8));
        assert_eq!(
            locator
                .locate(UtilityType::V8C)
                .expect_err("missing sibling"),
            LocatorError::MissingSibling {
                utility: UtilityType::V8C,
                installation_root: first.installation_root,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn pinned_resolution_rejects_sibling_symlink_to_nested_installation() {
        let dir = tempdir().expect("tempdir");
        let explicit_root = dir.path().join("explicit");
        let installation = explicit_root.join("8.3.25.1234");
        let v8 = installation
            .join("bin")
            .join(UtilityType::V8.executable_name());
        let nested_installation = installation.join("nested").join("8.3.25.1234");
        let nested_v8c = nested_installation
            .join("bin")
            .join(UtilityType::V8C.executable_name());
        let sibling_alias = installation
            .join("bin")
            .join(UtilityType::V8C.executable_name());
        touch_executable(&v8);
        touch_executable(&nested_v8c);
        std::os::unix::fs::symlink(&nested_v8c, &sibling_alias).expect("sibling symlink");
        let mut locator = strict_locator(explicit_root, Some("8.3.25.1234"), vec![], vec![]);
        let first = locator.locate(UtilityType::V8).expect("first utility");

        assert_eq!(
            locator
                .locate(UtilityType::V8C)
                .expect_err("nested installation must not satisfy pin"),
            LocatorError::MissingSibling {
                utility: UtilityType::V8C,
                installation_root: first.installation_root,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn strict_resolution_rejects_version_directory_symlink_outside_boundary() {
        let dir = tempdir().expect("tempdir");
        let explicit_root = dir.path().join("explicit");
        let outside_root = dir.path().join("outside").join("8.3.25.1234");
        let outside_binary = outside_root.join("bin").join("1cv8");
        touch_executable(&outside_binary);
        fs::create_dir_all(&explicit_root).expect("explicit root");
        std::os::unix::fs::symlink(&outside_root, explicit_root.join("8.3.25.1234"))
            .expect("version symlink");
        let mut locator =
            strict_locator(explicit_root.clone(), Some("8.3.25.1234"), vec![], vec![]);

        assert_eq!(
            locator
                .locate(UtilityType::V8)
                .expect_err("boundary escape"),
            LocatorError::StrictBoundaryNotFound {
                utility: UtilityType::V8,
                boundary: canonical(&explicit_root),
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn strict_resolution_recomputes_version_after_canonicalizing_alias() {
        let dir = tempdir().expect("tempdir");
        let explicit_root = dir.path().join("explicit");
        let actual_root = explicit_root.join("8.3.26.9999");
        let actual_binary = actual_root.join("bin").join("1cv8");
        touch_executable(&actual_binary);
        std::os::unix::fs::symlink(&actual_root, explicit_root.join("8.3.25.1234"))
            .expect("version alias");
        let mut locator = strict_locator(explicit_root, Some("8.3.25.1234"), vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::V8).expect_err("alias mismatch"),
            LocatorError::VersionMismatch {
                utility: UtilityType::V8,
                path: canonical(&actual_binary),
                required: PlatformVersionRequirement::parse("8.3.25.1234").expect("requirement"),
                found: PlatformVersion::parse_strict("8.3.26.9999").expect("actual version"),
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn strict_file_symlink_hint_resolves_sibling_in_canonical_installation() {
        let dir = tempdir().expect("tempdir");
        let installation = dir.path().join("actual").join("8.3.25.1234");
        let v8 = installation
            .join("bin")
            .join(UtilityType::V8.executable_name());
        let v8c = installation
            .join("bin")
            .join(UtilityType::V8C.executable_name());
        touch_executable(&v8);
        touch_executable(&v8c);
        let aliases = dir.path().join("aliases");
        fs::create_dir_all(&aliases).expect("alias root");
        let hint = aliases.join(UtilityType::V8.executable_name());
        std::os::unix::fs::symlink(&v8, &hint).expect("file symlink");
        let mut missing_locator = strict_locator(hint.clone(), Some("8.3.25.1234"), vec![], vec![]);
        assert_eq!(
            missing_locator
                .locate(UtilityType::Ibcmd)
                .expect_err("missing canonical sibling"),
            LocatorError::StrictBoundaryNotFound {
                utility: UtilityType::Ibcmd,
                boundary: canonical(&installation),
            }
        );
        let mut locator = strict_locator(hint, Some("8.3.25.1234"), vec![], vec![]);

        let location = locator.locate(UtilityType::V8C).expect("canonical sibling");

        assert_eq!(location.path, canonical(&v8c));
        assert_eq!(location.installation_root, canonical(&installation));
        assert_eq!(location.source, ResolutionSource::Explicit);
    }

    #[cfg(unix)]
    #[test]
    fn strict_cache_rejects_executable_replaced_by_outbound_symlink() {
        let dir = tempdir().expect("tempdir");
        let installation = dir.path().join("platform").join("8.3.25.1234");
        let binary = installation
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&binary);
        let mut locator = strict_locator(installation.clone(), Some("8.3.25.1234"), vec![], vec![]);
        let first = locator.locate(UtilityType::V8).expect("initial location");
        let outside = dir
            .path()
            .join("outside")
            .join("8.3.25.1234")
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&outside);
        fs::remove_file(&binary).expect("replace cached executable");
        std::os::unix::fs::symlink(&outside, &binary).expect("outbound symlink");

        assert_eq!(
            locator
                .locate(UtilityType::V8)
                .expect_err("cached path must be revalidated"),
            LocatorError::MissingSibling {
                utility: UtilityType::V8,
                installation_root: first.installation_root,
            }
        );
    }

    #[test]
    fn empty_path_component_is_captured_as_current_directory() {
        let current = PathBuf::from("/captured/current-directory");

        assert_eq!(
            normalize_path_roots(
                vec![PathBuf::new(), PathBuf::from("/configured/bin")],
                &current,
            ),
            vec![current, PathBuf::from("/configured/bin")]
        );
    }

    #[test]
    fn relative_path_components_are_absolutized_against_captured_current_directory() {
        #[cfg(windows)]
        let captured = PathBuf::from(r"C:\captured\current-directory");
        #[cfg(windows)]
        let absolute = PathBuf::from(r"C:\absolute\bin");
        #[cfg(not(windows))]
        let captured = PathBuf::from("/captured/current-directory");
        #[cfg(not(windows))]
        let absolute = PathBuf::from("/absolute/bin");

        assert_eq!(
            normalize_path_roots(
                vec![
                    PathBuf::from("relative/bin"),
                    PathBuf::from("."),
                    absolute.clone(),
                ],
                &captured,
            ),
            vec![captured.join("relative/bin"), captured, absolute]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_relative_path_components_use_captured_drive_directories() {
        let captured = PathBuf::from(r"C:\captured\current-directory");
        let other_drive = PathBuf::from(r"D:other");
        let resolved_other_drive = std::path::absolute(&other_drive)
            .expect("Windows resolves a drive-relative path using that drive's current directory");

        assert_eq!(
            normalize_path_roots(vec![PathBuf::from(r"C:tools"), other_drive], &captured),
            vec![captured.join("tools"), resolved_other_drive]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_executable_hint_identity_is_ascii_case_insensitive() {
        let dir = tempdir().expect("tempdir");
        let hint = dir.path().join("1CV8.EXE");
        touch_executable(&hint);

        let candidates = super::explicit_direct_candidates(
            &hint,
            UtilityType::V8,
            super::FileHintSiblingResolution::Lexical,
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, canonical(&hint));
    }

    #[cfg(windows)]
    #[test]
    fn windows_installation_root_layout_components_are_ascii_case_insensitive() {
        let version_root = PathBuf::from(r"C:\Program Files\1cv8\8.3.25.1234");

        assert_eq!(
            super::installation_root_for_executable(&version_root.join(r"BIN\1CV8.EXE")),
            version_root
        );
        assert_eq!(
            super::installation_root_for_executable(&version_root.join(r"1CEDT\1CEDTCLI.EXE")),
            version_root
        );
    }

    #[test]
    fn lenient_path_resolution_ignores_version_for_direct_hint() {
        let dir = tempdir().expect("tempdir");
        let explicit_root = dir.path().join("explicit");
        let default_root = dir.path().join("default");
        let explicit = explicit_root
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&explicit);
        touch_versioned_platform_executable(&default_root, "8.3.25.1234", UtilityType::V8, true);
        let mut locator = lenient_locator(
            Some(explicit_root),
            Some("8.3.25.1234"),
            vec![default_root],
            vec![],
        );

        let location = locator.locate(UtilityType::V8).expect("direct hint");

        assert_eq!(location.path, canonical(&explicit));
        assert_eq!(location.version, None);
        assert_eq!(location.source, ResolutionSource::Explicit);
    }

    #[test]
    fn lenient_path_resolution_ignores_version_for_versioned_hint() {
        let dir = tempdir().expect("tempdir");
        let explicit_root = dir.path().join("explicit");
        let default_root = dir.path().join("default");
        let explicit = touch_versioned_platform_executable(
            &explicit_root,
            "8.3.24.9999",
            UtilityType::V8,
            true,
        );
        touch_versioned_platform_executable(&default_root, "8.3.25.1234", UtilityType::V8, true);
        let mut locator = lenient_locator(
            Some(explicit_root),
            Some("8.3.25.1234"),
            vec![default_root],
            vec![],
        );

        let location = locator
            .locate(UtilityType::V8)
            .expect("version ignored for explicit path");

        assert_eq!(location.path, canonical(&explicit));
        assert_eq!(
            location.version,
            Some(UtilityVersion::Platform(
                PlatformVersion::parse_strict("8.3.24.9999").expect("version")
            ))
        );
        assert_eq!(location.source, ResolutionSource::Explicit);
    }

    #[test]
    fn lenient_path_resolution_does_not_fallback_to_default_roots() {
        let dir = tempdir().expect("tempdir");
        let explicit_root = dir.path().join("explicit");
        let default_root = dir.path().join("default");
        let v8 = touch_versioned_platform_executable(
            &explicit_root,
            "8.3.25.1234",
            UtilityType::V8,
            true,
        );
        touch_versioned_platform_executable(&default_root, "8.3.25.1234", UtilityType::V8C, true);
        let mut locator = lenient_locator(
            Some(explicit_root),
            Some("8.3.25.1234"),
            vec![default_root],
            vec![],
        );

        let first = locator.locate(UtilityType::V8).expect("explicit v8");

        assert_eq!(first.path, canonical(&v8));
        assert_eq!(first.source, ResolutionSource::Explicit);
        assert_eq!(
            locator
                .locate(UtilityType::V8C)
                .expect_err("configured path must not fallback"),
            LocatorError::NotFound(UtilityType::V8C)
        );
    }

    #[cfg(unix)]
    #[test]
    fn lenient_file_symlink_hint_cache_revalidates_current_hint() {
        let dir = tempdir().expect("tempdir");
        let actual = dir
            .path()
            .join("actual")
            .join("8.3.25.1234")
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&actual);
        let aliases = dir.path().join("aliases");
        fs::create_dir_all(&aliases).expect("aliases");
        let hint = aliases.join(UtilityType::V8.executable_name());
        std::os::unix::fs::symlink(&actual, &hint).expect("hint symlink");
        let mut locator = lenient_locator(Some(hint.clone()), None, vec![], vec![]);

        let first = locator.locate(UtilityType::V8).expect("initial hint");
        fs::remove_file(&hint).expect("remove hint");

        assert_eq!(first.path, canonical(&actual));
        assert_eq!(
            locator
                .locate(UtilityType::V8)
                .expect_err("current hint is gone"),
            LocatorError::NotFound(UtilityType::V8)
        );
    }

    #[test]
    fn version_only_resolution_uses_default_roots_and_path_with_version_filter() {
        let dir = tempdir().expect("tempdir");
        let default_root = dir.path().join("default");
        let path_root = dir.path().join("path-bin");
        touch_versioned_platform_executable(&default_root, "8.3.24.9999", UtilityType::V8, true);
        let wanted = touch_versioned_platform_executable(
            &default_root,
            "8.3.25.1234",
            UtilityType::V8,
            true,
        );
        touch_executable(&path_root.join(UtilityType::V8.executable_name()));
        let mut locator = lenient_locator(
            None,
            Some("8.3.25.1234"),
            vec![default_root],
            vec![path_root],
        );

        let location = locator.locate(UtilityType::V8).expect("versioned utility");

        assert_eq!(location.path, canonical(&wanted));
        assert_eq!(location.source, ResolutionSource::DefaultRoot);
    }

    #[test]
    fn strict_without_path_uses_default_roots_with_version_filter() {
        let dir = tempdir().expect("tempdir");
        let default_root = dir.path().join("default");
        let wanted = touch_versioned_platform_executable(
            &default_root,
            "8.3.25.1234",
            UtilityType::V8,
            true,
        );
        let mut locator = Locator::with_search_roots(
            None,
            PlatformVersionRequirement::parse("8.3.25.1234"),
            PlatformResolutionPolicy::Strict,
            None,
            None,
            vec![default_root],
            vec![],
            vec![],
        );

        let location = locator.locate(UtilityType::V8).expect("versioned utility");

        assert_eq!(location.path, canonical(&wanted));
        assert_eq!(location.source, ResolutionSource::DefaultRoot);
    }

    #[test]
    fn lenient_resolution_without_hint_uses_injected_path_roots_and_reports_path_source() {
        let dir = tempdir().expect("tempdir");
        let path_root = dir.path().join("path-bin");
        let binary = path_root.join(UtilityType::Ibcmd.executable_name());
        touch_executable(&binary);
        let mut locator = lenient_locator(None, None, vec![], vec![path_root.clone()]);

        let location = locator.locate(UtilityType::Ibcmd).expect("PATH utility");

        assert_eq!(location.path, canonical(&binary));
        assert_eq!(location.source, ResolutionSource::Path);
        assert_eq!(location.installation_root, canonical(&path_root));
    }

    #[test]
    fn parse_strict_platform_version_requires_four_parts() {
        assert!(PlatformVersion::parse_strict("8.3.25").is_none());
        assert!(PlatformVersion::parse_strict("8.3.25.1234").is_some());
    }

    #[test]
    fn parse_platform_version_requirement_accepts_prefix_or_exact_version() {
        let minor_prefix = PlatformVersionRequirement::parse("8.3").expect("minor prefix");
        assert_eq!(minor_prefix.to_string(), "8.3");
        assert!(
            minor_prefix.matches(&PlatformVersion::parse_strict("8.3.25.1234").expect("version"))
        );
        assert!(
            !minor_prefix.matches(&PlatformVersion::parse_strict("8.4.25.1234").expect("version"))
        );

        let prefix = PlatformVersionRequirement::parse("8.3.25").expect("prefix");
        assert_eq!(prefix.to_string(), "8.3.25");
        assert!(prefix.matches(&PlatformVersion::parse_strict("8.3.25.1234").expect("version")));
        assert!(!prefix.matches(&PlatformVersion::parse_strict("8.3.24.9999").expect("version")));

        let exact = PlatformVersionRequirement::parse("8.3.25.1234").expect("exact");
        assert_eq!(exact.to_string(), "8.3.25.1234");
        assert!(exact.matches(&PlatformVersion::parse_strict("8.3.25.1234").expect("version")));
        assert!(!exact.matches(&PlatformVersion::parse_strict("8.3.25.9999").expect("version")));
    }

    #[test]
    fn parse_lenient_edt_version_extracts_numeric_tokens() {
        let version = EdtVersion::parse_lenient("1c-edt-2025.1.0+656-x86_64").expect("version");
        assert_eq!(version.parts, vec![1, 2025, 1, 0, 656, 86, 64]);
    }

    #[cfg(unix)]
    #[test]
    fn explicit_file_hint_supports_sibling_binary_lookup() {
        let dir = tempdir().expect("tempdir");
        let v8 = dir.path().join("1cv8");
        let v8c = dir.path().join("1cv8c");
        touch_executable(&v8);
        touch_executable(&v8c);

        let mut locator = Locator::with_roots(Some(v8.clone()), None, None, None, vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::V8C).expect("locate").path,
            canonical(&v8c)
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_file_alias_uses_lexical_name_for_utility_identity() {
        let dir = tempdir().expect("tempdir");
        let actual = dir.path().join("actual").join("bin").join("1cv8-real");
        touch_executable(&actual);
        let aliases = dir.path().join("aliases");
        fs::create_dir_all(&aliases).expect("aliases");
        let hint = aliases.join(UtilityType::V8.executable_name());
        std::os::unix::fs::symlink(&actual, &hint).expect("file alias");
        let mut locator = Locator::with_roots(Some(hint), None, None, None, vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::V8).expect("aliased V8").path,
            canonical(&actual)
        );
    }

    #[cfg(unix)]
    #[test]
    fn fallback_file_symlink_hint_resolves_lexical_sibling() {
        let dir = tempdir().expect("tempdir");
        let actual_v8 = dir
            .path()
            .join("actual")
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&actual_v8);
        let aliases = dir.path().join("aliases");
        fs::create_dir_all(&aliases).expect("aliases");
        let hint = aliases.join(UtilityType::V8.executable_name());
        let lexical_v8c = aliases.join(UtilityType::V8C.executable_name());
        std::os::unix::fs::symlink(&actual_v8, &hint).expect("V8 alias");
        touch_executable(&lexical_v8c);
        let mut locator = Locator::with_roots(Some(hint), None, None, None, vec![], vec![]);

        let location = locator
            .locate(UtilityType::V8C)
            .expect("lexical fallback sibling");

        assert_eq!(location.path, canonical(&lexical_v8c));
        assert_eq!(location.source, ResolutionSource::Explicit);
    }

    #[cfg(unix)]
    #[test]
    fn explicit_directory_hint_checks_direct_and_bin_layouts() {
        let dir = tempdir().expect("tempdir");
        let install_dir = dir.path().join("install");
        let binary = install_dir.join("bin").join("1cv8");
        touch_executable(&binary);

        let mut locator = Locator::with_roots(Some(install_dir), None, None, None, vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::V8).expect("locate").path,
            canonical(&binary)
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_root_hint_searches_versioned_children() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform-root");
        let version = PlatformVersionRequirement::parse("8.3.25.1234").expect("version");
        let thin = root.join("8.3.25.1234").join("bin").join("1cv8c");
        touch_executable(&thin);

        let mut locator =
            Locator::with_roots(Some(root), Some(version), None, None, vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::V8C).expect("locate").path,
            canonical(&thin)
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_root_hint_searches_versioned_children_for_ibcmd() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform-root");
        let version = PlatformVersionRequirement::parse("8.3.27.1789").expect("version");
        let ibcmd = root.join("8.3.27.1789").join("bin").join("ibcmd");
        touch_executable(&ibcmd);

        let mut locator =
            Locator::with_roots(Some(root), Some(version), None, None, vec![], vec![]);

        assert_eq!(
            locator.locate(UtilityType::Ibcmd).expect("locate").path,
            canonical(&ibcmd)
        );
    }

    #[cfg(unix)]
    #[test]
    fn platform_search_prefers_exact_version_match() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let wanted = root.join("8.3.25.1234").join("1cv8");
        let other = root.join("8.3.24.9999").join("1cv8");
        touch_executable(&wanted);
        touch_executable(&other);

        let mut locator = Locator::with_roots(
            None,
            Some(PlatformVersionRequirement::parse("8.3.25.1234").expect("version")),
            None,
            None,
            vec![root],
            vec![],
        );

        let location = locator.locate(UtilityType::V8).expect("locate");
        assert_eq!(location.path, canonical(&wanted));
        assert!(matches!(
            location.version,
            Some(UtilityVersion::Platform(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn platform_search_prefix_picks_highest_matching_build() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let wanted = root.join("8.3.25.9999").join("1cv8");
        let older = root.join("8.3.25.1234").join("1cv8");
        let other_patch = root.join("8.3.24.9999").join("1cv8");
        touch_executable(&wanted);
        touch_executable(&older);
        touch_executable(&other_patch);

        let mut locator = Locator::with_roots(
            None,
            Some(PlatformVersionRequirement::parse("8.3.25").expect("version")),
            None,
            None,
            vec![root],
            vec![],
        );

        let location = locator.locate(UtilityType::V8).expect("locate");
        assert_eq!(location.path, canonical(&wanted));
        assert_eq!(
            location.version,
            Some(UtilityVersion::Platform(
                PlatformVersion::parse_strict("8.3.25.9999").expect("version")
            ))
        );
    }

    #[cfg(unix)]
    #[test]
    fn platform_search_minor_prefix_picks_highest_matching_version() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let wanted = root.join("8.3.27.1789").join("1cv8");
        let older_build = root.join("8.3.27.1000").join("1cv8");
        let older_patch = root.join("8.3.20.9999").join("1cv8");
        let other_minor = root.join("8.4.1.1").join("1cv8");
        touch_executable(&wanted);
        touch_executable(&older_build);
        touch_executable(&older_patch);
        touch_executable(&other_minor);

        let mut locator = Locator::with_roots(
            None,
            Some(PlatformVersionRequirement::parse("8.3").expect("version")),
            None,
            None,
            vec![root],
            vec![],
        );

        let location = locator.locate(UtilityType::V8).expect("locate");
        assert_eq!(location.path, canonical(&wanted));
        assert_eq!(
            location.version,
            Some(UtilityVersion::Platform(
                PlatformVersion::parse_strict("8.3.27.1789").expect("version")
            ))
        );
    }

    #[cfg(unix)]
    #[test]
    fn platform_version_matrix_applies_to_all_platform_utilities() {
        for utility in [UtilityType::V8, UtilityType::V8C, UtilityType::Ibcmd] {
            let dir = tempdir().expect("tempdir");
            let root = dir
                .path()
                .join(format!("platform-{}", utility.executable_name()));

            let exact = touch_versioned_platform_executable(&root, "8.3.27.1789", utility, true);
            let _older_build =
                touch_versioned_platform_executable(&root, "8.3.27.1000", utility, false);
            let patch_best =
                touch_versioned_platform_executable(&root, "8.3.20.9999", utility, false);
            let _patch_older =
                touch_versioned_platform_executable(&root, "8.3.20.1000", utility, true);
            let _other_minor =
                touch_versioned_platform_executable(&root, "8.4.1.1", utility, false);

            let mut exact_locator = Locator::with_roots(
                None,
                Some(PlatformVersionRequirement::parse("8.3.27.1789").expect("version")),
                None,
                None,
                vec![root.clone()],
                vec![],
            );
            assert_eq!(
                exact_locator.locate(utility).expect("exact").path,
                canonical(&exact)
            );

            let mut patch_locator = Locator::with_roots(
                None,
                Some(PlatformVersionRequirement::parse("8.3.20").expect("version")),
                None,
                None,
                vec![root.clone()],
                vec![],
            );
            assert_eq!(
                patch_locator.locate(utility).expect("patch").path,
                canonical(&patch_best)
            );

            let mut minor_locator = Locator::with_roots(
                None,
                Some(PlatformVersionRequirement::parse("8.3").expect("version")),
                None,
                None,
                vec![root],
                vec![],
            );
            assert_eq!(
                minor_locator.locate(utility).expect("minor").path,
                canonical(&exact)
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn default_platform_roots_match_linux_install_contract() {
        assert_eq!(
            super::default_platform_roots(),
            vec![
                PathBuf::from("/opt/1cv8/x86_64"),
                PathBuf::from("/opt/1cv8/i386"),
                PathBuf::from("/usr/local/1cv8"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn edt_search_picks_highest_lenient_version() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("edt");
        let newer = root
            .join("1c-edt-2025.1.0+656-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        let older = root
            .join("1c-edt-2024.2.0+100-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        touch_executable(&newer);
        touch_executable(&older);

        let mut locator = Locator::with_roots(None, None, None, None, vec![], vec![root]);

        assert_eq!(
            locator.locate(UtilityType::EdtCli).expect("locate").path,
            canonical(&newer)
        );
    }

    #[cfg(unix)]
    #[test]
    fn invalidates_broken_cache_entries_and_relocates() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let version = PlatformVersionRequirement::parse("8.3.25.1234").expect("version");
        let first = root.join("8.3.25.1234").join("1cv8");
        touch_executable(&first);

        let mut locator =
            Locator::with_roots(None, Some(version), None, None, vec![root.clone()], vec![]);
        let first_path = locator.locate(UtilityType::V8).expect("first").path;
        assert_eq!(first_path, canonical(&first));

        fs::remove_file(&first).expect("remove");
        let second = root.join("8.3.25.1234").join("bin").join("1cv8");
        touch_executable(&second);

        let second_path = locator.locate(UtilityType::V8).expect("second").path;
        assert_eq!(second_path, canonical(&second));
    }

    #[test]
    fn utility_location_can_infer_platform_version_from_path() {
        let path = PathBuf::from("/opt/1cv8/x86_64/8.3.25.1234/1cv8");
        let version = super::infer_version(UtilityType::V8, &path);

        assert!(matches!(version, Some(UtilityVersion::Platform(_))));
    }

    #[cfg(unix)]
    #[test]
    fn edt_search_accepts_version_prefix_hint() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("edt");
        let wanted = root
            .join("1c-edt-2025.2.3+30-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        let other = root
            .join("1c-edt-2025.1.9+100-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        touch_executable(&wanted);
        touch_executable(&other);

        let mut locator = Locator::with_roots(
            None,
            None,
            None,
            Some(EdtVersion::parse_lenient("1c-edt-2025.2.3").expect("version")),
            vec![],
            vec![root],
        );

        assert_eq!(
            locator.locate(UtilityType::EdtCli).expect("locate").path,
            canonical(&wanted)
        );
    }

    #[cfg(unix)]
    #[test]
    fn edt_version_requirement_rejects_canonical_symlink_mismatch() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("edt");
        let actual = dir.path().join("actual").join("1c-edt-2024.1.0+10-x86_64");
        let binary = actual
            .join("1cedt")
            .join(UtilityType::EdtCli.executable_name());
        touch_executable(&binary);
        fs::create_dir_all(&root).expect("EDT root");
        std::os::unix::fs::symlink(&actual, root.join("1c-edt-2025.2.3+30-x86_64"))
            .expect("version alias");
        let mut locator = Locator::with_roots(
            None,
            None,
            None,
            EdtVersion::parse_lenient("2025.2.3"),
            vec![],
            vec![root],
        );

        assert_eq!(
            locator
                .locate(UtilityType::EdtCli)
                .expect_err("canonical EDT version mismatch"),
            LocatorError::NotFound(UtilityType::EdtCli)
        );
    }

    #[cfg(unix)]
    #[test]
    fn edt_version_requirement_accepts_matching_canonical_symlink_target() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("edt");
        let actual = dir.path().join("actual").join("1c-edt-2025.2.3+30-x86_64");
        let binary = actual
            .join("1cedt")
            .join(UtilityType::EdtCli.executable_name());
        touch_executable(&binary);
        fs::create_dir_all(&root).expect("EDT root");
        std::os::unix::fs::symlink(&actual, root.join("1c-edt-2024.1.0+10-x86_64"))
            .expect("version alias");
        let mut locator = Locator::with_roots(
            None,
            None,
            None,
            EdtVersion::parse_lenient("2025.2.3"),
            vec![],
            vec![root],
        );

        let location = locator
            .locate(UtilityType::EdtCli)
            .expect("canonical EDT version match");

        assert_eq!(location.path, canonical(&binary));
        assert!(matches!(location.version, Some(UtilityVersion::Edt(_))));
    }

    #[cfg(unix)]
    #[test]
    fn edt_search_accepts_plain_numeric_version_hint() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("edt");
        let wanted = root
            .join("1c-edt-2025.2.3+30-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        let other = root
            .join("1c-edt-2025.1.9+100-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        touch_executable(&wanted);
        touch_executable(&other);

        let mut locator = Locator::with_roots(
            None,
            None,
            None,
            Some(EdtVersion::parse_lenient("2025.2.3").expect("version")),
            vec![],
            vec![root],
        );

        assert_eq!(
            locator.locate(UtilityType::EdtCli).expect("locate").path,
            canonical(&wanted)
        );
    }
}
