//! Collect resolved dependency declarations and original legal texts, never
//! dependency code. Unknown license expressions or missing evidence fail closed.
use super::*;
use cargo_metadata::{CargoOpt, DependencyKind, Metadata, Package};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Notices {
    pub files: Vec<(String, Vec<u8>)>,
}
impl Notices {
    pub fn collect(
        root: &Path,
        packages: &[String],
        args: &[String],
        extra_features: &[String],
    ) -> anyhow::Result<Self> {
        let mut command = cargo_metadata::MetadataCommand::new();
        command.manifest_path(root.join("Cargo.toml"));
        let shallow = cargo_metadata::MetadataCommand::new()
            .manifest_path(root.join("Cargo.toml"))
            .no_deps()
            .exec()?;
        let mut features = extra_features.to_vec();
        let mut target = std::env::var("CARGO_BUILD_TARGET").ok();
        let mut options = vec!["--locked".to_owned()];
        let mut items = args.iter();
        while let Some(arg) = items.next() {
            match arg.as_str() {
                "--features" | "-F" => {
                    qualify_features(
                        items.next().context("missing features")?,
                        packages,
                        &shallow,
                        &mut features,
                    );
                }
                "--all-features" => {
                    command.features(CargoOpt::AllFeatures);
                }
                "--no-default-features" => {
                    command.features(CargoOpt::NoDefaultFeatures);
                }
                "--target" => target = Some(items.next().context("missing target")?.clone()),
                "--offline" | "--frozen" => options.push(arg.clone()),
                "--config" => {
                    options.push(arg.clone());
                    options.push(items.next().context("missing config")?.clone());
                }
                _ => {
                    if let Some(value) = arg.strip_prefix("--features=") {
                        qualify_features(value, packages, &shallow, &mut features);
                    }
                    if let Some(value) = arg.strip_prefix("--target=") {
                        target = Some(value.into());
                    }
                }
            }
        }
        // Explicit cross-builds use their target graph. Native builds use the
        // rustc host; release CI must pass --target explicitly (see docs).
        let target = target.map(Ok).unwrap_or_else(host_target)?;
        options.extend(["--filter-platform".into(), target.clone()]);
        command.other_options(options);
        if !features.is_empty() {
            command.features(CargoOpt::SomeFeatures(features));
        }
        let metadata = command.exec()?;
        Self::from_metadata(root, &metadata, packages, &target)
    }
    fn from_metadata(
        root: &Path,
        metadata: &Metadata,
        packages: &[String],
        target: &str,
    ) -> anyhow::Result<Self> {
        let legal_root = root.join("docs/third-party-licenses");
        let overrides = fs::read_to_string(legal_root.join("upstream.tsv"))?;
        let selected = dependency_ids(metadata, packages)?;
        let mut notice = String::from("THIRD-PARTY SOFTWARE NOTICES\n\nOriginal notices below retain their upstream terms and owners.\nStandard license texts at the end are terms/templates, not invented copyright\nclaims. The project licenses do not replace these dependency licenses.\nThis inventory follows resolved normal-dependency edges, including proc macros;\ncompiler dead stripping may omit some listed code. It is not binary inspection\nor a legal clearance. OS-provided libraries/SDKs are not bundled by this tool.\n\n");
        notice.push_str(&format!(
            "Target: {target}\nRoot packages: {}\n\n",
            packages.join(", ")
        ));
        let mut inventory = String::from("target\tpackage\tversion\tdeclared license\tselected terms\tsource\trepository\tauthors\n");
        let mut mpl = String::from("MPL-2.0 SOURCE AVAILABILITY\n\nThe MPL-covered dependencies listed below are used without source modifications.\nTheir corresponding source remains under MPL-2.0; our separate files retain\ntheir own licenses. Download links identify the exact published crate versions.\nThese source archives preserve the upstream notices. No source archive needs\nto be inside this executable package.\nLicense: https://www.mozilla.org/en-US/MPL/2.0/\n\n");
        let mut standards = BTreeSet::new();
        let mut reproduced = BTreeMap::new();
        let mut dependencies: Vec<_> = metadata
            .packages
            .iter()
            .filter(|p| selected.contains(&p.id.to_string()) && p.source.is_some())
            .collect();
        dependencies.sort_by_key(|p| {
            (
                p.name.to_string(),
                p.version.to_string(),
                p.source.as_ref().unwrap().to_string(),
            )
        });
        for package in dependencies {
            let expression = package.license.as_deref().with_context(|| {
                format!(
                    "{} has no declared license; review before release",
                    package.name
                )
            })?;
            let terms = selected_terms(expression)?;
            standards.extend(terms.iter().copied());
            let source = source_url(package)?;
            let name = package.name.to_string();
            let version = package.version.to_string();
            inventory.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                target,
                name,
                version,
                expression,
                terms.join(" AND "),
                source,
                package.repository.as_deref().unwrap_or(""),
                package.authors.join("; ")
            ));
            notice.push_str(&format!("\n================================================================\n{name} {version}\nDeclared license: {expression}\nSelected terms: {}\nSource: {source}\nRepository: {}\nAuthors (manifest metadata, not inferred copyright ownership): {}\n", terms.join(" AND "), package.repository.as_deref().unwrap_or("not declared"), package.authors.join("; ")));
            let directory = package
                .manifest_path
                .parent()
                .context("package has no directory")?
                .as_std_path();
            let mut files = Vec::new();
            collect_texts(directory, directory, &mut files)?;
            if let Some(path) = &package.license_file {
                let path = directory.join(path);
                if path.is_file() && !files.contains(&path) {
                    files.push(path);
                }
            }
            files.sort();
            files.dedup();
            let mut evidence = 0;
            for file in files {
                let relative = file
                    .strip_prefix(directory)
                    .context("legal text outside its package")?;
                notice.push_str(&format!(
                    "\n--- packaged upstream {} ---\n",
                    relative.display()
                ));
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("non-text legal file: {}", file.display()))?;
                append_unique(
                    &mut notice,
                    &mut reproduced,
                    format!("{name} {version}: {}", relative.display()),
                    text,
                );
                evidence += 1;
            }
            // Some crates omit repository-level legal files from their package.
            // Reviewed copies are indexed by exact name/version with upstream
            // commit URLs and SHA-256 provenance, without changing the cache.
            for row in overrides
                .lines()
                .filter(|line| !line.starts_with('#') && !line.is_empty())
            {
                let parts: Vec<_> = row.split('\t').collect();
                if parts.len() != 6 {
                    bail!("invalid upstream license index row");
                }
                if parts[0] != name || parts[1] != version || parts[5] != source {
                    continue;
                }
                let path = Path::new(parts[2]);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    bail!("invalid override path");
                }
                notice.push_str(&format!(
                    "\n--- upstream {} ---\nSource: {}\nRecorded SHA-256: {}\n",
                    parts[2], parts[3], parts[4]
                ));
                append_unique(
                    &mut notice,
                    &mut reproduced,
                    format!("{name} {version}: {}", parts[2]),
                    fs::read_to_string(legal_root.join(path))?,
                );
                evidence += 1;
            }
            if evidence == 0 {
                bail!("no original legal text/declaration collected for {name} {version}; review upstream before releasing");
            }
            if terms.contains(&"MPL-2.0") {
                mpl.push_str(&format!(
                    "{name} {version}\nLicense: MPL-2.0\nCorresponding source: {source}\n\n"
                ));
            }
        }
        for name in standards {
            notice.push_str(&format!("\n================================================================\nSTANDARD LICENSE TERMS: {name}\nAny copyright placeholders are templates; package attribution appears above.\n\n"));
            notice.push_str(&fs::read_to_string(
                legal_root.join("standard").join(format!("{name}.txt")),
            )?);
        }
        Ok(Self {
            files: vec![
                ("THIRD-PARTY-NOTICES.txt".into(), notice.into_bytes()),
                ("DEPENDENCIES.tsv".into(), inventory.into_bytes()),
                ("MPL-SOURCES.txt".into(), mpl.into_bytes()),
            ],
        })
    }
    pub fn merge(&mut self, other: Self) {
        for (name, bytes) in other.files {
            let (_, destination) = self
                .files
                .iter_mut()
                .find(|(key, _)| *key == name)
                .expect("fixed notice file set");
            if name == "DEPENDENCIES.tsv" {
                if let Some(newline) = bytes.iter().position(|b| *b == b'\n') {
                    destination.extend_from_slice(&bytes[newline + 1..]);
                }
            } else {
                destination.extend_from_slice(b"\nADDITIONAL BUILD TARGET\n\n");
                destination.extend(bytes);
            }
        }
    }
    pub fn write(&self, directory: &Path) -> anyhow::Result<()> {
        for (name, bytes) in &self.files {
            fs::write(directory.join(name), bytes)?;
        }
        self.verify(directory)
    }
    pub fn verify(&self, directory: &Path) -> anyhow::Result<()> {
        for (name, bytes) in &self.files {
            if fs::read(directory.join(name))
                .with_context(|| format!("missing release notice: {name}"))?
                != *bytes
            {
                bail!("release notice mismatch: {name}");
            }
        }
        Ok(())
    }
}
fn append_unique(
    output: &mut String,
    reproduced: &mut BTreeMap<String, String>,
    label: String,
    text: String,
) {
    if let Some(previous) = reproduced.get(&text) {
        output.push_str(&format!(
            "Identical legal text reproduced above under: {previous}\n"
        ));
    } else {
        output.push_str(&text);
        output.push('\n');
        reproduced.insert(text, label);
    }
}
fn qualify_features(
    value: &str,
    packages: &[String],
    metadata: &Metadata,
    features: &mut Vec<String>,
) {
    for feature in value.split([',', ' ']).filter(|s| !s.is_empty()) {
        if feature.contains('/') {
            features.push(feature.into());
        } else {
            for name in packages {
                if metadata
                    .packages
                    .iter()
                    .any(|p| p.name.as_str() == name && p.features.contains_key(feature))
                {
                    features.push(format!("{name}/{feature}"));
                }
            }
        }
    }
}
fn host_target() -> anyhow::Result<String> {
    let output = std::process::Command::new("rustc").arg("-vV").output()?;
    if !output.status.success() {
        bail!("could not determine native target");
    }
    String::from_utf8(output.stdout)?
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .context("rustc did not report its host target")
}
fn dependency_ids(metadata: &Metadata, packages: &[String]) -> anyhow::Result<BTreeSet<String>> {
    let graph = metadata
        .resolve
        .as_ref()
        .context("missing resolved dependency graph")?;
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id.to_string(), n)).collect();
    let mut pending = Vec::new();
    for name in packages {
        pending.push(
            metadata
                .packages
                .iter()
                .find(|p| p.name.as_str() == name)
                .with_context(|| format!("unknown package {name}"))?
                .id
                .to_string(),
        );
    }
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for dependency in &nodes.get(&id).context("missing dependency node")?.deps {
            if dependency
                .dep_kinds
                .iter()
                .any(|kind| kind.kind == DependencyKind::Normal)
            {
                pending.push(dependency.pkg.to_string());
            }
        }
    }
    Ok(seen)
}
fn source_url(package: &Package) -> anyhow::Result<String> {
    let source = package
        .source
        .as_ref()
        .context("workspace package is not third party")?
        .to_string();
    if matches!(
        source.as_str(),
        "registry+https://github.com/rust-lang/crates.io-index"
            | "registry+sparse+https://index.crates.io/"
            | "registry+https://index.crates.io/"
    ) {
        return Ok(format!(
            "https://crates.io/api/v1/crates/{}/{}/download",
            package.name, package.version
        ));
    }
    if source.starts_with("git+") {
        let revision = source
            .rsplit_once('#')
            .context("git dependency has no locked revision")?
            .1;
        let repository = source
            .trim_start_matches("git+")
            .split(['?', '#'])
            .next()
            .unwrap()
            .trim_end_matches(".git");
        return Ok(format!("{repository}/archive/{revision}.tar.gz"));
    }
    bail!("unreviewed dependency source: {source}")
}
fn collect_texts(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if entry.file_name() != ".git" {
                collect_texts(root, &path, files)?;
            }
        } else if kind.is_file() {
            let name = entry.file_name().to_string_lossy().to_uppercase();
            let legal = ["LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE"]
                .iter()
                .any(|prefix| name.starts_with(prefix));
            let in_legal_directory = path.strip_prefix(root)?.parent().is_some_and(|p| {
                p.components()
                    .any(|c| matches!(c.as_os_str().to_str(), Some("licenses" | "licences")))
            });
            if legal || in_legal_directory {
                files.push(path);
            }
        }
    }
    Ok(())
}
fn selected_terms(expression: &str) -> anyhow::Result<Vec<&'static str>> {
    // Deliberate small policy: adding a different license requires review,
    // rather than silently treating a new dependency as permissive.
    let normalized = expression
        .replace('/', " OR ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized == "(MIT OR Apache-2.0) AND Unicode-3.0" {
        return Ok(vec!["MIT", "Unicode-3.0"]);
    }
    if normalized == "(Apache-2.0 OR MIT) AND BSD-3-Clause" {
        return Ok(vec!["MIT", "BSD-3-Clause"]);
    }
    if normalized == "Apache-2.0 AND MIT" {
        return Ok(vec!["Apache-2.0", "MIT"]);
    }
    let supported = [
        "MIT",
        "Apache-2.0",
        "MPL-2.0",
        "ISC",
        "BSD-1-Clause",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "Zlib",
        "Unlicense",
        "0BSD",
        "Unicode-3.0",
    ];
    let alternatives: Vec<_> = normalized.split(" OR ").collect();
    // Alternatives outside this set can be declined, but never conjunctions.
    if alternatives.iter().any(|s| s.contains(" AND ")) {
        bail!("license expression needs review: {expression}");
    }
    for preferred in supported {
        if alternatives.contains(&preferred) {
            return Ok(vec![preferred]);
        }
    }
    bail!("license expression needs review: {expression}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_follows_normal_edges_with_target_and_feature_filtering() {
        let scratch = crate::tests::scratch();
        for name in ["app", "normal", "optional", "build", "dev", "win"] {
            let dir = scratch.0.join(name);
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(dir.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
            fs::write(
                dir.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n"),
            )
            .unwrap();
        }
        let manifest = scratch.0.join("app/Cargo.toml");
        fs::write(
            &manifest,
            r#"[package]
name = "app"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
normal = { path = "../normal" }
optional = { path = "../optional", optional = true }
[features]
selected = ["dep:optional"]
[build-dependencies]
build = { path = "../build" }
[dev-dependencies]
dev = { path = "../dev" }
[target.'cfg(windows)'.dependencies]
win = { path = "../win" }
"#,
        )
        .unwrap();
        for (target, feature, wanted) in [
            (
                "aarch64-apple-darwin",
                true,
                vec!["app", "normal", "optional"],
            ),
            (
                "x86_64-pc-windows-msvc",
                false,
                vec!["app", "normal", "win"],
            ),
        ] {
            let mut command = cargo_metadata::MetadataCommand::new();
            command.manifest_path(&manifest).other_options(vec![
                "--offline".into(),
                "--filter-platform".into(),
                target.into(),
            ]);
            if feature {
                command.features(CargoOpt::SomeFeatures(vec!["selected".into()]));
            }
            let metadata = command.exec().unwrap();
            let ids = dependency_ids(&metadata, &["app".into()]).unwrap();
            let mut names: Vec<_> = metadata
                .packages
                .iter()
                .filter(|p| ids.contains(&p.id.to_string()))
                .map(|p| p.name.to_string())
                .collect();
            names.sort();
            assert_eq!(names, wanted);
        }
    }
    #[test]
    fn identical_legal_texts_are_reproduced_once_with_a_cross_reference() {
        let mut output = String::new();
        let mut seen = BTreeMap::new();
        append_unique(
            &mut output,
            &mut seen,
            "first".into(),
            "original legal text".into(),
        );
        append_unique(
            &mut output,
            &mut seen,
            "second".into(),
            "original legal text".into(),
        );
        assert_eq!(output.matches("original legal text").count(), 1);
        assert!(output.contains("under: first"));
    }
    #[test]
    fn policy_keeps_conjunctions_and_declines_optional_copyleft_alternatives() {
        assert_eq!(
            selected_terms("MIT OR Apache-2.0 OR LGPL-2.1-or-later").unwrap(),
            ["MIT"]
        );
        assert_eq!(
            selected_terms("(MIT OR Apache-2.0) AND Unicode-3.0").unwrap(),
            ["MIT", "Unicode-3.0"]
        );
        assert_eq!(selected_terms("MPL-2.0").unwrap(), ["MPL-2.0"]);
        assert!(selected_terms("MIT AND LicenseRef-Unknown").is_err());
        assert!(selected_terms("LicenseRef-Unknown").is_err());
    }
    #[test]
    fn missing_or_changed_release_notices_fail_verification() {
        let scratch = crate::tests::scratch();
        let notices = Notices {
            files: vec![("MPL-SOURCES.txt".into(), b"exact source".to_vec())],
        };
        assert!(notices.verify(&scratch.0).is_err());
        notices.write(&scratch.0).unwrap();
        fs::write(scratch.0.join("MPL-SOURCES.txt"), b"wrong").unwrap();
        assert!(notices.verify(&scratch.0).is_err());
    }
}
