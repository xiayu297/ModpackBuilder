use std::collections::BTreeMap;

use crate::modscan::ScanResult;
use crate::types::{DepKind, Dependency, InstalledMod, MemberRef, Version};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueKind {
    IncompatibleTarget {
        found: String,
    },
    MissingMember {
        version: Version,
    },
    InvalidDependency {
        raw: String,
    },
    MissingRequired {
        dep: String,
        required: Option<String>,
    },
    DependencyNotIncluded {
        dep: String,
        enabled: bool,
    },
    DependencyVersion {
        dep: String,
        required: String,
        found: Version,
    },
    OptionalVersion {
        dep: String,
        required: String,
        found: Version,
    },
    RecommendedMissing {
        dep: String,
    },
    RecommendedVersion {
        dep: String,
        required: String,
        found: Version,
    },
    ConflictSelected {
        dep: String,
    },
    ConflictInstalled {
        dep: String,
        enabled: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Hard,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub member: String,
    pub kind: IssueKind,
}

impl Issue {
    pub fn severity(&self) -> Severity {
        match &self.kind {
            IssueKind::IncompatibleTarget { .. } | IssueKind::ConflictSelected { .. } => {
                Severity::Hard
            }
            _ => Severity::Warning,
        }
    }

    pub fn is_hard(&self) -> bool {
        self.severity() == Severity::Hard
    }
}

pub fn hard_count(issues: &[Issue]) -> usize {
    issues.iter().filter(|issue| issue.is_hard()).count()
}

pub fn warning_count(issues: &[Issue]) -> usize {
    issues.iter().filter(|issue| !issue.is_hard()).count()
}

pub fn check(members: &[MemberRef], scan: &ScanResult) -> Vec<Issue> {
    let installed = scan.installed_map();
    let selected: BTreeMap<String, Version> = members
        .iter()
        .map(|member| (member.name.to_ascii_lowercase(), member.version))
        .collect();
    let mut issues = Vec::new();

    for member in members {
        let key = member.name.to_ascii_lowercase();
        let Some(installed_mod) = installed.get(&key).copied() else {
            issues.push(Issue {
                member: member.name.clone(),
                kind: IssueKind::MissingMember {
                    version: member.version,
                },
            });
            continue;
        };
        if !installed_mod.info.is_target_version() {
            issues.push(Issue {
                member: member.name.clone(),
                kind: IssueKind::IncompatibleTarget {
                    found: installed_mod.info.factorio_version.clone(),
                },
            });
        }
        for raw in &installed_mod.info.invalid_dependencies {
            issues.push(Issue {
                member: member.name.clone(),
                kind: IssueKind::InvalidDependency { raw: raw.clone() },
            });
        }
        for dependency in &installed_mod.info.dependencies {
            inspect_dependency(member, dependency, &selected, &installed, &mut issues);
        }
    }

    issues
}

fn inspect_dependency(
    member: &MemberRef,
    dependency: &Dependency,
    selected: &BTreeMap<String, Version>,
    installed: &BTreeMap<String, &InstalledMod>,
    issues: &mut Vec<Issue>,
) {
    let dep_key = dependency.name.to_ascii_lowercase();
    if installed
        .get(&dep_key)
        .is_some_and(|mod_info| mod_info.is_builtin())
    {
        return;
    }
    match dependency.kind {
        DepKind::Conflict => {
            if selected.contains_key(&dep_key) {
                issues.push(Issue {
                    member: member.name.clone(),
                    kind: IssueKind::ConflictSelected {
                        dep: dependency.name.clone(),
                    },
                });
            } else if let Some(other) = installed.get(&dep_key) {
                issues.push(Issue {
                    member: member.name.clone(),
                    kind: IssueKind::ConflictInstalled {
                        dep: dependency.name.clone(),
                        enabled: other.enabled,
                    },
                });
            }
        }
        DepKind::Required => {
            if let Some(version) = selected.get(&dep_key) {
                if let Some(req) = &dependency.req
                    && !req.matches(*version)
                {
                    issues.push(Issue {
                        member: member.name.clone(),
                        kind: IssueKind::DependencyVersion {
                            dep: dependency.name.clone(),
                            required: req.display(),
                            found: *version,
                        },
                    });
                }
            } else if let Some(other) = installed.get(&dep_key) {
                issues.push(Issue {
                    member: member.name.clone(),
                    kind: IssueKind::DependencyNotIncluded {
                        dep: dependency.name.clone(),
                        enabled: other.enabled,
                    },
                });
                if let Some(req) = &dependency.req
                    && !req.matches(other.info.version)
                {
                    issues.push(Issue {
                        member: member.name.clone(),
                        kind: IssueKind::DependencyVersion {
                            dep: dependency.name.clone(),
                            required: req.display(),
                            found: other.info.version,
                        },
                    });
                }
            } else {
                issues.push(Issue {
                    member: member.name.clone(),
                    kind: IssueKind::MissingRequired {
                        dep: dependency.name.clone(),
                        required: dependency.required_version(),
                    },
                });
            }
        }
        DepKind::Optional | DepKind::HiddenOptional => {
            if let Some(version) = selected.get(&dep_key) {
                if let Some(req) = &dependency.req
                    && !req.matches(*version)
                    && matches!(dependency.kind, DepKind::Optional | DepKind::HiddenOptional)
                {
                    issues.push(Issue {
                        member: member.name.clone(),
                        kind: IssueKind::OptionalVersion {
                            dep: dependency.name.clone(),
                            required: req.display(),
                            found: *version,
                        },
                    });
                }
            } else if let Some(other) = installed.get(&dep_key)
                && let Some(req) = &dependency.req
                && !req.matches(other.info.version)
                && matches!(dependency.kind, DepKind::Optional | DepKind::HiddenOptional)
            {
                issues.push(Issue {
                    member: member.name.clone(),
                    kind: IssueKind::OptionalVersion {
                        dep: dependency.name.clone(),
                        required: req.display(),
                        found: other.info.version,
                    },
                });
            }
        }
        DepKind::Recommended => {
            if let Some(version) = selected.get(&dep_key) {
                if let Some(req) = &dependency.req
                    && !req.matches(*version)
                {
                    issues.push(Issue {
                        member: member.name.clone(),
                        kind: IssueKind::RecommendedVersion {
                            dep: dependency.name.clone(),
                            required: req.display(),
                            found: *version,
                        },
                    });
                }
            } else if let Some(other) = installed.get(&dep_key) {
                if let Some(req) = &dependency.req
                    && !req.matches(other.info.version)
                {
                    issues.push(Issue {
                        member: member.name.clone(),
                        kind: IssueKind::RecommendedVersion {
                            dep: dependency.name.clone(),
                            required: req.display(),
                            found: other.info.version,
                        },
                    });
                }
            } else {
                issues.push(Issue {
                    member: member.name.clone(),
                    kind: IssueKind::RecommendedMissing {
                        dep: dependency.name.clone(),
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModInfo, ModSource};

    fn installed(name: &str, version: Version, deps: &[&str]) -> InstalledMod {
        let mut parsed = Vec::new();
        for dep in deps {
            parsed.push(Dependency::parse(dep).unwrap());
        }
        InstalledMod {
            info: ModInfo {
                name: name.to_string(),
                version,
                title: name.to_string(),
                author: String::new(),
                description: String::new(),
                factorio_version: "2.1".to_string(),
                dependencies: parsed,
                invalid_dependencies: Vec::new(),
            },
            source: ModSource::Folder(std::path::PathBuf::from(name)),
            enabled: false,
            marker: None,
            duplicate_count: 1,
        }
    }

    #[test]
    fn flags_conflict_between_members_as_hard() {
        let scan = ScanResult {
            mods: vec![
                installed(
                    "squeak-through-2",
                    Version::new(1, 0, 0),
                    &["! Squeak Through"],
                ),
                installed("Squeak Through", Version::new(1, 0, 0), &[]),
            ],
            warnings: Vec::new(),
        };
        let members = vec![
            MemberRef {
                name: "squeak-through-2".to_string(),
                version: Version::new(1, 0, 0),
            },
            MemberRef {
                name: "Squeak Through".to_string(),
                version: Version::new(1, 0, 0),
            },
        ];
        let issues = check(&members, &scan);
        assert_eq!(hard_count(&issues), 1);
    }

    #[test]
    fn checks_installed_conflicts_without_blocking_the_pack() {
        let members = vec![MemberRef {
            name: "squeak-through-2".to_string(),
            version: Version::new(1, 0, 0),
        }];
        let mut legacy = installed("Squeak Through", Version::new(1, 0, 0), &[]);
        legacy.enabled = true;
        let scan = ScanResult {
            mods: vec![
                installed(
                    "squeak-through-2",
                    Version::new(1, 0, 0),
                    &["! Squeak Through"],
                ),
                legacy,
            ],
            warnings: Vec::new(),
        };
        let issues = check(&members, &scan);
        assert_eq!(hard_count(&issues), 0);
        assert!(issues.iter().any(|issue| matches!(
            &issue.kind,
            IssueKind::ConflictInstalled { dep, enabled }
                if dep == "Squeak Through" && *enabled
        )));

        let without_legacy = ScanResult {
            mods: vec![installed(
                "squeak-through-2",
                Version::new(1, 0, 0),
                &["! Squeak Through"],
            )],
            warnings: Vec::new(),
        };
        assert!(check(&members, &without_legacy).is_empty());
    }

    #[test]
    fn ignores_builtin_dependencies() {
        let mut base = installed("base", Version::new(2, 1, 0), &[]);
        base.source = ModSource::Builtin;
        let scan = ScanResult {
            mods: vec![
                installed("alpha", Version::new(1, 0, 0), &["base >= 2.1"]),
                base,
            ],
            warnings: Vec::new(),
        };
        let members = vec![MemberRef {
            name: "alpha".to_string(),
            version: Version::new(1, 0, 0),
        }];
        assert!(check(&members, &scan).is_empty());
    }

    #[test]
    fn warns_missing_required_dependency() {
        let scan = ScanResult {
            mods: vec![installed(
                "alpha",
                Version::new(1, 0, 0),
                &["gamma >= 2.0.0"],
            )],
            warnings: Vec::new(),
        };
        let members = vec![MemberRef {
            name: "alpha".to_string(),
            version: Version::new(1, 0, 0),
        }];
        let issues = check(&members, &scan);
        assert!(matches!(issues[0].kind, IssueKind::MissingRequired { .. }));
        assert_eq!(hard_count(&issues), 0);
    }
}
