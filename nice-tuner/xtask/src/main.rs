//! Plugin bundler entry point: `cargo xtask bundle nice-tuner --release`.
//!
//! Everything about assembling the plugin is `nice-plug-xtask`'s. This file adds
//! the font licence notices that must accompany the embedded editor assets.

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
const FONT_LICENCES: &[(&str, &str)] = &[
    ("assets/fonts/OFL.txt", "OFL.txt"),
    (
        "nice-tuner/assets/fonts/LICENSE-FluentSystemIcons.txt",
        "LICENSE-FluentSystemIcons.txt",
    ),
];

fn main() -> anyhow::Result<()> {
    nice_plug_xtask::chdir_workspace_root()?;
    let root = std::env::current_dir().context("could not read the working directory")?;
    let bundling = std::env::args().nth(1).as_deref() == Some("bundle");

    // Read before building, not after: a licence text that has gone missing is a
    // reason to stop, not a reason to spend two minutes compiling and then mention
    // it. `chdir_workspace_root` is idempotent, so `main` calling it again for its
    // own sake costs nothing.
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

    nice_plug_xtask::main()?;

    for (name, licence) in licences {
        ship_font_licence(&root, name, &licence)?;
    }
    Ok(())
}

/// Write the font's licence text into every bundle just built.
///
/// Two destinations, because the two formats are different shapes. A `.vst3` is a
/// directory and a host reads what it asks for, so the text can ride inside at
/// `Contents/Resources/OFL.txt` and a bundle copied anywhere carries its own
/// licence. A CLAP is deliberately *one file* — that is the format's point — so
/// nothing goes inside it; the text lands beside the bundles with a line saying so,
/// because from there the duty belongs to whoever builds the archive, and the only
/// defence against forgetting is making it unavoidable.
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
        if path.is_dir() && bundle_name.ends_with(".vst3") {
            let target = path.join("Contents").join("Resources").join(name);
            let dir = target
                .parent()
                .context("a VST3 bundle path has no parent to create")?;
            fs::create_dir_all(dir)?;
            fs::write(&target, licence)?;
            println!(
                "Added the font licence to the VST3 bundle at '{}'",
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
