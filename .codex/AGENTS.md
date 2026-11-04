# Instructions for all coding agents

These rules apply to the entire frost repository and all Cargo workspace members.

## Dependencies — mandatory rule

- NEVER modify third-party dependency source, including vendored copies,
  registry caches, or checked-out dependencies.
- Do not patch, fork, replace, or change dependency versions.
- Enabling existing dependency features in the application's Cargo.toml is
  allowed. This does not authorize editing the dependency itself.
- Implement changes in project-owned code using public dependency APIs.
- If those APIs cannot support the task, explain the limitation. Do not work
  around it by altering a dependency.
- Existing local dependency modifications do not grant permission to make
  further dependency changes.
