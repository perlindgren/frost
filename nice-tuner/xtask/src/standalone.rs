//! Stage standalone executables and their notices as self-contained directories.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};

pub(super) fn package(root: &Path, args: &[String]) -> anyhow::Result<()> {
    let (mut packages, build_args) = bundle_args(args)?;
    for package in &mut packages {
        if package == "nice-rack" {
            *package = "nice-rack-standalone".into();
        }
    }
    if build_args.iter().any(|a| a.starts_with("--message-format")) {
        bail!("package-standalone owns --message-format to locate the built executables");
    }
    // Read all mandatory notices before starting an expensive build.
    let common = COMMON_LICENCES
        .iter()
        .map(|(path, name)| {
            Ok((
                *name,
                fs::read(root.join(path))
                    .with_context(|| format!("missing release licence: {path}"))?,
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let nam_notice = fs::read(root.join("nice-nam/NOTICE.txt"))?;
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()?;
    let mut selected = BTreeMap::new();
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
        selected.insert(package.id.to_string(), name.clone());
        if package.features.contains_key("standalone") {
            features.push(format!("{name}/standalone"));
        }
    }
    let notices = crate::notices::Notices::collect(root, &packages, &build_args, &features)?;
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "build",
        "--bins",
        "--message-format=json-render-diagnostics",
    ]);
    for package in &packages {
        command.args(["-p", package]);
    }
    if !features.is_empty() {
        command.args(["--features", &features.join(",")]);
    }
    command
        .args(build_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .context("could not build standalone applications")?;
    let mut executables = BTreeMap::new();
    let mut parse_error = None;
    for message in
        cargo_metadata::Message::parse_stream(std::io::BufReader::new(child.stdout.take().unwrap()))
    {
        match message {
            Ok(cargo_metadata::Message::CompilerArtifact(artifact))
                if selected.contains_key(&artifact.package_id.to_string()) =>
            {
                if let Some(path) = artifact.executable {
                    executables.insert(artifact.package_id.to_string(), path);
                }
            }
            Ok(cargo_metadata::Message::CompilerMessage(message)) => {
                if let Some(rendered) = message.message.rendered {
                    eprint!("{rendered}");
                }
            }
            Err(error) => {
                parse_error = Some(error);
            }
            _ => {}
        }
    }
    if !child.wait()?.success() {
        bail!("standalone build failed; no release directories were staged");
    }
    if let Some(error) = parse_error {
        return Err(error.into());
    }
    let mut staged = BTreeSet::new();
    for (id, package) in selected {
        let executable = executables
            .get(&id)
            .with_context(|| format!("cargo did not report an executable for {package}"))?;
        let name = executable
            .file_stem()
            .context("executable has no file name")?;
        let directory = metadata.target_directory.join("packaged").join(name);
        stage(
            executable.as_std_path(),
            directory.as_std_path(),
            &common,
            (package == "nice-nam").then_some(nam_notice.as_slice()),
        )?;
        staged.insert(directory);
    }
    let shared = metadata.target_directory.join("packaged");
    notices.write(shared.as_std_path())?;
    for path in staged {
        println!("Standalone package: {path}");
    }
    Ok(())
}

fn stage(
    executable: &Path,
    directory: &Path,
    common: &[(&str, Vec<u8>)],
    nam: Option<&[u8]>,
) -> anyhow::Result<()> {
    fs::create_dir_all(directory)?;
    fs::copy(
        executable,
        directory.join(executable.file_name().context("executable has no name")?),
    )?;
    for (name, bytes) in common {
        fs::write(directory.join(name), bytes)?;
    }
    if let Some(notice) = nam {
        fs::write(directory.join("NOTICE.txt"), notice)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_packages_keep_all_texts_and_the_nam_notice_beside_the_executable() {
        let scratch = crate::tests::scratch();
        let binary = scratch.0.join("nice-nam-standalone");
        fs::write(&binary, b"executable").unwrap();
        let common = COMMON_LICENCES
            .iter()
            .map(|(_, name)| (*name, name.as_bytes().to_vec()))
            .collect::<Vec<_>>();
        for nam in [false, true] {
            let dir = scratch.0.join(if nam { "nam" } else { "other" });
            stage(&binary, &dir, &common, nam.then_some(b"upstream notice")).unwrap();
            assert_eq!(
                fs::read(dir.join("nice-nam-standalone")).unwrap(),
                b"executable"
            );
            for (name, content) in &common {
                assert_eq!(fs::read(dir.join(name)).unwrap(), *content);
            }
            assert_eq!(dir.join("NOTICE.txt").exists(), nam);
        }
    }
}
