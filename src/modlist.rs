use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ModListEntry {
    pub name: String,
    pub enabled: bool,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
pub struct ModList {
    path: PathBuf,
    bom: bool,
    body: Vec<u8>,
    entries: Vec<ModListEntry>,
    array_open: usize,
    array_close: usize,
}

impl ModList {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path)
            .with_context(|| format!("cannot read mod-list file {}", path.display()))?;
        Self::from_bytes(path.to_path_buf(), bytes)
    }

    pub fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self> {
        let (bom, body) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (true, bytes[3..].to_vec())
        } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
            bail!("mod-list.json is UTF-16 encoded; only UTF-8 is supported");
        } else {
            (false, bytes)
        };
        let value: Value = serde_json::from_slice(&body)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        let mods = value
            .get("mods")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("mod-list.json has no mods array"))?;
        for entry in mods {
            let object = entry
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("mod-list.json contains a non-object mod entry"))?;
            if !object.get("name").is_some_and(Value::is_string) {
                bail!("mod-list.json contains an entry without a string name");
            }
            if !object.get("enabled").is_some_and(Value::is_boolean) {
                bail!("mod-list.json contains an entry without a boolean enabled field");
            }
        }
        let (array_open, array_close) = find_mods_array(&body)?;
        let spans = collect_spans(&body, array_open, array_close)?;
        if spans.len() != mods.len() {
            bail!("mod-list.json structure could not be analyzed safely");
        }
        let mut entries = Vec::with_capacity(spans.len());
        for (span, value) in spans.into_iter().zip(mods) {
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("mod-list.json entry has no name"))?;
            let enabled = value
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| anyhow::anyhow!("mod-list.json entry has no enabled field"))?;
            entries.push(ModListEntry {
                name: name.to_string(),
                enabled,
                start: span.0,
                end: span.1,
            });
        }
        Ok(Self {
            path,
            bom,
            body,
            entries,
            array_open,
            array_close,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[ModListEntry] {
        &self.entries
    }

    pub fn has_entry(&self, name: &str) -> bool {
        self.find_index(name).is_some()
    }

    pub fn is_enabled(&self, name: &str) -> Option<bool> {
        self.find_index(name)
            .map(|index| self.entries[index].enabled)
    }

    pub fn enabled_map(&self) -> std::collections::BTreeMap<String, bool> {
        self.entries
            .iter()
            .map(|entry| (entry.name.to_ascii_lowercase(), entry.enabled))
            .collect()
    }

    fn find_index(&self, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.name == name)
            .or_else(|| {
                self.entries
                    .iter()
                    .position(|entry| entry.name.eq_ignore_ascii_case(name))
            })
    }

    pub fn append_disabled(&self, name: &str) -> Result<Option<Vec<u8>>> {
        if self.has_entry(name) {
            return Ok(None);
        }
        let mut body = if self.entries.is_empty() {
            self.append_to_empty_array(name)?
        } else {
            let last = self.entries.last().expect("non-empty entry list");
            let new_entry = format_entry(&self.body, last, name)?;
            let mut prefix = if self.entries.len() >= 2 {
                let previous = &self.entries[self.entries.len() - 2];
                self.body[previous.end..last.start].to_vec()
            } else {
                let mut prefix = vec![b','];
                prefix.extend_from_slice(&self.body[self.array_open + 1..last.start]);
                prefix
            };
            while matches!(prefix.last(), Some(b' ' | b'\t')) {
                prefix.pop();
            }
            let mut updated = self.body.clone();
            let mut inserted = prefix;
            inserted.extend_from_slice(new_entry.as_bytes());
            updated.splice(last.end..last.end, inserted);
            updated
        };
        validate_result(&mut body, self.entries.len() + 1, Some((name, false)))?;
        Ok(Some(self.wrap(body)))
    }

    fn append_to_empty_array(&self, name: &str) -> Result<Vec<u8>> {
        let json_name = serde_json::to_string(name)?;
        let inner = &self.body[self.array_open + 1..self.array_close];
        let compact = !inner.contains(&b'\n');
        let newline = detect_newline(&self.body);
        let array_indent = line_indent(&self.body, self.array_open);
        let object_indent = format!("{array_indent}  ");
        let field_indent = format!("{object_indent}  ");
        let entry = if compact {
            format!("{{\"name\": {json_name}, \"enabled\": false}}")
        } else {
            format!(
                "{newline}{object_indent}{{{newline}{field_indent}\"name\": {json_name},{newline}{field_indent}\"enabled\": false{newline}{object_indent}}}{newline}{array_indent}"
            )
        };
        let mut updated = self.body.clone();
        if compact {
            updated.splice(self.array_open + 1..self.array_open + 1, entry.into_bytes());
        } else {
            updated.splice(self.array_open + 1..self.array_close, entry.into_bytes());
        }
        Ok(updated)
    }

    pub fn remove_entry(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(index) = self.find_index(name) else {
            return Ok(None);
        };
        let target = &self.entries[index];
        let (delete_start, delete_end) = if self.entries.len() == 1 {
            (target.start, target.end)
        } else if index == 0 {
            (target.start, self.entries[1].start)
        } else {
            (self.entries[index - 1].end, target.end)
        };
        let mut body = self.body.clone();
        body.drain(delete_start..delete_end);
        validate_result(&mut body, self.entries.len() - 1, None)?;
        Ok(Some(self.wrap(body)))
    }

    fn wrap(&self, body: Vec<u8>) -> Vec<u8> {
        if self.bom {
            let mut result = Vec::with_capacity(body.len() + 3);
            result.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            result.extend_from_slice(&body);
            result
        } else {
            body
        }
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid mod-list path {}", path.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("cannot create a temporary file in {}", parent.display()))?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| anyhow::anyhow!("cannot replace {}: {}", path.display(), error.error))?;
    Ok(())
}

fn validate_result(
    body: &mut [u8],
    expected_count: usize,
    expected_entry: Option<(&str, bool)>,
) -> Result<()> {
    let value: Value = serde_json::from_slice(body).context("edited mod-list.json is invalid")?;
    let mods = value
        .get("mods")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("edited mod-list.json lost its mods array"))?;
    if mods.len() != expected_count {
        bail!("edited mod-list.json has an unexpected number of mods");
    }
    if let Some((name, enabled)) = expected_entry {
        let found = mods.iter().any(|entry| {
            entry.get("name").and_then(Value::as_str) == Some(name)
                && entry.get("enabled").and_then(Value::as_bool) == Some(enabled)
        });
        if !found {
            bail!("edited mod-list.json does not contain the new entry");
        }
    }
    Ok(())
}

fn format_entry(body: &[u8], last: &ModListEntry, name: &str) -> Result<String> {
    let json_name = serde_json::to_string(name)?;
    let source = &body[last.start..last.end];
    let newline = detect_newline(body);
    if !source.contains(&b'\n') {
        return Ok(format!("{{\"name\": {json_name}, \"enabled\": false}}"));
    }
    let object_indent = line_indent(body, last.start);
    let field_indent =
        first_inner_line_indent(source).unwrap_or_else(|| format!("{object_indent}  "));
    Ok(format!(
        "{object_indent}{{{newline}{field_indent}\"name\": {json_name},{newline}{field_indent}\"enabled\": false{newline}{object_indent}}}"
    ))
}

fn detect_newline(body: &[u8]) -> &'static str {
    if body.windows(2).any(|window| window == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn line_indent(body: &[u8], position: usize) -> String {
    let line_start = body[..position]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    body[line_start..position]
        .iter()
        .take_while(|byte| **byte == b' ' || **byte == b'\t')
        .map(|byte| *byte as char)
        .collect()
}

fn first_inner_line_indent(source: &[u8]) -> Option<String> {
    let newline = source.iter().position(|byte| *byte == b'\n')?;
    let rest = &source[newline + 1..];
    let indent: String = rest
        .iter()
        .take_while(|byte| **byte == b' ' || **byte == b'\t')
        .map(|byte| *byte as char)
        .collect();
    if indent.is_empty() {
        None
    } else {
        Some(indent)
    }
}

fn find_mods_array(body: &[u8]) -> Result<(usize, usize)> {
    let mut depth = 0i32;
    let mut index = 0usize;
    while index < body.len() {
        match body[index] {
            b'"' => {
                let end = skip_string(body, index)?;
                if depth == 1 {
                    let after = skip_ws(body, end);
                    if body.get(after) == Some(&b':') && &body[index + 1..end - 1] == b"mods" {
                        let value_start = skip_ws(body, after + 1);
                        if body.get(value_start) != Some(&b'[') {
                            bail!("mods is not an array");
                        }
                        let value_end = find_matching(body, value_start)?;
                        return Ok((value_start, value_end));
                    }
                }
                index = end;
            }
            b'{' | b'[' => {
                depth += 1;
                index += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    bail!("mod-list.json does not contain a mods array");
}

fn collect_spans(
    body: &[u8],
    array_open: usize,
    array_close: usize,
) -> Result<Vec<(usize, usize)>> {
    let mut spans = Vec::new();
    let mut index = array_open + 1;
    while index < array_close {
        index = skip_ws(body, index);
        if index >= array_close {
            break;
        }
        match body[index] {
            b'{' => {
                let end = find_matching(body, index)?;
                if end >= array_close {
                    bail!("mod-list.json contains an unterminated mod entry");
                }
                spans.push((index, end + 1));
                index = end + 1;
            }
            b',' => {
                index += 1;
            }
            _ => bail!("mod-list.json contains unexpected data in the mods array"),
        }
    }
    Ok(spans)
}

fn skip_ws(body: &[u8], mut index: usize) -> usize {
    while let Some(byte) = body.get(index) {
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            index += 1;
        } else {
            break;
        }
    }
    index
}

fn skip_string(body: &[u8], start: usize) -> Result<usize> {
    if body.get(start) != Some(&b'"') {
        bail!("expected a JSON string");
    }
    let mut index = start + 1;
    while index < body.len() {
        match body[index] {
            b'\\' => index += 2,
            b'"' => return Ok(index + 1),
            _ => index += 1,
        }
    }
    bail!("unterminated JSON string")
}

fn find_matching(body: &[u8], start: usize) -> Result<usize> {
    let close = match body.get(start) {
        Some(b'[') => b']',
        Some(b'{') => b'}',
        _ => bail!("expected a JSON array or object"),
    };
    let mut depth = 0i32;
    let mut index = start;
    while index < body.len() {
        match body[index] {
            b'"' => {
                index = skip_string(body, index)?;
                continue;
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    if body[index] != close {
                        bail!("mismatched JSON brackets");
                    }
                    return Ok(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    bail!("unterminated JSON array or object")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        b"\n{\n  \"mods\": \n  [\n    \n    {\n      \"name\": \"base\",\n      \"enabled\": true\n    },\n    \n    {\n      \"name\": \"other-mod\",\n      \"enabled\": false\n    }\n  ]\n}".to_vec()
    }

    #[test]
    fn appends_following_existing_style() -> Result<()> {
        let list = ModList::from_bytes(PathBuf::from("mod-list.json"), sample())?;
        let updated = list.append_disabled("my-pack")?.expect("changed");
        let text = String::from_utf8(updated.clone())?;
        assert!(text.ends_with(
            "    },\n    \n    {\n      \"name\": \"my-pack\",\n      \"enabled\": false\n    }\n  ]\n}"
        ));
        let reparsed = ModList::from_bytes(PathBuf::from("mod-list.json"), updated)?;
        assert!(reparsed.has_entry("my-pack"));
        assert_eq!(reparsed.is_enabled("my-pack"), Some(false));
        Ok(())
    }

    #[test]
    fn skips_existing_entry() -> Result<()> {
        let list = ModList::from_bytes(PathBuf::from("mod-list.json"), sample())?;
        assert!(list.append_disabled("base")?.is_none());
        Ok(())
    }

    #[test]
    fn removes_first_middle_and_last() -> Result<()> {
        for target in ["base", "other-mod"] {
            let list = ModList::from_bytes(PathBuf::from("mod-list.json"), sample())?;
            let updated = list.remove_entry(target)?.expect("changed");
            let reparsed = ModList::from_bytes(PathBuf::from("mod-list.json"), updated)?;
            assert!(!reparsed.has_entry(target));
            assert_eq!(reparsed.entries().len(), 1);
        }

        let source = br#"{"mods":[{"name":"a","enabled":true},{"name":"b","enabled":true},{"name":"c","enabled":true}]}"#.to_vec();
        let list = ModList::from_bytes(PathBuf::from("mod-list.json"), source)?;
        let updated = list.remove_entry("b")?.expect("changed");
        let text = String::from_utf8(updated)?;
        assert_eq!(
            text,
            r#"{"mods":[{"name":"a","enabled":true},{"name":"c","enabled":true}]}"#
        );
        Ok(())
    }

    #[test]
    fn preserves_bom_and_crlf() -> Result<()> {
        let mut source = vec![0xEF, 0xBB, 0xBF];
        source.extend_from_slice(
            b"{\r\n  \"mods\": [\r\n    {\r\n      \"name\": \"a\",\r\n      \"enabled\": true\r\n    }\r\n  ]\r\n}",
        );
        let list = ModList::from_bytes(PathBuf::from("mod-list.json"), source)?;
        let updated = list.append_disabled("pack")?.expect("changed");
        assert!(updated.starts_with(&[0xEF, 0xBB, 0xBF]));
        let text = String::from_utf8(updated)?;
        assert!(
            text.contains("\r\n      \"name\": \"pack\",\r\n      \"enabled\": false\r\n    }")
        );
        Ok(())
    }

    #[test]
    fn handles_empty_array() -> Result<()> {
        let list =
            ModList::from_bytes(PathBuf::from("mod-list.json"), br#"{"mods": []}"#.to_vec())?;
        let updated = list.append_disabled("pack")?.expect("changed");
        let reparsed = ModList::from_bytes(PathBuf::from("mod-list.json"), updated)?;
        assert!(reparsed.has_entry("pack"));
        assert_eq!(reparsed.entries().len(), 1);
        Ok(())
    }
}
