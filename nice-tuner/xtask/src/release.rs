//! One downloadable archive per target, with large dependency notices shared
//! once by all selected plugin formats, standalone applications and nice-rack.
use super::*;
use std::process::Command;

pub(super) fn package(root: &Path, args: &[String]) -> anyhow::Result<()> {
    let (mut packages, build_args) = bundle_args(args)?;
    for name in &mut packages {
        if name == "nice-rack" {
            *name = "nice-rack-standalone".into();
        }
    }
    // This prevents an inventory/build mismatch through machine-local Cargo
    // target configuration. Native releases also name their target explicitly.
    let target = build_args
        .iter()
        .enumerate()
        .find_map(|(index, arg)| {
            arg.strip_prefix("--target=")
                .map(str::to_owned)
                .or_else(|| {
                    (arg == "--target")
                        .then(|| build_args.get(index + 1).cloned())
                        .flatten()
                })
        })
        .context("package-release requires --target <triple>, including native releases")?;
    if target.contains(['/', '\\']) || target.starts_with('.') {
        bail!("release target must be a target triple, not a custom target file");
    }
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()?;
    let mut plugin_names = Vec::new();
    let mut features = Vec::new();
    for name in &packages {
        let package = metadata
            .packages
            .iter()
            .find(|p| p.name.as_str() == name)
            .with_context(|| format!("unknown package: {name}"))?;
        if !name.starts_with("nice-") || !package.targets.iter().any(|t| t.is_bin()) {
            bail!("{name} is not a nice standalone application");
        }
        if package.features.contains_key("standalone") {
            features.push(format!("{name}/standalone"));
        }
        if name != "nice-rack-standalone" {
            plugin_names.push(name.clone());
        }
    }
    // Collect and validate legal evidence before invoking builds.
    let notices = crate::notices::Notices::collect(root, &packages, &build_args, &features)?;
    let common = COMMON_LICENCES
        .iter()
        .map(|(path, name)| Ok((*name, fs::read(root.join(path))?)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let nam_notice = fs::read(root.join("nice-nam/NOTICE.txt"))?;
    let executable = std::env::current_exe()?;
    for (command, selected) in [("bundle", &plugin_names), ("package-standalone", &packages)] {
        if selected.is_empty() {
            continue;
        }
        let mut child = Command::new(&executable);
        child.current_dir(root).arg(command);
        for package in selected {
            child.args(["-p", package]);
        }
        if !child.args(&build_args).status()?.success() {
            bail!("{command} failed; no release archive was generated");
        }
    }
    let output = metadata
        .target_directory
        .join("downloads")
        .join(format!("frost-nice-{target}"));
    if output.exists() {
        fs::remove_dir_all(&output)?;
    }
    fs::create_dir_all(output.join("plugins"))?;
    fs::create_dir_all(output.join("standalone"))?;
    let config = fs::read_to_string(root.join("bundler.toml"))?;
    for package in &plugin_names {
        let name = bundle_name(&config, package)?;
        for format in ["clap", "vst3"] {
            let file = format!("{name}.{format}");
            copy(
                &metadata
                    .target_directory
                    .join("bundled")
                    .join(&file)
                    .into_std_path_buf(),
                &output.join("plugins").join(file).into_std_path_buf(),
            )?;
        }
    }
    let extension = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    for package in &packages {
        let info = metadata
            .packages
            .iter()
            .find(|p| p.name.as_str() == package)
            .unwrap();
        for binary in info.targets.iter().filter(|t| t.is_bin()) {
            let name = format!("{}{extension}", binary.name);
            copy(
                &metadata
                    .target_directory
                    .join("packaged")
                    .join(&binary.name)
                    .join(&name)
                    .into_std_path_buf(),
                &output.join("standalone").join(&name).into_std_path_buf(),
            )?;
        }
    }
    for (name, bytes) in &common {
        fs::write(output.join(name), bytes)?;
    }
    let nam = packages
        .iter()
        .any(|p| p == "nice-nam")
        .then_some(("NOTICE-NeuralAmpModeler-rs.txt", nam_notice.as_slice()));
    if let Some((name, bytes)) = nam {
        fs::write(output.join(name), bytes)?;
    }
    notices.write(output.as_std_path())?;
    crate::archive::create(
        output.as_std_path(),
        output.as_std_path(),
        &common,
        &notices,
        nam,
    )?;
    Ok(())
}
fn copy(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let kind = fs::symlink_metadata(source)
        .with_context(|| format!("missing built artifact: {}", source.display()))?
        .file_type();
    if kind.is_symlink() {
        bail!(
            "release artifact contains an unreviewed symlink: {}",
            source.display()
        );
    }
    if kind.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if kind.is_file() {
        fs::copy(source, destination)?;
    } else {
        bail!("unsupported release artifact: {}", source.display());
    }
    Ok(())
}
fn bundle_name(config: &str, package: &str) -> anyhow::Result<String> {
    let mut selected = false;
    for line in config.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            selected = section == package;
        } else if selected {
            if let Some(name) = line
                .strip_prefix("name = \"")
                .and_then(|s| s.strip_suffix('"'))
            {
                if name.contains(['/', '\\', '"']) || name.is_empty() || name.starts_with('.') {
                    bail!("invalid bundle name");
                }
                return Ok(name.into());
            }
        }
    }
    bail!("no supported bundle name for {package}")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_names_preserve_spaces_and_reject_path_escape() {
        assert_eq!(
            bundle_name("[nice-dist]\nname = \"Nice Dist\"", "nice-dist").unwrap(),
            "Nice Dist"
        );
        assert!(bundle_name("[nice-dist]\nname = \"../escape\"", "nice-dist").is_err());
        assert!(bundle_name("[other]\nname = \"Other\"", "nice-dist").is_err());
    }
}
