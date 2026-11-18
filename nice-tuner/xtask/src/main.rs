//! Plugin bundler entry point: `cargo xtask bundle nice-tuner --release`.
//!
//! Native bundle assembly uses nice-plug-xtask public APIs. This wrapper keeps
//! nested worktrees in their own workspace and ships embedded font licences.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// `src/gui.rs` embeds `assets/fonts/FiraCode-VariableFont_wght.ttf`, and OFL 1.1
/// permits that on the condition that its licence text travels with the font. The
/// embedded font is the awkward case precisely because it succeeds: the font never
/// appears as a file, so nothing ever reaches for its licence — and a bundle from
/// `nice-plug-xtask` 0.1.1 contains the binary and nothing else, because that step
/// does not exist upstream. This puts the text where a distributor meets it rather
/// than remembers it.
///
/// Both texts go into every bundle this workspace builds, whichever plugin was
/// named on the command line, because every editor embeds both fonts: Fira Code
/// from the repository's `assets/fonts/`, and the icon font now held by
/// `nice-common` and handed out by `nice_common::fonts::icon_font()`.
const FONT_LICENCES: &[(&str, &str)] = &[
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
    let bundling = matches!(
        std::env::args().nth(1).as_deref(),
        Some("bundle" | "bundle-universal")
    );

    // Read before building, not after: a licence text that has gone missing is a
    // reason to stop, not a reason to spend two minutes compiling and then mention
    // it. The bundler runs below without changing to an outer checkout.
    let licences = if bundling {
        FONT_LICENCES
            .iter()
            .map(|(source, name)| {
                fs::read(root.join(source))
                    .map(|bytes| (*name, bytes))
                    .with_context(|| format!("the embedded font licence '{source}' is missing"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    run_command(&root, std::env::args().skip(1).collect())?;

    for (name, licence) in licences {
        ship_font_licence(&root, name, &licence)?;
    }
    Ok(())
}

/// Write the font's licence text into every bundle just built.
///
/// Directory bundles (VST3 and macOS CLAP) carry notices in Contents/Resources.
/// Single-file CLAP exports carry their notices beside the binary.
fn ship_font_licence(root: &Path, name: &str, licence: &[u8]) -> anyhow::Result<()> {
    // `nice-plug-xtask` resolves the real target directory through
    // `cargo metadata`; this repeats the part of that rule a build here can hit. A
    // `[build] target-dir` in a cargo config would land elsewhere, and that case
    // becomes the error below rather than a silence.
    let bundled = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join("bundled");

    let entries = fs::read_dir(&bundled).with_context(|| {
        format!(
            "no bundle directory at '{}'; a `[build] target-dir` set in a cargo config is not \
             followed by this step",
            bundled.display()
        )
    })?;

    let mut written = 0usize;
    let mut single_file_bundles = 0usize;
    for entry in entries {
        let path = entry?.path();
        let bundle_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() && (bundle_name.ends_with(".vst3") || bundle_name.ends_with(".clap")) {
            let target = path.join("Contents").join("Resources").join(name);
            let dir = target
                .parent()
                .context("a VST3 bundle path has no parent to create")?;
            fs::create_dir_all(dir)?;
            fs::write(&target, licence)?;
            println!(
                "Added the font licence to the plugin bundle at '{}'",
                target.display()
            );
            written += 1;
        } else if path.is_file() && bundle_name.ends_with(".clap") {
            single_file_bundles += 1;
        }
    }

    if single_file_bundles > 0 {
        // One copy per bundle directory rather than per plugin: same font, same text.
        let target = bundled.join(name);
        fs::write(&target, licence)?;
        println!(
            "Added the font licence at '{}' — a CLAP is a single file, so this text travels in \
             whatever archive carries it",
            target.display()
        );
        written += 1;
    }

    // `nice-plug-xtask::main` returned Ok, so it built something. If none of it is
    // here, the reasoning above is wrong, and the last thing this step should do is
    // say nothing.
    if written == 0 {
        bail!(
            "bundling succeeded but no `.vst3` or `.clap` was found in '{}', so the font licence \
             went nowhere",
            bundled.display()
        );
    }
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
