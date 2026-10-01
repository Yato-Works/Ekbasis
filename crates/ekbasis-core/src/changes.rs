//! Application of experiment changes to an experiment worktree (pipeline step “Apply Changes”).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_yaml_ng::Value as YamlValue;

use crate::spec::Change;

/// What a single change did to the worktree.
#[derive(Debug, Clone)]
pub struct ChangeOutcome {
    /// File the change touched, relative to the repository root.
    pub file: String,
    /// `config`, `replace` or `file`.
    pub kind: &'static str,
    /// Human readable summary, e.g. `startup.preload: true -> false`.
    pub detail: String,
}

/// Applies every change of an experiment inside `root` (a worktree of the repository).
///
/// The function is intentionally strict: a `config` change with a `from` guard fails when the
/// current value differs, and a `replace` change fails when the searched text is absent. An
/// alternative timeline is only worth measuring when we know exactly what was changed.
pub fn apply_changes(root: &Path, changes: &[Change]) -> Result<Vec<ChangeOutcome>> {
    let mut outcomes = Vec::with_capacity(changes.len());
    for change in changes {
        outcomes.push(apply_change(root, change)?);
    }
    Ok(outcomes)
}

fn apply_change(root: &Path, change: &Change) -> Result<ChangeOutcome> {
    let path = resolve(root, change.path())?;
    match change {
        Change::Config {
            file,
            key,
            from,
            to,
        } => {
            let format = ConfigFormat::detect(&path)?;
            let text = read_text(&path)?;
            let (updated, previous) = format
                .set_key(&text, key, from.as_ref(), to)
                .with_context(|| format!("{} change in `{file}` failed", format.name()))?;
            write_text(&path, &updated)?;
            Ok(ChangeOutcome {
                file: file.clone(),
                kind: "config",
                detail: format!("{key}: {} -> {}", describe_yaml(&previous), describe_yaml(to)),
            })
        }
        Change::Replace { file, from, to } => {
            let text = read_text(&path)?;
            let occurrences = text.matches(from.as_str()).count();
            if occurrences == 0 {
                bail!("replace change in `{file}` found no occurrence of {from:?}");
            }
            let updated = text.replace(from.as_str(), to.as_str());
            write_text(&path, &updated)?;
            Ok(ChangeOutcome {
                file: file.clone(),
                kind: "replace",
                detail: format!("{occurrences} occurrence(s) replaced"),
            })
        }
        Change::File { path: file, content } => {
            write_text(&path, content)?;
            Ok(ChangeOutcome {
                file: file.clone(),
                kind: "file",
                detail: format!("wrote {} bytes", content.len()),
            })
        }
    }
}

/// Resolves a repository-relative path and refuses to escape the repository root.
fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let candidate = Path::new(relative);
    if candidate.is_absolute() {
        bail!("path `{relative}` must be relative to the repository root");
    }
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("path `{relative}` must not contain `..`");
    }
    Ok(root.join(candidate))
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("cannot read `{}`", path.display()))
}

fn write_text(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create `{}`", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("cannot write `{}`", path.display()))
}

/// Structured config formats AION can edit key by key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    Toml,
    Json,
    Yaml,
    Env,
}

/// Renders a YAML value on a single line for CLI output (`true`, `"fast"`, `[1, 2]`).
pub fn describe_yaml(value: &YamlValue) -> String {
    match serde_yaml_ng::to_string(value) {
        Ok(text) => {
            let single = text.trim().replace('\n', " ");
            if single.is_empty() { "~".to_string() } else { single }
        }
        Err(_) => "<unprintable>".to_string(),
    }
}

impl ConfigFormat {
    /// Chooses the editor based on the file name / extension.
    fn detect(path: &Path) -> Result<ConfigFormat> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name == ".env" || name.starts_with(".env.") || extension == "env" {
            return Ok(ConfigFormat::Env);
        }
        match extension.as_str() {
            "toml" => Ok(ConfigFormat::Toml),
            "json" => Ok(ConfigFormat::Json),
            "yaml" | "yml" => Ok(ConfigFormat::Yaml),
            "" => bail!(
                "`{}` has no file extension, so AION cannot tell how to edit it; \
                 use a `type: replace` change instead",
                path.display()
            ),
            other => bail!(
                "`{}` is not a structured config file AION can edit (`{other}`); \
                 use a `type: replace` change instead",
                path.display()
            ),
        }
    }

    fn name(self) -> &'static str {
        match self {
            ConfigFormat::Toml => "toml",
            ConfigFormat::Json => "json",
            ConfigFormat::Yaml => "yaml",
            ConfigFormat::Env => "env",
        }
    }

    /// Sets a dotted key, returning the new file content and the previous value.
    fn set_key(
        &self,
        text: &str,
        key: &str,
        expected: Option<&YamlValue>,
        new_value: &YamlValue,
    ) -> Result<(String, YamlValue)> {
        match self {
            ConfigFormat::Toml => set_toml_key(text, key, expected, new_value),
            ConfigFormat::Json => set_json_key(text, key, expected, new_value),
            ConfigFormat::Yaml => set_yaml_key(text, key, expected, new_value),
            ConfigFormat::Env => set_env_key(text, key, expected, new_value),
        }
    }
}

/// Splits a dotted key path into its parts.
fn key_parts(key: &str) -> Result<Vec<String>> {
    let parts: Vec<String> = key
        .split('.')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        bail!("key must not be empty");
    }
    Ok(parts)
}

/// Enforces the optional `from` guard of a config change.
fn ensure_guard(actual: &YamlValue, expected: Option<&YamlValue>, key: &str) -> Result<()> {
    if let Some(expected) = expected {
        if !yaml_equal(actual, expected) {
            bail!(
                "expected `{key}` to be {} but found {}",
                describe_yaml(expected),
                describe_yaml(actual)
            );
        }
    }
    Ok(())
}

fn yaml_equal(left: &YamlValue, right: &YamlValue) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => describe_yaml(left) == describe_yaml(right),
    }
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

fn yaml_kind(value: &YamlValue) -> &'static str {
    match value {
        YamlValue::Null => "null",
        YamlValue::Bool(_) => "a boolean",
        YamlValue::Number(_) => "a number",
        YamlValue::String(_) => "a string",
        YamlValue::Sequence(_) => "a sequence",
        YamlValue::Mapping(_) => "a mapping",
        YamlValue::Tagged(_) => "a tagged value",
    }
}

fn json_to_yaml(value: &serde_json::Value) -> YamlValue {
    match value {
        serde_json::Value::Null => YamlValue::Null,
        serde_json::Value::Bool(flag) => YamlValue::Bool(*flag),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                YamlValue::Number(integer.into())
            } else if let Some(unsigned) = number.as_u64() {
                YamlValue::Number(unsigned.into())
            } else {
                YamlValue::Number(number.as_f64().unwrap_or_default().into())
            }
        }
        serde_json::Value::String(text) => YamlValue::String(text.clone()),
        serde_json::Value::Array(items) => {
            YamlValue::Sequence(items.iter().map(json_to_yaml).collect())
        }
        serde_json::Value::Object(map) => YamlValue::Mapping(
            map.iter()
                .map(|(key, value)| (YamlValue::String(key.clone()), json_to_yaml(value)))
                .collect(),
        ),
    }
}

/// Sets a dotted key inside a JSON document (numeric path parts index into arrays).
fn set_json_key(
    text: &str,
    key: &str,
    expected: Option<&YamlValue>,
    new_value: &YamlValue,
) -> Result<(String, YamlValue)> {
    let mut document: serde_json::Value =
        serde_json::from_str(text).context("cannot parse JSON document")?;
    let parts = key_parts(key)?;
    let previous = {
        let cursor = navigate_json_mut(&mut document, &parts, key)?;
        let previous = json_to_yaml(cursor);
        *cursor = serde_json::to_value(new_value).context("cannot convert the new value to JSON")?;
        previous
    };
    ensure_guard(&previous, expected, key)?;
    let mut serialized =
        serde_json::to_string_pretty(&document).context("cannot serialize JSON document")?;
    serialized.push('\n');
    Ok((serialized, previous))
}

fn navigate_json_mut<'a>(
    document: &'a mut serde_json::Value,
    parts: &[String],
    key: &str,
) -> Result<&'a mut serde_json::Value> {
    let (parents, leaf) = parts.split_at(parts.len() - 1);
    let leaf = leaf.first().expect("key_parts never returns an empty path");
    let mut cursor: &mut serde_json::Value = document;
    for part in parents {
        cursor = match cursor {
            serde_json::Value::Object(map) => map
                .get_mut(part.as_str())
                .ok_or_else(|| anyhow::anyhow!("key `{key}` not found: `{part}` is missing"))?,
            serde_json::Value::Array(items) => {
                let index = part.parse::<usize>().map_err(|_| {
                    anyhow::anyhow!(
                        "key `{key}`: `{part}` is not a numeric index, but the value is an array"
                    )
                })?;
                items.get_mut(index).ok_or_else(|| {
                    anyhow::anyhow!("key `{key}`: array index {index} is out of range")
                })?
            }
            other => bail!(
                "key `{key}`: cannot descend into {} with `{part}`",
                json_kind(other)
            ),
        };
    }
    match cursor {
        serde_json::Value::Object(map) => {
            if !map.contains_key(leaf.as_str()) {
                map.insert(leaf.clone(), serde_json::Value::Null);
            }
            Ok(map.get_mut(leaf.as_str()).expect("key inserted above"))
        }
        serde_json::Value::Array(items) => {
            let index = leaf.parse::<usize>().map_err(|_| {
                anyhow::anyhow!(
                    "key `{key}`: `{leaf}` is not a numeric index, but the value is an array"
                )
            })?;
            items
                .get_mut(index)
                .ok_or_else(|| anyhow::anyhow!("key `{key}`: array index {index} is out of range"))
        }
        other => bail!("key `{key}`: cannot set `{leaf}` inside {}", json_kind(other)),
    }
}

/// Looks up a string key in a YAML mapping without relying on index trait details.
fn yaml_map_contains(map: &serde_yaml_ng::Mapping, name: &str) -> bool {
    map.keys()
        .any(|key| matches!(key, YamlValue::String(text) if text == name))
}

fn yaml_map_get_mut<'a>(
    map: &'a mut serde_yaml_ng::Mapping,
    name: &str,
) -> Option<&'a mut YamlValue> {
    map.iter_mut()
        .find(|(key, _)| matches!(key, YamlValue::String(text) if text == name))
        .map(|(_, value)| value)
}

/// Sets a dotted key inside a YAML document (numeric path parts index into sequences).
fn set_yaml_key(
    text: &str,
    key: &str,
    expected: Option<&YamlValue>,
    new_value: &YamlValue,
) -> Result<(String, YamlValue)> {
    let mut document: YamlValue =
        serde_yaml_ng::from_str(text).context("cannot parse YAML document")?;
    let parts = key_parts(key)?;
    let previous = {
        let cursor = navigate_yaml_mut(&mut document, &parts, key)?;
        let previous = cursor.clone();
        *cursor = new_value.clone();
        previous
    };
    ensure_guard(&previous, expected, key)?;
    let mut serialized =
        serde_yaml_ng::to_string(&document).context("cannot serialize YAML document")?;
    if !serialized.ends_with('\n') {
        serialized.push('\n');
    }
    Ok((serialized, previous))
}

fn navigate_yaml_mut<'a>(
    document: &'a mut YamlValue,
    parts: &[String],
    key: &str,
) -> Result<&'a mut YamlValue> {
    let (parents, leaf) = parts.split_at(parts.len() - 1);
    let leaf = leaf.first().expect("key_parts never returns an empty path");
    let mut cursor: &mut YamlValue = document;
    for part in parents {
        cursor = match cursor {
            YamlValue::Mapping(map) => yaml_map_get_mut(map, part)
                .ok_or_else(|| anyhow::anyhow!("key `{key}` not found: `{part}` is missing"))?,
            YamlValue::Sequence(items) => {
                let index = part.parse::<usize>().map_err(|_| {
                    anyhow::anyhow!(
                        "key `{key}`: `{part}` is not a numeric index, but the value is a sequence"
                    )
                })?;
                items.get_mut(index).ok_or_else(|| {
                    anyhow::anyhow!("key `{key}`: sequence index {index} is out of range")
                })?
            }
            other => bail!(
                "key `{key}`: cannot descend into {} with `{part}`",
                yaml_kind(other)
            ),
        };
    }
    match cursor {
        YamlValue::Mapping(map) => {
            if !yaml_map_contains(map, leaf) {
                map.insert(YamlValue::String(leaf.clone()), YamlValue::Null);
            }
            Ok(yaml_map_get_mut(map, leaf).expect("key inserted above"))
        }
        YamlValue::Sequence(items) => {
            let index = leaf.parse::<usize>().map_err(|_| {
                anyhow::anyhow!(
                    "key `{key}`: `{leaf}` is not a numeric index, but the value is a sequence"
                )
            })?;
            items
                .get_mut(index)
                .ok_or_else(|| anyhow::anyhow!("key `{key}`: sequence index {index} is out of range"))
        }
        other => bail!("key `{key}`: cannot set `{leaf}` inside {}", yaml_kind(other)),
    }
}

/// Sets a flat `KEY=VALUE` entry inside an `.env` style file.
fn set_env_key(
    text: &str,
    key: &str,
    expected: Option<&YamlValue>,
    new_value: &YamlValue,
) -> Result<(String, YamlValue)> {
    let parts = key_parts(key)?;
    let name = if parts.len() == 1 {
        parts[0].clone()
    } else {
        bail!("env files only support flat `KEY=VALUE` entries, but got `{key}`");
    };
    let rendered = match new_value {
        YamlValue::String(text) => text.clone(),
        YamlValue::Bool(flag) => flag.to_string(),
        YamlValue::Number(number) => number.to_string(),
        YamlValue::Null => String::new(),
        other => bail!(
            "env value for `{name}` must be a scalar, found {}",
            describe_yaml(other)
        ),
    };

    let mut lines: Vec<String> = Vec::new();
    let mut previous: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let is_target = !trimmed.starts_with('#')
            && trimmed
                .split_once('=')
                .map(|(candidate, _)| candidate.trim() == name)
                .unwrap_or(false);
        if is_target && previous.is_none() {
            let old = line
                .split_once('=')
                .map(|(_, value)| value.trim().trim_matches('"').to_string())
                .unwrap_or_default();
            previous = Some(old);
            lines.push(format!("{name}={rendered}"));
        } else {
            lines.push(line.to_string());
        }
    }
    if previous.is_none() {
        lines.push(format!("{name}={rendered}"));
    }
    let previous_value = previous.map(YamlValue::String).unwrap_or(YamlValue::Null);
    ensure_guard(&previous_value, expected, key)?;
    let mut serialized = lines.join("\n");
    serialized.push('\n');
    Ok((serialized, previous_value))
}

/// Sets a dotted key inside a TOML document, preserving formatting and comments.
fn set_toml_key(
    text: &str,
    key: &str,
    expected: Option<&YamlValue>,
    new_value: &YamlValue,
) -> Result<(String, YamlValue)> {
    let mut document: toml_edit::DocumentMut = text.parse().context("cannot parse TOML document")?;
    let parts = key_parts(key)?;
    let (parents, leaf) = parts.split_at(parts.len() - 1);
    let leaf = leaf.first().expect("key_parts never returns an empty path");

    let mut item: &mut toml_edit::Item = document.as_item_mut();
    for part in parents {
        item = toml_child_mut(item, part, key)?;
    }

    let previous = match item.get(leaf.as_str()) {
        Some(existing) => toml_item_to_yaml(existing)?,
        None => YamlValue::Null,
    };
    ensure_guard(&previous, expected, key)?;

    let replacement = toml_item_from_yaml(new_value)?;
    match item {
        toml_edit::Item::Table(table) => {
            if let Some(existing) = table.get_mut(leaf.as_str()) {
                // Keep the comment that was attached to the old value: an experiment should
                // change the value, not the explanation next to it.
                let decor = existing.as_value().map(|value| value.decor().clone());
                *existing = replacement;
                if let (Some(decor), toml_edit::Item::Value(value)) = (decor, existing) {
                    *value.decor_mut() = decor;
                }
            } else {
                table.insert(leaf.as_str(), replacement);
            }
        }
        toml_edit::Item::Value(toml_edit::Value::InlineTable(table)) => {
            let value = match replacement {
                toml_edit::Item::Value(value) => value,
                _ => bail!("key `{key}`: cannot insert a nested table into an inline table"),
            };
            table.insert(leaf.as_str(), value);
        }
        other => bail!(
            "key `{key}`: cannot set `{leaf}` inside {}",
            toml_kind(other)
        ),
    }

    let mut serialized = document.to_string();
    if !serialized.ends_with('\n') {
        serialized.push('\n');
    }
    Ok((serialized, previous))
}

/// Descends one level into a TOML item by table name.
///
/// TOML keys address tables by name (`startup.preload`, `build.release.lto`); arrays of tables
/// (`[[x]]`) cannot be indexed from an AION key path. Non-existent keys are a hard error, so a
/// typo in a spec never silently measures the wrong configuration.
fn toml_child_mut<'a>(
    item: &'a mut toml_edit::Item,
    part: &str,
    key: &str,
) -> Result<&'a mut toml_edit::Item> {
    if item.get(part).is_none() {
        bail!("key `{key}` not found: `{part}` is missing");
    }
    match item.get_mut(part) {
        Some(child) => Ok(child),
        None => bail!("key `{key}` not found: `{part}` is missing"),
    }
}

fn toml_kind(item: &toml_edit::Item) -> &'static str {
    match item {
        toml_edit::Item::None => "nothing",
        toml_edit::Item::Value(_) => "a value",
        toml_edit::Item::Table(_) => "a table",
        toml_edit::Item::ArrayOfTables(_) => "an array of tables",
    }
}

/// Converts a YAML value into a TOML value.
fn toml_value_from_yaml(value: &YamlValue) -> Result<toml_edit::Value> {
    match value {
        YamlValue::Null => {
            bail!("TOML has no null value; use a string, number or boolean instead")
        }
        YamlValue::Bool(flag) => Ok((*flag).into()),
        YamlValue::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Ok(integer.into())
            } else if let Some(float) = number.as_f64() {
                toml_float(float)
            } else {
                bail!(
                    "number `{}` cannot be represented in TOML",
                    describe_yaml(value)
                )
            }
        }
        YamlValue::String(text) => Ok(text.clone().into()),
        YamlValue::Sequence(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(toml_value_from_yaml(item)?);
            }
            Ok(array.into())
        }
        YamlValue::Mapping(map) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, item) in map {
                let name = match key {
                    YamlValue::String(name) => name.clone(),
                    other => bail!("TOML keys must be strings, found {}", describe_yaml(other)),
                };
                inline.insert(name.as_str(), toml_value_from_yaml(item)?);
            }
            Ok(inline.into())
        }
        other => bail!("value {} cannot be represented in TOML", describe_yaml(other)),
    }
}

fn toml_item_from_yaml(value: &YamlValue) -> Result<toml_edit::Item> {
    Ok(toml_value_from_yaml(value)?.into())
}

fn toml_float(value: f64) -> Result<toml_edit::Value> {
    if value.is_finite() {
        Ok(value.into())
    } else {
        bail!("TOML cannot store non-finite floats")
    }
}

/// Converts a TOML item back into a YAML value (used for guards and CLI output).
fn toml_item_to_yaml(item: &toml_edit::Item) -> Result<YamlValue> {
    match item {
        toml_edit::Item::None => Ok(YamlValue::Null),
        toml_edit::Item::Value(value) => toml_value_to_yaml(value),
        toml_edit::Item::Table(table) => toml_table_to_yaml(table),
        toml_edit::Item::ArrayOfTables(array) => {
            let mut items = Vec::new();
            for table in array.iter() {
                items.push(toml_table_to_yaml(table)?);
            }
            Ok(YamlValue::Sequence(items))
        }
    }
}

fn toml_table_to_yaml(table: &toml_edit::Table) -> Result<YamlValue> {
    let mut map = serde_yaml_ng::Mapping::new();
    for (key, value) in table.iter() {
        map.insert(YamlValue::String(key.to_string()), toml_item_to_yaml(value)?);
    }
    Ok(YamlValue::Mapping(map))
}

fn toml_value_to_yaml(value: &toml_edit::Value) -> Result<YamlValue> {
    Ok(match value {
        toml_edit::Value::String(text) => YamlValue::String(text.value().clone()),
        toml_edit::Value::Integer(integer) => YamlValue::Number((*integer.value()).into()),
        toml_edit::Value::Float(float) => yaml_number_from_f64(*float.value()),
        toml_edit::Value::Boolean(flag) => YamlValue::Bool(*flag.value()),
        toml_edit::Value::Datetime(datetime) => YamlValue::String(datetime.value().to_string()),
        toml_edit::Value::Array(array) => {
            let mut items = Vec::new();
            for entry in array.iter() {
                items.push(toml_value_to_yaml(entry)?);
            }
            YamlValue::Sequence(items)
        }
        toml_edit::Value::InlineTable(inline) => {
            let mut map = serde_yaml_ng::Mapping::new();
            for (key, entry) in inline.iter() {
                map.insert(YamlValue::String(key.to_string()), toml_value_to_yaml(entry)?);
            }
            YamlValue::Mapping(map)
        }
    })
}

fn yaml_number_from_f64(value: f64) -> YamlValue {
    if value.is_finite() {
        YamlValue::Number(value.into())
    } else {
        YamlValue::String(value.to_string())
    }
}
