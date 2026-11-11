use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::NsReader;

const SETTINGS_NAMESPACE: &[u8] = b"http://v8.1c.ru/8.3/config/merge/settings";
const SETTINGS_SECTIONS: &[&[u8]] = &[
    b"MainConfiguration",
    b"SecondConfiguration",
    b"OldVendorConfiguration",
    b"Parameters",
    b"SupportRules",
    b"Conformities",
    b"Objects",
];

/// Validate the merge-settings document sent to Designer by /UpdateCfg.
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    let xml = decode_settings(bytes)?;
    check_settings_xml(&xml, bytes)
}

fn decode_settings(bytes: &[u8]) -> Result<std::borrow::Cow<'_, str>, String> {
    match bytes {
        [a, b, content @ ..] if (*a, *b) == (0xff, 0xfe) || (*a, *b) == (0xfe, 0xff) => {
            let (words, remainder) = content.as_chunks::<2>();
            if !remainder.is_empty() {
                return Err("incomplete UTF-16 code unit".into());
            }
            let decode_word = if *a == 0xff {
                u16::from_le_bytes
            } else {
                u16::from_be_bytes
            };
            let words: Vec<_> = words.iter().copied().map(decode_word).collect();
            String::from_utf16(&words)
                .map(Into::into)
                .map_err(|e| e.to_string())
        }
        _ => std::str::from_utf8(bytes)
            .map(Into::into)
            .map_err(|e| e.to_string()),
    }
}

struct SettingsElement {
    name: Vec<u8>,
    text: String,
    seen: Vec<bool>,
}

impl SettingsElement {
    fn new(name: &[u8]) -> Self {
        Self {
            name: name.to_vec(),
            text: String::new(),
            seen: vec![false; allowed_children(name).len()],
        }
    }

    fn add_child(&mut self, name: &[u8]) -> Result<(), String> {
        let allowed = allowed_children(&self.name);
        let index = allowed
            .iter()
            .position(|child| *child == name)
            .ok_or_else(|| format!("unexpected {} in {}", label(name), label(&self.name)))?;
        let duplicate = self.seen[index] && !repeated_child(&self.name, name);
        let out_of_order = self.seen[index + 1..].iter().any(|seen| *seen);
        let conflicting_choice = choice(&self.name).is_some_and(|(first, _)| {
            index >= first && self.seen[first..].iter().any(|seen| *seen)
        });
        if duplicate || out_of_order || conflicting_choice {
            return Err(format!(
                "duplicate, misplaced or conflicting {} in {}",
                label(name),
                label(&self.name)
            ));
        }
        self.seen[index] = true;
        Ok(())
    }

    fn finish(self, parent: Option<&[u8]>) -> Result<(), String> {
        for (child, seen) in allowed_children(&self.name).iter().zip(&self.seen) {
            if !seen && required_child(&self.name, child) {
                return Err(format!("{} requires {}", label(&self.name), label(child)));
            }
        }
        if choice(&self.name).is_some_and(|(first, required)| {
            required && !self.seen[first..].iter().any(|seen| *seen)
        }) {
            return Err(format!("{} requires a merge choice", label(&self.name)));
        }
        check_settings_value(&self.name, parent, &self.text)
    }
}

fn repeated_child(parent: &[u8], child: &[u8]) -> bool {
    match parent {
        b"ComparisonLanguages"
        | b"Conformities"
        | b"Objects"
        | b"Properties"
        | b"Content"
        | b"Types" => true,
        b"Methods" => matches!(child, b"Method" | b"DirectiveArea"),
        _ => false,
    }
}

fn required_child(parent: &[u8], child: &[u8]) -> bool {
    match parent {
        b"Settings" => child == b"Objects",
        b"ComparisonLanguages"
        | b"Conformities"
        | b"SupportRules"
        | b"NewObjects"
        | b"DuplicateObjectsAndModifiedObjectsWithGetFromSecondConfigurationRule"
        | b"ModifiedObjectsWithoutGetFromSecondConfigurationRule"
        | b"Properties"
        | b"Content"
        | b"Types"
        | b"Item"
        | b"Type" => true,
        b"SpreadsheetDocument"
        | b"Method"
        | b"DirectiveArea"
        | b"VariableDeclarationArea"
        | b"MainArea" => child == b"MergeRule",
        _ => false,
    }
}

/// The final alternatives in the child list form one choice in the settings schema.
fn choice(parent: &[u8]) -> Option<(usize, bool)> {
    match parent {
        b"Property" => Some((MERGE_FIELDS.len(), false)),
        b"Module" => Some((0, true)),
        b"Method" | b"DirectiveArea" | b"VariableDeclarationArea" | b"MainArea" => Some((1, true)),
        _ => None,
    }
}

/// Check the merge-settings XML vocabulary before Designer has a chance to ignore the file.
/// Object references and choices that depend on actual configurations are checked by Designer.
fn check_settings_xml(xml: &str, bytes: &[u8]) -> Result<(), String> {
    if !valid_xml_chars(xml) {
        return Err("invalid XML character".into());
    }
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().check_comments = true;
    let mut stack: Vec<SettingsElement> = Vec::new();
    let mut root_seen = false;
    let mut at_start = true;
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|e| e.to_string())?;
        match event {
            Event::Start(ref node) | Event::Empty(ref node) => {
                if namespace != ResolveResult::Bound(Namespace(SETTINGS_NAMESPACE)) {
                    return Err("element is outside the platform merge-settings namespace".into());
                }
                let name = node.local_name();
                let name = name.as_ref();
                let mut attributes = Vec::new();
                for attribute in node.attributes() {
                    let attribute = attribute.map_err(|e| e.to_string())?;
                    let value = attribute.unescape_value().map_err(|e| e.to_string())?;
                    let key = attribute.key.as_ref();
                    let (namespace, local) = reader.resolve_attribute(attribute.key);
                    let schema_hint = namespace
                        == ResolveResult::Bound(Namespace(
                            b"http://www.w3.org/2001/XMLSchema-instance",
                        ))
                        && matches!(
                            local.as_ref(),
                            b"schemaLocation" | b"noNamespaceSchemaLocation"
                        );
                    if !valid_xml_chars(&value) {
                        return Err("invalid XML character in an attribute".into());
                    }
                    if key != b"xmlns"
                        && !key.starts_with(b"xmlns:")
                        && !schema_hint
                        && !allowed_attribute(name, key)
                    {
                        return Err(format!(
                            "unexpected {} attribute on {}",
                            label(key),
                            label(name)
                        ));
                    }
                    attributes.push((key.to_vec(), value.into_owned()));
                }
                let attribute = |key: &[u8]| {
                    attributes
                        .iter()
                        .find(|(name, _)| name == key)
                        .map(|(_, value)| value.as_str())
                        .filter(|value| !value.trim().is_empty())
                };
                match stack.last_mut() {
                    None if root_seen || name != b"Settings" => {
                        return Err("expected exactly one Settings root element".into());
                    }
                    None if attribute(b"version").is_none() => {
                        return Err("Settings requires a version attribute".into());
                    }
                    None => root_seen = true,
                    Some(parent) => parent.add_child(name)?,
                }
                match name {
                    b"Conformity" if attribute(b"fullName").is_none() => {
                        return Err("Conformity requires a fullName attribute".into());
                    }
                    b"Property"
                    | b"Method"
                    | b"DirectiveArea"
                    | b"VariableDeclarationArea"
                    | b"MainArea"
                        if attribute(b"name").is_none() =>
                    {
                        return Err(format!("{} requires a name attribute", label(name)));
                    }
                    b"Item" | b"Type" if attribute(b"Name").is_none() => {
                        return Err(format!("{} requires a Name attribute", label(name)));
                    }
                    b"Subsystem"
                        if !matches!(attribute(b"configuration"), Some("Main" | "Second"))
                            || !attribute(b"includeObjectsFromSubordinateSubsystems")
                                .is_some_and(is_xml_boolean) =>
                    {
                        return Err("Subsystem requires valid configuration and includeObjectsFromSubordinateSubsystems attributes".into());
                    }
                    _ => {}
                }
                let element = SettingsElement::new(name);
                if matches!(event, Event::Start(_)) {
                    stack.push(element);
                } else {
                    element.finish(stack.last().map(|node| node.name.as_slice()))?;
                }
            }
            Event::End(_) => {
                let element = stack.pop().ok_or("unexpected closing element")?;
                element.finish(stack.last().map(|node| node.name.as_slice()))?;
            }
            Event::Text(text) => {
                let text = text.unescape().map_err(|e| e.to_string())?;
                append_settings_text(stack.last_mut(), &text)?;
            }
            Event::CData(text) => {
                let text = std::str::from_utf8(&text).map_err(|e| e.to_string())?;
                append_settings_text(stack.last_mut(), text)?;
            }
            Event::Decl(decl) => {
                if !at_start {
                    return Err("unexpected XML declaration".into());
                }
                if decl.version().map_err(|e| e.to_string())?.as_ref() != b"1.0" {
                    return Err("settings require XML version 1.0".into());
                }
                if let Some(encoding) = decl.encoding().transpose().map_err(|e| e.to_string())? {
                    let matches_input = match bytes {
                        [0xff, 0xfe, ..] => {
                            encoding.eq_ignore_ascii_case(b"UTF-16")
                                || encoding.eq_ignore_ascii_case(b"UTF-16LE")
                        }
                        [0xfe, 0xff, ..] => {
                            encoding.eq_ignore_ascii_case(b"UTF-16")
                                || encoding.eq_ignore_ascii_case(b"UTF-16BE")
                        }
                        _ => {
                            encoding.eq_ignore_ascii_case(b"UTF-8")
                                || (encoding.eq_ignore_ascii_case(b"US-ASCII") && bytes.is_ascii())
                        }
                    };
                    if !matches_input {
                        return Err("settings encoding does not match UTF-8 or UTF-16 input".into());
                    }
                }
            }
            Event::DocType(_) => return Err("document type declarations are unsupported".into()),
            Event::Eof if root_seen && stack.is_empty() => return Ok(()),
            Event::Eof => return Err("expected complete Settings with an Objects section".into()),
            _ => {}
        }
        at_start = false;
    }
}

fn valid_xml_chars(value: &str) -> bool {
    value.chars().all(|character| matches!(character,
        '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
}

fn allowed_attribute(element: &[u8], key: &[u8]) -> bool {
    match element {
        b"Settings" => matches!(key, b"version" | b"platformVersion"),
        b"Conformity" | b"Object" => matches!(key, b"fullName" | b"fullNameInSecondConfiguration"),
        b"Property" => key == b"name",
        b"Method" | b"DirectiveArea" | b"VariableDeclarationArea" | b"MainArea" => {
            matches!(key, b"name" | b"nameInSecondConfiguration")
        }
        b"Item" | b"Type" => key == b"Name",
        b"Subsystem" => matches!(
            key,
            b"configuration" | b"includeObjectsFromSubordinateSubsystems"
        ),
        _ => false,
    }
}

fn label(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

const MERGE_FIELDS: &[&[u8]] = &[
    b"MergeRule",
    b"MergeRuleForPropertiesChangedTwice",
    b"MergeRuleForPropertiesChangedOnce",
];

fn allowed_children(name: &[u8]) -> &'static [&'static [u8]] {
    match name {
        b"Settings" => SETTINGS_SECTIONS,
        b"MainConfiguration" | b"SecondConfiguration" | b"OldVendorConfiguration" => {
            &[b"Name", b"Version", b"Vendor"]
        }
        b"Parameters" => &[
            b"ConfigurationsRelation",
            b"ComparisonLanguages",
            b"AllowMainConfigurationObjectDeletion",
            b"CopyObjectsMode",
        ],
        b"ComparisonLanguages" => &[b"Language"],
        b"SupportRules" => &[
            b"NewObjects",
            b"DuplicateObjectsAndModifiedObjectsWithGetFromSecondConfigurationRule",
            b"ModifiedObjectsWithoutGetFromSecondConfigurationRule",
        ],
        b"NewObjects"
        | b"DuplicateObjectsAndModifiedObjectsWithGetFromSecondConfigurationRule"
        | b"ModifiedObjectsWithoutGetFromSecondConfigurationRule" => {
            &[b"ChangesAllowedRule", b"ChangesNotRecommendedRule"]
        }
        b"Conformities" => &[b"Conformity"],
        b"Objects" => &[b"Configuration", b"Object"],
        b"Configuration" => &[
            b"MergeRule",
            b"MergeRuleForPropertiesChangedTwice",
            b"MergeRuleForPropertiesChangedOnce",
            b"ObjectOrder",
            b"Properties",
        ],
        b"Object" => &[
            b"MergeRule",
            b"MergeRuleForPropertiesChangedTwice",
            b"MergeRuleForPropertiesChangedOnce",
            b"ObjectOrder",
            b"Properties",
            b"Subsystem",
        ],
        b"Subsystem" => &[
            b"MergeRule",
            b"MergeRuleForPropertiesChangedTwice",
            b"MergeRuleForPropertiesChangedOnce",
            b"ObjectOrder",
        ],
        b"Properties" => &[b"Property"],
        b"Property" => &[
            b"MergeRule",
            b"MergeRuleForPropertiesChangedTwice",
            b"MergeRuleForPropertiesChangedOnce",
            b"Module",
            b"FormModule",
            b"SpreadsheetDocument",
            b"Content",
            b"Types",
        ],
        b"Module" => &[b"Methods", b"Patch"],
        b"FormModule" => &[
            b"MergeRule",
            b"MergeRuleForPropertiesChangedTwice",
            b"MergeRuleForPropertiesChangedOnce",
            b"Module",
        ],
        b"SpreadsheetDocument" => MERGE_FIELDS,
        b"Methods" => &[
            b"Method",
            b"DirectiveArea",
            b"VariableDeclarationArea",
            b"MainArea",
        ],
        b"Method" | b"DirectiveArea" | b"VariableDeclarationArea" | b"MainArea" => {
            &[b"MergeRule", b"Text", b"Patch"]
        }
        b"Content" => &[b"Item"],
        b"Types" => &[b"Type"],
        b"Item" | b"Type" => &[b"MergeRule"],
        _ => &[],
    }
}

fn append_settings_text(element: Option<&mut SettingsElement>, text: &str) -> Result<(), String> {
    if !valid_xml_chars(text) {
        return Err("invalid XML character in text".into());
    }
    match element {
        Some(element)
            if allowed_children(&element.name).is_empty() && element.name != b"Conformity" =>
        {
            element.text.push_str(text)
        }
        _ if !text.trim().is_empty() => return Err("unexpected text in merge-settings XML".into()),
        _ => {}
    }
    Ok(())
}

fn is_xml_boolean(value: &str) -> bool {
    matches!(value.trim(), "true" | "false" | "0" | "1")
}

fn check_settings_value(name: &[u8], parent: Option<&[u8]>, value: &str) -> Result<(), String> {
    let value = value.trim();
    let valid = match name {
        _ if MERGE_FIELDS.contains(&name) && parent == Some(b"SpreadsheetDocument") => {
            matches!(value, "Merge" | "Unite" | "MergeBase")
        }
        _ if MERGE_FIELDS.contains(&name) => matches!(
            value,
            "DoNotMerge"
                | "GetFromSecondConfiguration"
                | "MergePrioritizingMainConfiguration"
                | "MergePrioritizingSecondConfiguration"
                | "MergeWithExternalTool"
        ),
        b"ConfigurationsRelation" => matches!(
            value,
            "ConfigurationsNotRelated"
                | "SecondConfigurationIsDescendantOfMainConfiguration"
                | "MainConfigurationIsDescendantOfSecondConfiguration"
        ),
        b"ObjectOrder" => matches!(
            value,
            "GetFromMainConfiguration" | "GetFromSecondConfiguration"
        ),
        b"AllowMainConfigurationObjectDeletion" | b"CopyObjectsMode" => is_xml_boolean(value),
        b"ChangesAllowedRule" | b"ChangesNotRecommendedRule" => match (parent, value) {
            (Some(b"NewObjects"), "KeepCurrentRule")
            | (
                Some(b"ModifiedObjectsWithoutGetFromSecondConfigurationRule"),
                "ObjectNotEditable",
            ) => false,
            (
                _,
                "KeepCurrentRule"
                | "ObjectNotEditable"
                | "ObjectIsEditableSupportEnabled"
                | "ObjectNotSupported",
            ) => true,
            _ => false,
        },
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid {} value", label(name)))
    }
}
