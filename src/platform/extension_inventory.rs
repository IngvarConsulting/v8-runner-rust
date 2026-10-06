//! Parser for the extension inventory `ibcmd` prints as text.
//!
//! `ibcmd config extension list` and `... info` have no machine-readable output mode, so
//! the shape below is measured from 8.3.27.2074 rather than documented: records are
//! separated by a blank line, each line is a key padded with spaces followed by `: `,
//! string values are quoted, enums and booleans are bare, and an unset field is empty.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::domain::extensions::InstalledExtension;

/// Properties found in the configuration descriptor exported from a saved DB CFE.
#[derive(Debug, PartialEq, Eq)]
pub struct AppliedExtensionDescriptor {
    pub name: String,
    pub version: Option<String>,
    pub purpose: String,
    pub name_prefix: String,
}

/// Read only direct `Configuration/Properties` fields. A missing `NamePrefix` is
/// unknown, while `<NamePrefix/>` is a known empty prefix.
pub fn read_applied_extension_descriptor(
    path: &Path,
) -> Result<AppliedExtensionDescriptor, String> {
    let file =
        File::open(path).map_err(|error| format!("cannot open exported descriptor: {error}"))?;
    let mut reader = Reader::from_reader(BufReader::new(file));
    reader.config_mut().trim_text(true);
    let mut event_buffer = Vec::new();
    let mut path_stack: Vec<Vec<u8>> = Vec::new();
    let mut properties_seen = false;
    let mut field_text: Option<(&'static str, String)> = None;
    let mut name = None;
    let mut version = None;
    let mut purpose = None;
    let mut name_prefix = None;

    loop {
        let event = reader
            .read_event_into(&mut event_buffer)
            .map_err(|error| format!("invalid exported descriptor XML: {error}"))?;
        match event {
            Event::Start(tag) => {
                path_stack.push(tag.local_name().as_ref().to_vec());
                if is_properties_path(&path_stack) {
                    if properties_seen {
                        return Err("exported descriptor has duplicate Properties".to_owned());
                    }
                    properties_seen = true;
                }
                if let Some(field) = descriptor_field(&path_stack) {
                    field_text = Some((field, String::new()));
                } else if field_text.is_some() {
                    return Err("exported descriptor has a nested property value".to_owned());
                }
            }
            Event::Empty(tag) => {
                path_stack.push(tag.local_name().as_ref().to_vec());
                if let Some(field) = descriptor_field(&path_stack) {
                    set_descriptor_field(
                        field,
                        String::new(),
                        &mut name,
                        &mut version,
                        &mut purpose,
                        &mut name_prefix,
                    )?;
                } else if field_text.is_some() {
                    return Err("exported descriptor has a nested property value".to_owned());
                }
                path_stack.pop();
            }
            Event::Text(text) => {
                if let Some((_, value)) = field_text.as_mut() {
                    value.push_str(
                        &text
                            .unescape()
                            .map_err(|error| format!("invalid exported property text: {error}"))?,
                    );
                }
            }
            Event::CData(_) if field_text.is_some() => {
                return Err("exported descriptor has CDATA in a required property".to_owned());
            }
            Event::End(_) => {
                if descriptor_field(&path_stack).is_some() {
                    let (field, value) = field_text.take().ok_or_else(|| {
                        "exported descriptor has an incomplete property".to_owned()
                    })?;
                    set_descriptor_field(
                        field,
                        value,
                        &mut name,
                        &mut version,
                        &mut purpose,
                        &mut name_prefix,
                    )?;
                }
                path_stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        event_buffer.clear();
    }

    if !path_stack.is_empty() || !properties_seen {
        return Err("exported descriptor has no complete Configuration/Properties".to_owned());
    }
    let name = name
        .filter(|value: &String| !value.is_empty())
        .ok_or_else(|| "exported descriptor has no Name".to_owned())?;
    let purpose = purpose
        .ok_or_else(|| "exported descriptor has no ConfigurationExtensionPurpose".to_owned())?;
    let name_prefix =
        name_prefix.ok_or_else(|| "exported descriptor has no NamePrefix".to_owned())?;
    Ok(AppliedExtensionDescriptor {
        name,
        version: version.flatten(),
        purpose,
        name_prefix,
    })
}

fn is_properties_path(path: &[Vec<u8>]) -> bool {
    matches!(
        path,
        [configuration, properties] if configuration == b"Configuration" && properties == b"Properties"
    ) || matches!(
        path,
        [metadata, configuration, properties]
            if metadata == b"MetaDataObject"
                && configuration == b"Configuration"
                && properties == b"Properties"
    )
}

fn descriptor_field(path: &[Vec<u8>]) -> Option<&'static str> {
    let (leaf, parent) = path.split_last()?;
    if !is_properties_path(parent) {
        return None;
    }
    match leaf.as_slice() {
        b"Name" => Some("Name"),
        b"Version" => Some("Version"),
        b"ConfigurationExtensionPurpose" => Some("ConfigurationExtensionPurpose"),
        b"NamePrefix" => Some("NamePrefix"),
        _ => None,
    }
}

fn set_descriptor_field(
    field: &str,
    value: String,
    name: &mut Option<String>,
    version: &mut Option<Option<String>>,
    purpose: &mut Option<String>,
    name_prefix: &mut Option<String>,
) -> Result<(), String> {
    let slot = match field {
        "Name" => name,
        "NamePrefix" => name_prefix,
        "Version" => {
            return if version
                .replace((!value.is_empty()).then_some(value))
                .is_none()
            {
                Ok(())
            } else {
                Err("exported descriptor has duplicate Version".to_owned())
            };
        }
        "ConfigurationExtensionPurpose" => {
            let normalized = match value.as_str() {
                "Customization" => "customization",
                "AddOn" => "add-on",
                "Patch" => "patch",
                _ => return Err("exported descriptor has unknown extension purpose".to_owned()),
            };
            return if purpose.replace(normalized.to_owned()).is_none() {
                Ok(())
            } else {
                Err("exported descriptor has duplicate ConfigurationExtensionPurpose".to_owned())
            };
        }
        _ => return Err("unexpected exported descriptor field".to_owned()),
    };
    if slot.replace(value).is_some() {
        return Err(format!("exported descriptor has duplicate {field}"));
    }
    Ok(())
}

/// Имя расширения — идентификатор 1С: буква или подчёркивание, затем буквы, цифры и
/// подчёркивания. Такое имя годится и в аргумент платформы, и в имя каталога: имена
/// устройств Windows (`CON`, `NUL`, `COM1` и прочие) каталогом не стать, и их нет.
pub fn is_extension_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
        && !is_windows_device_name(value)
}

/// Имя устройства Windows без учёта регистра: каталог с таким именем там не создать.
fn is_windows_device_name(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => ["COM", "LPT"].iter().any(|prefix| {
            upper
                .strip_prefix(prefix)
                .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
        }),
    }
}

/// Разбирает `/Out` вызова `/DumpDBCfgList -AllExtensions`: одно имя на строку, BOM снят
/// при чтении журнала, пустой вывод — расширений нет (замер #187). Строка, которая не
/// является идентификатором, — неверный вывод, а не имя: текст сообщения платформы именем
/// расширения не становится.
pub fn parse_extension_name_list(output: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for line in output.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        if !is_extension_identifier(name) {
            return Err(format!(
                "extension list line is not an extension name: {name:?}"
            ));
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

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
        name_prefix: None,
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
    use std::fs;
    use std::path::Path;

    use super::{
        is_extension_identifier, parse_extension_inventory, parse_extension_name_list,
        read_applied_extension_descriptor,
    };

    /// Имя устройства Windows идентификатором расширения не считается в любом регистре:
    /// оно стало бы каталогом, которого там не создать. Похожие имена — обычные.
    #[test]
    fn a_windows_device_name_is_not_an_extension_identifier() {
        for reserved in ["CON", "nul", "Prn", "AUX", "COM1", "com9", "LPT1", "lpt9"] {
            assert!(!is_extension_identifier(reserved), "{reserved}");
        }
        for ordinary in [
            "CONSOLE",
            "COM",
            "COM10",
            "LPT0",
            "NUL_",
            "Расширение",
            "_x1",
        ] {
            assert!(is_extension_identifier(ordinary), "{ordinary}");
        }
    }

    /// Список Конфигуратора — имена по строке; пустой вывод — пустой список, а строка,
    /// не являющаяся идентификатором, — отказ, а не имя.
    #[test]
    fn the_designer_name_list_is_read_line_by_line_and_fail_closed() {
        assert_eq!(
            parse_extension_name_list("Расширение1\r\nРасширение2\r\n\r\n"),
            Ok(vec!["Расширение1".to_owned(), "Расширение2".to_owned()])
        );
        assert_eq!(parse_extension_name_list(""), Ok(Vec::new()));
        assert_eq!(parse_extension_name_list("\n  \n"), Ok(Vec::new()));
        assert!(parse_extension_name_list("Ext\nОшибка в параметрах.\n").is_err());
        assert!(parse_extension_name_list("1Ext\n").is_err());
    }

    fn parse_descriptor(xml: &str) -> Result<super::AppliedExtensionDescriptor, String> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Configuration.xml");
        fs::write(&path, xml).expect("descriptor");
        read_applied_extension_descriptor(&path)
    }

    #[test]
    fn parses_measured_applied_extension_properties_and_empty_prefix() {
        let xml = "<MetaDataObject><Configuration><Properties><Name>A8Probe</Name><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose><NamePrefix>A8_</NamePrefix><Version>1.2.3.4</Version></Properties></Configuration></MetaDataObject>";
        let descriptor = parse_descriptor(xml).expect("applied descriptor");
        assert_eq!(descriptor.name, "A8Probe");
        assert_eq!(descriptor.version.as_deref(), Some("1.2.3.4"));
        assert_eq!(descriptor.purpose, "add-on");
        assert_eq!(descriptor.name_prefix, "A8_");

        let empty = xml.replace("<NamePrefix>A8_</NamePrefix>", "<NamePrefix/>");
        assert_eq!(
            parse_descriptor(&empty)
                .expect("empty applied prefix")
                .name_prefix,
            ""
        );
    }

    #[test]
    fn parses_platform_extension_fixture() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/designer/extension/Configuration.xml"
        ));
        let descriptor = read_applied_extension_descriptor(path).expect("platform XML fixture");
        assert_eq!(descriptor.name, "Расширение1");
        assert_eq!(descriptor.version, None);
        assert_eq!(descriptor.purpose, "customization");
        assert_eq!(descriptor.name_prefix, "Расш1_");
    }

    #[test]
    fn applied_descriptor_refuses_missing_duplicate_and_nested_values() {
        let base = "<Configuration><Properties><Name>X</Name><ConfigurationExtensionPurpose>Patch</ConfigurationExtensionPurpose><NamePrefix>X_</NamePrefix><Version/></Properties></Configuration>";
        assert!(parse_descriptor(base).is_ok());
        assert!(
            parse_descriptor(&base.replace("<NamePrefix>X_</NamePrefix>", ""))
                .expect_err("missing prefix")
                .contains("NamePrefix")
        );
        assert!(parse_descriptor(&base.replace(
            "<NamePrefix>X_</NamePrefix>",
            "<NamePrefix>X_</NamePrefix><NamePrefix>Y_</NamePrefix>"
        ))
        .expect_err("duplicate prefix")
        .contains("duplicate"));
        assert!(parse_descriptor(&base.replace(
            "<NamePrefix>X_</NamePrefix>",
            "<NamePrefix><Nested>X_</Nested></NamePrefix>"
        ))
        .expect_err("nested prefix")
        .contains("nested"));
    }

    #[test]
    fn applied_descriptor_ignores_unrelated_nameprefix_and_refuses_unknown_purpose() {
        let xml = "<MetaDataObject><Configuration><Properties><Name>X</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>X_</NamePrefix><Version/></Properties><ChildObjects><NamePrefix>decoy</NamePrefix></ChildObjects></Configuration></MetaDataObject>";
        let descriptor = parse_descriptor(xml).expect("descriptor");
        assert_eq!(descriptor.name_prefix, "X_");
        assert_eq!(descriptor.purpose, "customization");
        assert!(parse_descriptor(&xml.replace("Customization", "Unknown"))
            .expect_err("unknown purpose")
            .contains("unknown"));
    }

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
