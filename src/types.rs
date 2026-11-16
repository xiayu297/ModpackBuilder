use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Version {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() != 3 {
            bail!("version must have exactly three numeric components");
        }
        let mut values = [0u16; 3];
        for (index, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                bail!("version component is not a number");
            }
            values[index] = part
                .parse()
                .map_err(|_| anyhow::anyhow!("version component exceeds 65535"))?;
        }
        Ok(Self::new(values[0], values[1], values[2]))
    }
}

fn parse_dependency_version(text: &str) -> Result<Version> {
    let text = text.trim();
    if text.split('.').count() == 2 {
        Version::parse(&format!("{text}.0"))
    } else {
        Version::parse(text)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct VersionVisitor;
        impl Visitor<'_> for VersionVisitor {
            type Value = Version;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a Factorio version such as 1.2.3")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Version, E> {
                Version::parse(value).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(VersionVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
}

impl CompareOp {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "=" => Some(Self::Equal),
            ">" => Some(Self::Greater),
            ">=" => Some(Self::GreaterEqual),
            "<" => Some(Self::Less),
            "<=" => Some(Self::LessEqual),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
            Self::Less => "<",
            Self::LessEqual => "<=",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReq {
    pub op: CompareOp,
    pub version: Version,
}

impl VersionReq {
    pub fn matches(&self, candidate: Version) -> bool {
        match self.op {
            CompareOp::Equal => candidate == self.version,
            CompareOp::Greater => candidate > self.version,
            CompareOp::GreaterEqual => candidate >= self.version,
            CompareOp::Less => candidate < self.version,
            CompareOp::LessEqual => candidate <= self.version,
        }
    }

    pub fn display(&self) -> String {
        format!("{} {}", self.op.as_str(), self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Required,
    Optional,
    HiddenOptional,
    Recommended,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub raw: String,
    pub kind: DepKind,
    pub name: String,
    pub req: Option<VersionReq>,
}

impl Dependency {
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("empty dependency");
        }
        let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("(?)") {
            (DepKind::HiddenOptional, rest)
        } else if let Some(rest) = trimmed.strip_prefix('!') {
            (DepKind::Conflict, rest)
        } else if let Some(rest) = trimmed.strip_prefix('?') {
            (DepKind::Optional, rest)
        } else if let Some(rest) = trimmed.strip_prefix('+') {
            (DepKind::Recommended, rest)
        } else if let Some(rest) = trimmed.strip_prefix('~') {
            (DepKind::Required, rest)
        } else {
            (DepKind::Required, trimmed)
        };
        let rest = rest.trim();
        let op_index = rest
            .char_indices()
            .find(|(_, ch)| matches!(ch, '<' | '>' | '='))
            .map(|(index, _)| index);
        let (name, req) = if let Some(index) = op_index {
            let name = rest[..index].trim();
            let op_text = rest[index..].trim();
            let mut chars = op_text.chars();
            let _ = chars.next();
            let op_len = if chars.next() == Some('=') { 2 } else { 1 };
            let op = CompareOp::parse(&op_text[..op_len])
                .ok_or_else(|| anyhow::anyhow!("invalid dependency operator"))?;
            let version_text = op_text[op_len..].trim();
            if version_text.is_empty() {
                bail!("dependency version requirement is empty");
            }
            (
                name,
                Some(VersionReq {
                    op,
                    version: parse_dependency_version(version_text)?,
                }),
            )
        } else {
            (rest, None)
        };
        if name.is_empty() {
            bail!("dependency name is empty");
        }
        if !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ' ')
        {
            bail!("dependency name contains unsupported characters");
        }
        Ok(Self {
            raw: raw.to_string(),
            kind,
            name: name.to_string(),
            req,
        })
    }

    pub fn required_version(&self) -> Option<String> {
        self.req.as_ref().map(VersionReq::display)
    }
}

#[derive(Debug, Clone)]
pub struct ModInfo {
    pub name: String,
    pub version: Version,
    pub title: String,
    pub author: String,
    pub description: String,
    pub factorio_version: String,
    pub dependencies: Vec<Dependency>,
    pub invalid_dependencies: Vec<String>,
}

#[derive(Deserialize)]
struct RawInfo {
    name: String,
    version: String,
    title: Option<String>,
    author: Option<String>,
    description: Option<String>,
    factorio_version: Option<String>,
    dependencies: Option<Vec<String>>,
}

impl ModInfo {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let raw: RawInfo = serde_json::from_slice(bytes)?;
        if raw.name.trim().is_empty() {
            bail!("info.json has an empty name");
        }
        let version = Version::parse(&raw.version)?;
        let mut dependencies = Vec::new();
        let mut invalid_dependencies = Vec::new();
        for entry in raw.dependencies.unwrap_or_default() {
            match Dependency::parse(&entry) {
                Ok(dependency) => dependencies.push(dependency),
                Err(_) => invalid_dependencies.push(entry),
            }
        }
        let title = raw
            .title
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| raw.name.clone());
        Ok(Self {
            name: raw.name,
            version,
            title,
            author: raw.author.unwrap_or_default(),
            description: raw.description.unwrap_or_default(),
            factorio_version: raw.factorio_version.unwrap_or_default(),
            dependencies,
            invalid_dependencies,
        })
    }

    pub fn is_target_version(&self) -> bool {
        self.factorio_version == "2.1"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModSource {
    Zip(PathBuf),
    Folder(PathBuf),
    Builtin,
}

impl ModSource {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Zip(path) => Some(path),
            Self::Folder(path) => Some(path),
            Self::Builtin => None,
        }
    }

    pub fn is_zip(&self) -> bool {
        matches!(self, Self::Zip(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkerMember {
    pub name: String,
    pub version: Version,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub format: u32,
    pub tool: String,
    pub internal_name: String,
    pub title: String,
    pub author: String,
    pub version: Version,
    pub created_at: String,
    pub updated_at: String,
    pub members: Vec<MarkerMember>,
}

impl Marker {
    pub const FORMAT: u32 = 1;
    pub const TOOL: &str = "ModpackBuilder";
    pub const FILE_NAME: &str = "module-set-builder.json";

    pub fn is_recognized(&self) -> bool {
        self.format == Self::FORMAT && self.tool == Self::TOOL
    }
}

#[derive(Debug, Clone)]
pub struct InstalledMod {
    pub info: ModInfo,
    pub source: ModSource,
    pub enabled: bool,
    pub marker: Option<Marker>,
    pub duplicate_count: usize,
}

impl InstalledMod {
    pub fn name(&self) -> &str {
        &self.info.name
    }

    pub fn title(&self) -> &str {
        &self.info.title
    }

    pub fn is_builtin(&self) -> bool {
        matches!(self.source, ModSource::Builtin)
    }

    pub fn is_pack(&self) -> bool {
        self.marker.is_some()
    }

    pub fn is_folder_pack(&self) -> bool {
        self.marker.is_some() && matches!(self.source, ModSource::Folder(_))
    }

    pub fn marker(&self) -> Option<&Marker> {
        self.marker.as_ref()
    }
}

impl Marker {
    pub fn from_parts(
        internal_name: &str,
        title: &str,
        author: &str,
        version: Version,
        created_at: &str,
        updated_at: &str,
        members: &[MemberRef],
    ) -> Self {
        Self {
            format: Self::FORMAT,
            tool: Self::TOOL.to_string(),
            internal_name: internal_name.to_string(),
            title: title.to_string(),
            author: author.to_string(),
            version,
            created_at: created_at.to_string(),
            updated_at: updated_at.to_string(),
            members: members
                .iter()
                .map(|member| MarkerMember {
                    name: member.name.clone(),
                    version: member.version,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRef {
    pub name: String,
    pub version: Version,
}

#[derive(Debug, Clone)]
pub struct PackSpec {
    pub internal_name: String,
    pub title: String,
    pub version: Version,
    pub author: String,
    pub members: Vec<MemberRef>,
    pub created_at: String,
    pub updated_at: String,
}

impl PackSpec {
    pub fn sorted_members(&self) -> Vec<MemberRef> {
        let mut members = self.members.clone();
        members.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });
        members
    }

    pub fn member_map(&self) -> BTreeMap<String, MemberRef> {
        self.members
            .iter()
            .map(|member| (member.name.to_ascii_lowercase(), member.clone()))
            .collect()
    }
}

pub const BUILTIN_MODS: &[(&str, &str, &str)] = &[
    ("base", "Base Mod", "基础模组"),
    ("elevated-rails", "Elevated Rails", "高架铁路"),
    ("quality", "Quality", "品质"),
    ("recycler", "Recycler", "回收机"),
    ("space-age", "Space Age", "太空时代"),
];

pub fn is_builtin_name(name: &str) -> bool {
    BUILTIN_MODS
        .iter()
        .any(|(builtin, _, _)| builtin.eq_ignore_ascii_case(name))
}

pub fn builtin_info(name: &str) -> Option<ModInfo> {
    BUILTIN_MODS
        .iter()
        .find(|(builtin, _, _)| builtin.eq_ignore_ascii_case(name))
        .map(|(builtin, title_en, _)| ModInfo {
            name: (*builtin).to_string(),
            version: Version::new(2, 1, 0),
            title: (*title_en).to_string(),
            author: "Factorio".to_string(),
            description: String::new(),
            factorio_version: "2.1".to_string(),
            dependencies: Vec::new(),
            invalid_dependencies: Vec::new(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_dependency_prefixes() {
        assert_eq!(
            Dependency::parse("? optional-mod").unwrap().kind,
            DepKind::Optional
        );
        assert_eq!(
            Dependency::parse("(?) hidden-optional").unwrap().kind,
            DepKind::HiddenOptional
        );
        assert_eq!(
            Dependency::parse("+ recommended-mod").unwrap().kind,
            DepKind::Recommended
        );
        let conflict = Dependency::parse("! Squeak Through").unwrap();
        assert_eq!(conflict.kind, DepKind::Conflict);
        assert_eq!(conflict.name, "Squeak Through");
        assert_eq!(
            Dependency::parse("~ unordered-mod").unwrap().kind,
            DepKind::Required
        );
        assert_eq!(
            Dependency::parse("plain-mod").unwrap().kind,
            DepKind::Required
        );
        assert!(Dependency::parse("(!) unsupported").is_err());
    }

    #[test]
    fn accepts_two_component_dependency_versions_only() {
        let dependency = Dependency::parse("base >= 2.1").unwrap();
        assert_eq!(dependency.req.unwrap().version, Version::new(2, 1, 0));
        assert!(Dependency::parse("base >= 2").is_err());
        assert!(Version::parse("2.1").is_err());
    }

    #[test]
    fn parses_version_requirements() {
        let dependency = Dependency::parse("alpha >= 2.3.4").unwrap();
        let requirement = dependency.req.unwrap();
        assert_eq!(requirement.op, CompareOp::GreaterEqual);
        assert_eq!(requirement.version, Version::new(2, 3, 4));
        assert!(requirement.matches(Version::new(2, 3, 4)));
        assert!(requirement.matches(Version::new(3, 0, 0)));
        assert!(!requirement.matches(Version::new(2, 3, 3)));
    }
}
