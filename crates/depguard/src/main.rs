use anyhow::{bail, Context, Result};
use cargo_metadata::{DependencyKind, MetadataCommand, Package};
use std::collections::{BTreeMap, BTreeSet};

fn main() {
    if let Err(err) = real_main() {
        eprintln!("rdp-depguard: {err:?}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("check") => {}
        Some(other) => bail!("unknown command: {other}. expected: check"),
        None => bail!("missing command. expected: check"),
    }

    let metadata = MetadataCommand::new()
        .exec()
        .context("failed to run cargo metadata")?;

    let packages_by_name: BTreeMap<&str, &Package> = metadata
        .packages
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();

    let mut failures: Vec<String> = Vec::new();

    // Layer names
    let domain = ["rd-interface", "rd-derive"];
    let core = ["rabbit-digger"];
    let bundle = ["rdp-bundle"];
    let app = ["rabbit-digger-pro"];
    let adapters = ["rd-std", "ss", "trojan", "rpc", "raw", "obfs"];

    let protocol_crates: BTreeSet<&str> = ["ss", "trojan", "rpc", "raw", "obfs"].into();
    let non_domain_crates: BTreeSet<&str> = [
        "rabbit-digger",
        "rdp-bundle",
        "rabbit-digger-pro",
        "rd-std",
        "ss",
        "trojan",
        "rpc",
        "raw",
        "obfs",
        "rdp",
    ]
    .into();

    // Domain must not directly depend on Core/Adapters/App/Bundle.
    for pkg_name in domain {
        if let Some(pkg) = packages_by_name.get(pkg_name) {
            let deps = normal_deps(pkg);
            for dep in deps {
                if non_domain_crates.contains(dep.as_str()) {
                    failures.push(format!(
                        "{pkg_name} must not depend on {dep} (domain -> non-domain)"
                    ));
                }
            }
        }
    }

    // Core should not depend on App/Bundle/Protocol crates; rd-std must be optional if present.
    for pkg_name in core {
        if let Some(pkg) = packages_by_name.get(pkg_name) {
            for dep in pkg
                .dependencies
                .iter()
                .filter(|d| d.kind == DependencyKind::Normal)
            {
                let dep_name = dep.name.as_str();
                if dep_name == "rd-std" {
                    if !dep.optional {
                        failures.push(format!(
                            "{pkg_name} dependency rd-std must be optional (to allow core-only build)"
                        ));
                    }
                    continue;
                }
                if dep_name == "rabbit-digger-pro" {
                    failures.push(format!(
                        "{pkg_name} must not depend on app crate rabbit-digger-pro"
                    ));
                }
                if dep_name == "rdp-bundle" {
                    failures.push(format!(
                        "{pkg_name} must not depend on bundle crate rdp-bundle"
                    ));
                }
                if protocol_crates.contains(dep_name) {
                    failures.push(format!(
                        "{pkg_name} must not depend on protocol crate {dep_name}"
                    ));
                }
            }
        }
    }

    // Adapters must not depend on Core/App/Bundle.
    for pkg_name in adapters {
        if let Some(pkg) = packages_by_name.get(pkg_name) {
            let deps = normal_deps(pkg);
            for dep in deps {
                if dep == "rabbit-digger" || dep == "rabbit-digger-pro" || dep == "rdp-bundle" {
                    failures.push(format!(
                        "{pkg_name} must not depend on {dep} (adapters -> core/app/bundle)"
                    ));
                }
            }
        }
    }

    // Bundle must not depend on App/FFI.
    for pkg_name in bundle {
        if let Some(pkg) = packages_by_name.get(pkg_name) {
            let deps = normal_deps(pkg);
            for dep in deps {
                if dep == "rabbit-digger-pro" || dep == "rdp" {
                    failures.push(format!("{pkg_name} must not depend on {dep}"));
                }
            }
        }
    }

    // App must not directly depend on protocol crates.
    for pkg_name in app {
        if let Some(pkg) = packages_by_name.get(pkg_name) {
            let deps = normal_deps(pkg);
            for dep in deps {
                if protocol_crates.contains(dep.as_str()) {
                    failures.push(format!("{pkg_name} must not directly depend on {dep}"));
                }
            }
        }
    }

    if failures.is_empty() {
        return Ok(());
    }

    failures.sort();
    let mut msg = String::new();
    msg.push_str("dependency direction violations:\n");
    for f in failures {
        msg.push_str("- ");
        msg.push_str(&f);
        msg.push('\n');
    }

    bail!(msg);
}

fn normal_deps(pkg: &Package) -> BTreeSet<String> {
    pkg.dependencies
        .iter()
        .filter(|d| d.kind == DependencyKind::Normal)
        .map(|d| d.name.clone())
        .collect()
}
