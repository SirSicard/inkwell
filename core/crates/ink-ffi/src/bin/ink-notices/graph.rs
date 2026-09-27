//! The crates the release links into the core, from cargo's own resolution.
//!
//! `cargo tree -p ink-ffi -e normal,no-proc-macro` resolves features exactly as the release build
//! (`cargo rustc -p ink-ffi`) does: only ink-ffi's features and its normal dependencies. `cargo
//! metadata` alone cannot: its resolve unifies the features of the whole workspace, dev-dependencies
//! included, and so names crates the core never compiles. So the tree gives the set, and the
//! metadata gives each crate's licence, authors, source and package directory.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

/// The crates.io source as `cargo metadata` names it (for the sparse protocol too).
pub const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// A crate from `cargo metadata`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Its licence expression, if it declares one.
    pub licence: Option<String>,
    /// Its authors, as its manifest lists them.
    pub authors: Vec<String>,
    /// Where it comes from (`None` for a path dependency).
    pub source: Option<String>,
    /// Its unpacked package (the directory of its manifest).
    pub dir: PathBuf,
    /// Whether it is one of the workspace's own crates.
    pub workspace: bool,
}

/// Reads `cargo metadata --format-version 1` output: every package by name and version.
pub fn packages(metadata: &str) -> Result<BTreeMap<(String, String), Vec<Package>>, String> {
    let v: Value =
        serde_json::from_str(metadata).map_err(|e| format!("cargo metadata's output: {e}"))?;
    let members: Vec<&str> = v["workspace_members"]
        .as_array()
        .ok_or("cargo metadata: no workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut out: BTreeMap<(String, String), Vec<Package>> = BTreeMap::new();
    for p in v["packages"]
        .as_array()
        .ok_or("cargo metadata: no packages")?
    {
        let field = |k: &str| p[k].as_str().map(str::to_string);
        let (Some(id), Some(name), Some(version), Some(manifest)) = (
            field("id"),
            field("name"),
            field("version"),
            field("manifest_path"),
        ) else {
            return Err(
                "cargo metadata: a package without id, name, version or manifest_path".into(),
            );
        };
        let dir = PathBuf::from(manifest)
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| {
                format!("cargo metadata: {name} {version}'s manifest has no directory")
            })?;
        let authors = p["authors"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let package = Package {
            workspace: members.contains(&id.as_str()),
            name: name.clone(),
            version: version.clone(),
            licence: field("license"),
            authors,
            source: field("source"),
            dir,
        };
        out.entry((name, version)).or_default().push(package);
    }
    Ok(out)
}

/// Reads `cargo tree --prefix none --format '{p}'` output: each crate once, as (name, version).
/// A line reads `name vX.Y.Z`, then its source if it is not crates.io, then ` (*)` when the
/// tree already showed it.
pub fn tree(output: &str) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in output.lines().filter(|l| !l.trim().is_empty()) {
        let mut words = line.split_whitespace();
        let (Some(name), Some(version)) = (words.next(), words.next()) else {
            return Err(format!("cargo tree: cannot read `{line}`"));
        };
        let Some(version) = version.strip_prefix('v') else {
            return Err(format!("cargo tree: no version in `{line}`"));
        };
        let key = (name.to_string(), version.to_string());
        if !out.contains(&key) {
            out.push(key);
        }
    }
    Ok(out)
}

/// The third-party crates of the release: each crate of the tree, matched to its package, the
/// workspace's own left out. Errors name every crate that cannot be matched or does not come
/// from crates.io.
pub fn release_crates(
    tree: &[(String, String)],
    packages: &BTreeMap<(String, String), Vec<Package>>,
) -> Result<Vec<Package>, Vec<String>> {
    let mut crates = Vec::new();
    let mut errors = Vec::new();
    for (name, version) in tree {
        let found = packages
            .get(&(name.clone(), version.clone()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        match found {
            [p] if p.workspace => {}
            [p] if p.source.as_deref() == Some(CRATES_IO) => crates.push(p.clone()),
            [p] => errors.push(format!(
                "{name} {version}: comes from {}, not crates.io; its notice needs a decision",
                p.source
                    .as_deref()
                    .unwrap_or("a path outside the workspace")
            )),
            [] => errors.push(format!(
                "{name} {version}: in cargo tree but not in cargo metadata"
            )),
            _ => errors.push(format!(
                "{name} {version}: more than one package by that name and version"
            )),
        }
    }
    if errors.is_empty() {
        Ok(crates)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> String {
        serde_json::json!({
            "workspace_members": ["path+file:///w/crates/ink-ffi#0.0.0"],
            "packages": [
                {"id": "path+file:///w/crates/ink-ffi#0.0.0", "name": "ink-ffi", "version": "0.0.0",
                 "license": "MIT", "authors": [], "source": null, "manifest_path": "/w/crates/ink-ffi/Cargo.toml"},
                {"id": "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0", "name": "serde",
                 "version": "1.0.0", "license": "MIT OR Apache-2.0", "authors": ["A <a@b.c>"],
                 "source": CRATES_IO, "manifest_path": "/r/serde-1.0.0/Cargo.toml"},
                {"id": "git+https://example.com/x#1", "name": "x", "version": "0.1.0", "license": "MIT",
                 "authors": [], "source": "git+https://example.com/x#1", "manifest_path": "/g/x/Cargo.toml"}
            ]
        })
        .to_string()
    }

    #[test]
    fn the_tree_lists_each_crate_once() {
        let out =
            "ink-ffi v0.0.0 (/w/crates/ink-ffi)\nserde v1.0.0\nlog v0.4.1\nserde v1.0.0 (*)\n\n";
        assert_eq!(
            tree(out).unwrap(),
            [("ink-ffi", "0.0.0"), ("serde", "1.0.0"), ("log", "0.4.1")]
                .map(|(n, v)| (n.to_string(), v.to_string()))
        );
        assert!(tree("serde 1.0.0\n").is_err(), "a version without its v");
    }

    #[test]
    fn the_release_is_the_trees_third_party_crates_with_their_packages() {
        let packages = packages(&metadata()).unwrap();
        let t = tree("ink-ffi v0.0.0 (/w/crates/ink-ffi)\nserde v1.0.0\n").unwrap();
        let crates = release_crates(&t, &packages).unwrap();
        assert_eq!(crates.len(), 1, "the workspace's own crate is left out");
        assert_eq!(crates[0].name, "serde");
        assert_eq!(crates[0].dir, PathBuf::from("/r/serde-1.0.0"));
        assert_eq!(crates[0].authors, ["A <a@b.c>"]);
    }

    #[test]
    fn a_crate_from_elsewhere_or_unknown_is_refused_by_name() {
        let packages = packages(&metadata()).unwrap();
        let t = tree("x v0.1.0 (https://example.com/x#1)\nghost v9.9.9\nserde v1.0.0\n").unwrap();
        let errors = release_crates(&t, &packages).unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].starts_with("x 0.1.0: comes from git+"),
            "{errors:?}"
        );
        assert!(
            errors[1].starts_with("ghost 9.9.9: in cargo tree but not"),
            "{errors:?}"
        );
    }
}
