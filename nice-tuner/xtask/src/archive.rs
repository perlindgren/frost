//! Build and immediately verify distributable tar.gz archives using system tar.
//! No extraction to the filesystem and no dependency/source tree is archived.
use super::*;
use std::process::Command;

pub(super) fn create(
    artifact: &Path,
    resource_directory: &Path,
    common: &[(&str, Vec<u8>)],
    notices: &crate::notices::Notices,
    nam: Option<(&str, &[u8])>,
) -> anyhow::Result<PathBuf> {
    let parent = artifact.parent().context("artifact has no parent")?;
    let name = artifact.file_name().context("artifact has no name")?;
    let mut expected: Vec<(PathBuf, Vec<u8>)> = common
        .iter()
        .map(|(name, bytes)| (resource_directory.join(name), bytes.clone()))
        .collect();
    expected.extend(
        notices
            .files
            .iter()
            .map(|(name, bytes)| (resource_directory.join(name), bytes.clone())),
    );
    if let Some((name, bytes)) = nam {
        expected.push((resource_directory.join(name), bytes.to_vec()));
    }
    for (path, bytes) in &expected {
        if fs::read(path)? != *bytes {
            bail!("release text differs before archiving: {}", path.display());
        }
    }
    let archive = parent.join(format!("{}.tar.gz", name.to_string_lossy()));
    let temporary = parent.join(format!(
        ".{}.{}.tar.gz",
        name.to_string_lossy(),
        std::process::id()
    ));
    let mut command = Command::new("tar");
    command.env("COPYFILE_DISABLE", "1");
    command
        .arg("-czf")
        .arg(&temporary)
        .arg("-C")
        .arg(parent)
        .arg(name);
    if artifact.is_file() {
        // Single-file CLAP exports cannot contain their own notices.
        for (path, _) in &expected {
            command.arg(path.strip_prefix(parent)?);
        }
    }
    let output = command
        .output()
        .context("release archives require the system tar command")?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        bail!("tar failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let relative = expected
        .into_iter()
        .map(|(path, bytes)| Ok((path.strip_prefix(parent)?.to_path_buf(), bytes)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    if let Err(error) = verify(&temporary, &relative) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    fs::rename(&temporary, &archive)?;
    println!("Verified release archive: {}", archive.display());
    Ok(archive)
}
fn verify(archive: &Path, expected: &[(PathBuf, Vec<u8>)]) -> anyhow::Result<()> {
    let output = Command::new("tar").arg("-tzf").arg(archive).output()?;
    if !output.status.success() {
        bail!("cannot list release archive");
    }
    let listing = String::from_utf8(output.stdout)?;
    for name in listing.lines() {
        validate_entry(name)?;
    }
    for (name, bytes) in expected {
        let output = Command::new("tar")
            .arg("-xOzf")
            .arg(archive)
            .arg(name)
            .output()?;
        if !output.status.success() || output.stdout != *bytes {
            bail!("archive notice missing or changed: {}", name.display());
        }
    }
    Ok(())
}
fn validate_entry(name: &str) -> anyhow::Result<()> {
    let path = Path::new(name);
    if path.is_absolute()
        || name.contains('\\')
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("unsafe archive path: {name}");
    }
    let file = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file == ".DS_Store" || file.starts_with("._") || file.contains("-settings.") {
        bail!("unintended release file: {name}");
    }
    if matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("nam" | "wav" | "ron" | "png" | "jpg" | "jpeg" | "svg" | "pdf" | "rs" | "c" | "h")
    ) {
        bail!("unreviewed asset/source in release archive: {name}");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archives_are_read_back_and_missing_or_corrupted_notices_are_rejected() {
        let scratch = crate::tests::scratch();
        let product = scratch.0.join("app");
        fs::create_dir(&product).unwrap();
        fs::write(product.join("app"), b"binary").unwrap();
        let common = vec![("LICENSE-MIT", b"license".to_vec())];
        fs::write(product.join("LICENSE-MIT"), b"license").unwrap();
        let notices = crate::notices::Notices {
            files: vec![("MPL-SOURCES.txt".into(), b"source".to_vec())],
        };
        notices.write(&product).unwrap();
        let archive = create(&product, &product, &common, &notices, None).unwrap();
        assert!(verify(
            &archive,
            &[(PathBuf::from("app/LICENSE-MIT"), b"wrong".to_vec())]
        )
        .is_err());
        assert!(verify(&archive, &[(PathBuf::from("app/missing"), b"".to_vec())]).is_err());
    }
    #[test]
    fn private_settings_unreviewed_assets_and_traversal_are_rejected() {
        for name in [
            "../escape",
            "/absolute",
            "app/model.nam",
            "app/cab.wav",
            "app/nice-rack-settings.json",
            "app/.DS_Store",
        ] {
            assert!(validate_entry(name).is_err(), "{name}");
        }
        assert!(validate_entry("Nice Dist.vst3/Contents/Resources/LICENSE-MIT").is_ok());
    }
}
