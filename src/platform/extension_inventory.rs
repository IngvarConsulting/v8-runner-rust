//! Parser for the extension inventory `ibcmd` prints as text.
//!
//! `ibcmd config extension list` and `... info` have no machine-readable output mode, so
//! the shape below is measured from 8.3.27.2074 rather than documented: records are
//! separated by a blank line, each line is a key padded with spaces followed by `: `,
//! string values are quoted, enums and booleans are bare, and an unset field is empty.

use crate::domain::extensions::InstalledExtension;

/// Parses the inventory text into one record per extension.
///
/// A record missing a required field is a refusal, not a record with a guessed value:
/// the caller would otherwise read a default as an observation.
pub fn parse_extension_inventory(output: &str) -> Result<Vec<InstalledExtension>, String> {
    let mut extensions = Vec::new();
    for block in output.split("\n\n") {
        let fields = parse_block(block)?;
        if fields.is_empty() {
            continue;
        }
        extensions.push(build_extension(&fields)?);
    }
    Ok(extensions)
}

fn parse_block(block: &str) -> Result<Vec<(String, String)>, String> {
    let mut fields = Vec::new();
    for line in block.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("extension inventory line has no key: {line:?}"));
        };
        fields.push((key.trim().to_owned(), unquote(value.trim())));
    }
    Ok(fields)
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
        .to_owned()
}

fn build_extension(fields: &[(String, String)]) -> Result<InstalledExtension, String> {
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let required = |name: &str| {
        field(name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("extension inventory record has no {name}"))
    };
    let optional = |name: &str| {
        field(name)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let flag = |name: &str| match field(name) {
        Some("yes") => Ok(true),
        Some("no") => Ok(false),
        Some(other) => Err(format!(
            "extension inventory {name} is not yes/no: {other:?}"
        )),
        None => Err(format!("extension inventory record has no {name}")),
    };

    Ok(InstalledExtension {
        name: required("name")?.to_owned(),
        version: optional("version"),
        active: flag("active")?,
        purpose: required("purpose")?.to_owned(),
        safe_mode: flag("safe-mode")?,
        security_profile_name: optional("security-profile-name"),
        unsafe_action_protection: flag("unsafe-action-protection")?,
        used_in_distributed_infobase: flag("used-in-distributed-infobase")?,
        scope: required("scope")?.to_owned(),
        hash_sum: required("hash-sum")?.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_extension_inventory;

    /// Captured verbatim from `ibcmd config extension list` on 8.3.27.2074 against a file
    /// infobase holding two extensions.
    const MEASURED_LIST: &str = "name                         : \"Вторая\"\nversion                      : \nactive                       : yes\npurpose                      : patch\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"HFmRgcnNuCfOYQLhjscUgqE7ZJI=\"\n\nname                         : \"Проба\"\nversion                      : \nactive                       : yes\npurpose                      : add-on\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"9hfFb6YVX2OwLKZaL1L69Eq0Vrg=\"\n\n\n";

    #[test]
    fn parses_the_measured_two_extension_listing() {
        let extensions = parse_extension_inventory(MEASURED_LIST).expect("inventory");

        assert_eq!(extensions.len(), 2);
        let first = &extensions[0];
        assert_eq!(first.name, "Вторая");
        assert_eq!(first.purpose, "patch");
        assert_eq!(first.scope, "infobase");
        assert_eq!(first.hash_sum, "HFmRgcnNuCfOYQLhjscUgqE7ZJI=");
        assert!(first.active);
        assert!(first.safe_mode);
        assert!(first.unsafe_action_protection);
        assert!(!first.used_in_distributed_infobase);
        // Empty in the platform output means unset, not an empty string.
        assert!(first.version.is_none());
        assert!(first.security_profile_name.is_none());
        assert_eq!(extensions[1].name, "Проба");
        assert_eq!(extensions[1].purpose, "add-on");
    }

    #[test]
    fn empty_output_is_an_empty_inventory_not_a_failure() {
        assert!(parse_extension_inventory("").expect("inventory").is_empty());
        assert!(parse_extension_inventory("\n\n\n")
            .expect("inventory")
            .is_empty());
    }

    #[test]
    fn a_record_missing_a_required_field_is_refused() {
        let truncated =
            "name                         : \"X\"\nactive                       : yes\n";

        let error = parse_extension_inventory(truncated).expect_err("refusal");

        assert!(error.contains("purpose"), "{error}");
    }

    #[test]
    fn a_non_boolean_flag_is_refused_instead_of_defaulting() {
        let odd = MEASURED_LIST.replace(
            "active                       : yes",
            "active                       : maybe",
        );

        let error = parse_extension_inventory(&odd).expect_err("refusal");

        assert!(error.contains("active"), "{error}");
    }

    #[test]
    fn a_line_without_a_key_is_refused() {
        let error = parse_extension_inventory("name").expect_err("refusal");

        assert!(error.contains("no key"), "{error}");
    }
}
