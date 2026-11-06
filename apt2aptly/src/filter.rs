//! Selecting packages to leave out of a sync

use std::{fmt, str::FromStr};

use aptly_rest::key::AptlyKey;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Source,
    Binaries,
    Arch(String),
}

/// A package selected by name, written as `[COMPONENT:]NAME[=WHAT,…]`
///
/// `WHAT` is `source` for the source package, `binaries` for binary packages
/// of every architecture, or an architecture for binary packages of that
/// architecture only. Without `=WHAT`, both the source and binary packages
/// called `NAME` are selected. Without `COMPONENT:`, every component is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFilter {
    component: Option<String>,
    package: String,
    selections: Vec<Selection>,
}

fn is_valid_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-.".contains(c))
}

fn is_valid_arch(arch: &str) -> bool {
    !arch.is_empty()
        && arch
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl FromStr for PackageFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (selector, what) = match s.split_once('=') {
            Some((selector, what)) => (selector, Some(what)),
            None => (s, None),
        };
        let (component, package) = match selector.split_once(':') {
            Some((component, package)) => (Some(component), package),
            None => (None, selector),
        };

        if component.is_some_and(str::is_empty) {
            return Err(format!("Empty component in '{s}'"));
        }
        if !is_valid_name(package) {
            return Err(format!("Invalid package name '{package}' in '{s}'"));
        }

        let selections = what
            .map(|what| {
                what.split(',')
                    .map(|item| match item {
                        "source" => Ok(Selection::Source),
                        "binaries" => Ok(Selection::Binaries),
                        arch if is_valid_arch(arch) => Ok(Selection::Arch(arch.to_owned())),
                        _ => Err(format!("Invalid selection '{item}' in '{s}'")),
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            component: component.map(ToOwned::to_owned),
            package: package.to_owned(),
            selections,
        })
    }
}

impl fmt::Display for PackageFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(component) = &self.component {
            write!(f, "{component}:")?;
        }
        write!(f, "{}", self.package)?;
        for (i, selection) in self.selections.iter().enumerate() {
            f.write_str(if i == 0 { "=" } else { "," })?;
            match selection {
                Selection::Source => f.write_str("source")?,
                Selection::Binaries => f.write_str("binaries")?,
                Selection::Arch(arch) => f.write_str(arch)?,
            }
        }
        Ok(())
    }
}

impl PackageFilter {
    fn selects(&self, component: &str, package: &str) -> bool {
        self.component.as_deref().is_none_or(|c| c == component) && self.package == package
    }

    pub fn matches_source(&self, component: &str, package: &str) -> bool {
        self.selects(component, package)
            && (self.selections.is_empty() || self.selections.contains(&Selection::Source))
    }

    pub fn matches_binary(&self, component: &str, package: &str, arch: &str) -> bool {
        self.selects(component, package)
            && (self.selections.is_empty()
                || self.selections.iter().any(|s| match s {
                    Selection::Source => false,
                    Selection::Binaries => true,
                    Selection::Arch(a) => a == arch,
                }))
    }

    pub fn matches_key(&self, component: &str, key: &AptlyKey) -> bool {
        if key.is_binary() {
            self.matches_binary(component, key.package(), key.arch())
        } else {
            self.matches_source(component, key.package())
        }
    }
}

/// Packages to leave out of a sync
#[derive(clap::Args, Debug, Default, Clone)]
pub struct PackageFilters {
    /// Leave packages alone: don't add them to aptly or remove them from it.
    /// Format: [COMPONENT:]NAME[=WHAT,…], where WHAT is `source`, `binaries`
    /// or an architecture; without =WHAT, both source and binary packages
    /// called NAME are selected. Can be given several times.
    #[arg(long, value_name = "FILTER")]
    pub skip: Vec<PackageFilter>,
    /// Drop packages: don't add them to aptly, and remove them if they are
    /// already there. Same format as --skip.
    #[arg(long, value_name = "FILTER")]
    pub drop: Vec<PackageFilter>,
}

impl PackageFilters {
    fn origin_filters(&self) -> impl Iterator<Item = &PackageFilter> {
        self.skip.iter().chain(&self.drop)
    }

    /// Whether to leave this source package out of the apt repository's content
    pub fn ignores_origin_source(&self, component: &str, package: &str) -> bool {
        self.origin_filters()
            .any(|f| f.matches_source(component, package))
    }

    /// Whether to leave this binary package out of the apt repository's content
    pub fn ignores_origin_binary(&self, component: &str, package: &str, arch: &str) -> bool {
        self.origin_filters()
            .any(|f| f.matches_binary(component, package, arch))
    }

    /// Whether to leave this package out of the aptly repository's content
    pub fn ignores_aptly_key(&self, component: &str, key: &AptlyKey) -> bool {
        self.skip.iter().any(|f| f.matches_key(component, key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(s: &str) -> PackageFilter {
        s.parse().unwrap()
    }

    #[test]
    fn parse() {
        assert_eq!(
            filter("target:elfutils=source"),
            PackageFilter {
                component: Some("target".to_owned()),
                package: "elfutils".to_owned(),
                selections: vec![Selection::Source],
            }
        );
        assert_eq!(
            filter("libelf1=amd64,binaries"),
            PackageFilter {
                component: None,
                package: "libelf1".to_owned(),
                selections: vec![Selection::Arch("amd64".to_owned()), Selection::Binaries],
            }
        );
        assert_eq!(
            filter("updates/main:g++-12"),
            PackageFilter {
                component: Some("updates/main".to_owned()),
                package: "g++-12".to_owned(),
                selections: vec![],
            }
        );
    }

    #[test]
    fn parse_invalid() {
        for s in [
            "",
            ":elfutils",
            "target:",
            "Elfutils",
            "elfutils=",
            "elfutils=source,",
            "elfutils=AMD64",
            "elf*",
        ] {
            assert!(s.parse::<PackageFilter>().is_err(), "'{s}' was accepted");
        }
    }

    #[test]
    fn display_round_trip() {
        for s in [
            "elfutils",
            "target:elfutils=source",
            "libelf1=amd64,binaries",
        ] {
            assert_eq!(filter(s).to_string(), s);
        }
    }

    #[test]
    fn matching() {
        let f = filter("target:elfutils=source");
        assert!(f.matches_source("target", "elfutils"));
        assert!(!f.matches_source("development", "elfutils"));
        assert!(!f.matches_source("target", "libelf1"));
        assert!(!f.matches_binary("target", "elfutils", "amd64"));

        let f = filter("elfutils");
        assert!(f.matches_source("target", "elfutils"));
        assert!(f.matches_source("development", "elfutils"));
        assert!(f.matches_binary("target", "elfutils", "amd64"));
        assert!(f.matches_binary("target", "elfutils", "all"));
        assert!(!f.matches_binary("target", "libelf1", "amd64"));

        let f = filter("libelf1=amd64");
        assert!(f.matches_binary("target", "libelf1", "amd64"));
        assert!(!f.matches_binary("target", "libelf1", "arm64"));
        assert!(!f.matches_source("target", "libelf1"));

        let f = filter("libelf1=binaries");
        assert!(f.matches_binary("target", "libelf1", "arm64"));
        assert!(!f.matches_source("target", "libelf1"));
    }
}
