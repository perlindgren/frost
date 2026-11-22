# Original legal texts, not vendored implementation code

This directory contains license/attribution documents only. Dependency source,
versions and registry/git caches are not modified.

Some crates do not publish repository-level legal files in their crate archive.
`upstream.tsv` maps exact package name, version and distributed source URL to
reviewed copies from the upstream commit recorded in that crate's
`.cargo_vcs_info.json`. It records the original URL and SHA-256 for each copy.
Multiple crates may point to the same text. `standard/sources.tsv` similarly
records a pinned SPDX license-list snapshot and hashes for canonical terms.

Original legal files present in a resolved package are also collected directly
at packaging time, including NOTICE/COPYRIGHT texts. Identical text is reproduced
once with cross references; package-specific attribution is retained. License
alternatives are selected by a small explicit policy; unknown obligations stop
packaging for review rather than being treated as permissive.

A few upstream repositories provide only a license declaration or explanatory
LICENSE.md, without a full permission text or a copyright-holder notice.
The collector retains that declaration and the canonical license terms, and
labels Cargo author fields as author metadata rather than inventing ownership or
years. In particular `dispatch 0.2.0` has a pinned declaration record. These
records are evidence of upstream licensing, not a legal determination of rights.
The objc2 upstream legal explanation, including its Apple SDK discussion, is
retained verbatim; these tools do not resolve that upstream legal question.

Run `python nice-tuner/xtask/scripts/check-release-licenses.py` to verify the
recorded copies. If dependencies change, packaging fails on missing evidence or
unreviewed license terms. Review upstream notices at the new exact revision;
do not silently reuse another version's file, infer copyright from authorship,
or edit the dependency to satisfy the checker.

The generated `DEPENDENCIES.tsv` describes the resolved normal-dependency graph
for the selected roots/target/features. It conservatively includes normal-edge
proc macros and code the linker might discard. Build-only and dev-only edges
are excluded. It is not a binary scanner and does not account for independently
added native DLLs, drivers or SDK redistributables. Such additions require their
own licensing/packaging review before distribution.
