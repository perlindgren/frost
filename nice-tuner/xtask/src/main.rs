//! Plugin bundler entry point: `cargo xtask bundle nice-tuner --release`.
//!
//! Native bundle assembly uses nice-plug-xtask public APIs. This wrapper keeps
//! nested worktrees in their own workspace and ships embedded font licences.

mod archive;
mod notices;
mod release;
mod standalone;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// Texts shipped with every plugin bundle and standalone release directory.
/// Both fonts are embedded in the editors; their original notices must travel
/// with the executable even though the font files are inside the binary.
const COMMON_LICENCES: &[(&str, &str)] = &[
    ("LICENSE-MIT", "LICENSE-MIT"),
    ("LICENSE-APACHE", "LICENSE-APACHE"),
    ("docs/plugin-inspiration.md", "PLUGIN-INSPIRATION.md"),
    ("docs/NON-AFFILIATION.txt", "NON-AFFILIATION.txt"),
    ("assets/fonts/OFL.txt", "OFL.txt"),
    (
        "nice-common/assets/fonts/LICENSE-FluentSystemIcons.txt",
        "LICENSE-FluentSystemIcons.txt",
    ),
];

fn main() -> anyhow::Result<()> {
    // Cargo knows the nearest workspace. The upstream bundler instead picks
    // the outermost Cargo.toml, which leaves a worktree nested in a checkout.
    // Use the public build/bundle APIs from Cargo's actual workspace root.
    let located = std::process::Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()
        .context("could not locate the Cargo workspace")?;
    if !located.status.success() {
        bail!("cargo could not locate the workspace");
    }
    let manifest = String::from_utf8(located.stdout).context("workspace path is not UTF-8")?;
    let root = Path::new(manifest.trim())
        .parent()
        .context("workspace manifest has no parent")?
        .to_path_buf();
    std::env::set_current_dir(&root).context("could not enter the Cargo workspace")?;
    if std::env::args().nth(1).as_deref() == Some("package-release") {
        return release::package(&root, &std::env::args().skip(2).collect::<Vec<_>>());
    }
    if std::env::args().nth(1).as_deref() == Some("package-standalone") {
        return standalone::package(&root, &std::env::args().skip(2).collect::<Vec<_>>());
    }
    if std::env::args().nth(1).as_deref() == Some("release-notices") {
        let (packages, args) = bundle_args(&std::env::args().skip(2).collect::<Vec<_>>())?;
        let notices = notices::Notices::collect(&root, &packages, &args, &[])?;
        let metadata = cargo_metadata::MetadataCommand::new()
            .manifest_path(root.join("Cargo.toml"))
            .no_deps()
            .exec()?;
        let directory = metadata.target_directory.join("release-notices");
        fs::create_dir_all(&directory)?;
        notices.write(directory.as_std_path())?;
        println!("Release notices: {directory}");
        return Ok(());
    }
    let bundling = matches!(
        std::env::args().nth(1).as_deref(),
        Some("bundle" | "bundle-universal")
    );

    // Read before building, not after: a licence text that has gone missing is a
    // reason to stop, not a reason to spend two minutes compiling and then mention
    // it. The bundler runs below without changing to an outer checkout.
    let licences = if bundling {
        COMMON_LICENCES
            .iter()
            .map(|(source, name)| {
                fs::read(root.join(source))
                    .map(|bytes| (*name, bytes))
                    .with_context(|| format!("the release licence '{source}' is missing"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    let nam_notice = if bundling {
        Some(
            fs::read(root.join("nice-nam/NOTICE.txt"))
                .context("NAM's upstream NOTICE.txt is missing")?,
        )
    } else {
        None
    };
    let third_party = if bundling {
        let (packages, args) = bundle_args(&std::env::args().skip(2).collect::<Vec<_>>())?;
        if std::env::args().nth(1).as_deref() == Some("bundle-universal") {
            let mut combined: Option<notices::Notices> = None;
            for target in ["x86_64-apple-darwin", "aarch64-apple-darwin"] {
                let mut args = args.clone();
                args.extend(["--target".into(), target.into()]);
                let next = notices::Notices::collect(&root, &packages, &args, &[])?;
                match &mut combined {
                    None => combined = Some(next),
                    Some(previous) => previous.merge(next),
                }
            }
            combined
        } else {
            Some(notices::Notices::collect(&root, &packages, &args, &[])?)
        }
    } else {
        None
    };
    run_command(&root, std::env::args().skip(1).collect())?;

    let mut modified_bundles = std::collections::BTreeSet::new();
    for (name, licence) in licences {
        modified_bundles.extend(ship_bundle_licence(&root, name, &licence)?);
    }
    if let Some(notice) = nam_notice {
        modified_bundles.extend(ship_bundle_licence(&root, "NOTICE.txt", &notice)?);
    }
    if let Some(notices) = &third_party {
        let metadata = cargo_metadata::MetadataCommand::new()
            .manifest_path(root.join("Cargo.toml"))
            .no_deps()
            .exec()?;
        let bundled = metadata.target_directory.join("bundled");
        notices.write(bundled.as_std_path())?;
    }
    // The public bundler signs during assembly. Resource writes invalidate
    // that seal, so sign again only after every licence has been installed.
    for bundle in &modified_bundles {
        if bundle.join("Contents/MacOS").is_dir() {
            sign_and_verify(bundle)?;
        }
    }
    Ok(())
}

/// Write a common licence, or NAM's upstream notice, into matching bundles.
///
/// Directory bundles (VST3 and macOS CLAP) carry notices in Contents/Resources.
/// Single-file CLAP exports carry their notices beside the binary.
fn ship_bundle_licence(root: &Path, name: &str, licence: &[u8]) -> anyhow::Result<Vec<PathBuf>> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()?;
    let bundled = metadata.target_directory.join("bundled");
    ship_licence(bundled.as_std_path(), name, licence)
}

fn ship_licence(bundled: &Path, name: &str, licence: &[u8]) -> anyhow::Result<Vec<PathBuf>> {
    let entries = fs::read_dir(bundled).with_context(|| {
        format!(
            "no bundle directory at '{}' after bundling",
            bundled.display()
        )
    })?;

    let mut modified = Vec::new();
    let mut written = 0usize;
    let mut single_file_bundles = 0usize;
    for entry in entries {
        let path = entry?.path();
        let bundle_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == "NOTICE.txt" && path.file_stem().and_then(|s| s.to_str()) != Some("Nice NAM") {
            continue;
        }
        if path.is_dir() && (bundle_name.ends_with(".vst3") || bundle_name.ends_with(".clap")) {
            let target = path.join("Contents").join("Resources").join(name);
            let dir = target
                .parent()
                .context("a VST3 bundle path has no parent to create")?;
            fs::create_dir_all(dir)?;
            fs::write(&target, licence)?;
            println!(
                "Added the licence/notice to the plugin bundle at '{}'",
                target.display()
            );
            modified.push(path);
            written += 1;
        } else if path.is_file() && bundle_name.ends_with(".clap") {
            single_file_bundles += 1;
        }
    }

    if single_file_bundles > 0 {
        // One copy per bundle directory rather than per plugin: same font, same text.
        let target = bundled.join(if name == "NOTICE.txt" {
            "NOTICE-NeuralAmpModeler-rs.txt"
        } else {
            name
        });
        fs::write(&target, licence)?;
        println!(
            "Added the licence/notice at '{}' — a CLAP is a single file, so this text travels in \
             whatever archive carries it",
            target.display()
        );
        written += 1;
    }

    // `nice-plug-xtask::main` returned Ok, so it built something. If none of it is
    // here, the reasoning above is wrong, and the last thing this step should do is
    // say nothing.
    if written == 0 && name != "NOTICE.txt" {
        bail!(
            "bundling succeeded but no `.vst3` or `.clap` was found in '{}', so the licence \
             went nowhere",
            bundled.display()
        );
    }
    Ok(modified)
}

/// Seal the final resources and fail packaging if signing or verification fails.
/// No dependency sources are changed; non-macOS bundle layouts skip this step.
fn sign_and_verify(bundle: &Path) -> anyhow::Result<()> {
    for args in [
        &["--force", "--sign", "-"][..],
        &["--verify", "--deep", "--strict"][..],
    ] {
        let output = std::process::Command::new("codesign")
            .args(args)
            .arg(bundle)
            .output()
            .with_context(|| format!("could not run codesign for '{}'", bundle.display()))?;
        if !output.status.success() {
            bail!(
                "codesign {} failed for '{}': {}",
                args[0],
                bundle.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    println!(
        "Signed and verified completed bundle '{}'",
        bundle.display()
    );
    Ok(())
}

/// Keep workspace selection application-owned while delegating native artifact
/// construction to the bundler's public APIs (no dependency modifications).
fn run_command(root: &Path, args: Vec<String>) -> anyhow::Result<()> {
    let command = args
        .first()
        .context("expected bundle, bundle-universal, or known-packages")?;
    if command == "known-packages" {
        return nice_plug_xtask::list_known_packages();
    }
    if command != "bundle" && command != "bundle-universal" {
        bail!("unknown command: {command}");
    }
    let (packages, build_args) = bundle_args(&args[1..])?;
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()?;
    let universal = command == "bundle-universal";
    if universal {
        if build_args
            .iter()
            .any(|arg| arg == "--target" || arg.starts_with("--target="))
        {
            bail!("bundle-universal cannot take --target");
        }
        for target in ["x86_64-apple-darwin", "aarch64-apple-darwin"] {
            let mut target_args = build_args.clone();
            target_args.push(format!("--target={target}"));
            nice_plug_xtask::build(&packages, &target_args)?;
        }
    } else {
        nice_plug_xtask::build(&packages, &build_args)?;
    }
    for package in &packages {
        nice_plug_xtask::bundle(
            metadata.target_directory.as_std_path(),
            package,
            &build_args,
            universal,
        )?;
    }
    Ok(())
}
fn bundle_args(args: &[String]) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let mut packages = Vec::new();
    let mut build_args = Vec::new();
    let mut args = args.iter().peekable();
    if args.peek().is_some_and(|arg| !arg.starts_with('-')) {
        packages.push(args.next().unwrap().clone());
    }
    while let Some(arg) = args.next() {
        if arg == "-p" || arg == "--package" {
            packages.push(args.next().context("missing package name")?.clone());
        } else if let Some(package) = arg.strip_prefix("--package=") {
            packages.push(package.into());
        } else {
            build_args.push(arg.clone());
        }
    }
    if packages.is_empty() {
        bail!("specify a package: cargo xtask bundle nice-clone [--release]");
    }
    Ok((packages, build_args))
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) struct Scratch(pub PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    pub(super) fn scratch() -> Scratch {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "frost-release-notices-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }
    #[test]
    fn bundle_notices_cover_both_layouts_without_attributing_other_plugins_to_nam() {
        let scratch = scratch();
        let dir = &scratch.0;
        fs::create_dir(dir.join("Nice NAM.vst3")).unwrap();
        fs::create_dir(dir.join("Nice Echo.clap")).unwrap();
        fs::write(dir.join("Nice NAM.clap"), b"plugin").unwrap();
        fs::write(dir.join("Nice Tuner.clap"), b"plugin").unwrap();
        for (_, name) in COMMON_LICENCES {
            ship_licence(dir, name, name.as_bytes()).unwrap();
            for product in ["Nice NAM.vst3", "Nice Echo.clap"] {
                assert_eq!(
                    fs::read(dir.join(product).join("Contents/Resources").join(name)).unwrap(),
                    name.as_bytes()
                );
            }
            assert_eq!(fs::read(dir.join(name)).unwrap(), name.as_bytes());
        }
        let modified = ship_licence(dir, "NOTICE.txt", b"nam attribution").unwrap();
        assert_eq!(modified, [dir.join("Nice NAM.vst3")]);
        assert_eq!(
            fs::read(dir.join("Nice NAM.vst3/Contents/Resources/NOTICE.txt")).unwrap(),
            b"nam attribution"
        );
        assert!(!dir
            .join("Nice Echo.clap/Contents/Resources/NOTICE.txt")
            .exists());
        assert_eq!(
            fs::read(dir.join("NOTICE-NeuralAmpModeler-rs.txt")).unwrap(),
            b"nam attribution"
        );
    }
    #[test]
    fn nam_notice_is_not_added_to_a_release_containing_only_other_plugins() {
        let scratch = scratch();
        fs::write(scratch.0.join("Nice Tuner.clap"), b"plugin").unwrap();
        assert!(ship_licence(&scratch.0, "NOTICE.txt", b"nam")
            .unwrap()
            .is_empty());
        assert!(!scratch.0.join("NOTICE-NeuralAmpModeler-rs.txt").exists());
    }
    #[test]
    fn package_selection_keeps_cargo_build_options_intact() {
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            bundle_args(&args(&["nice-clone", "--release", "--offline"])).unwrap(),
            (args(&["nice-clone"]), args(&["--release", "--offline"]))
        );
        assert_eq!(
            bundle_args(&args(&[
                "-p",
                "nice-clone",
                "-p",
                "nice-wah",
                "--features",
                "standalone"
            ]))
            .unwrap(),
            (
                args(&["nice-clone", "nice-wah"]),
                args(&["--features", "standalone"])
            )
        );
        assert!(bundle_args(&args(&["-p"])).is_err());
    }
}
