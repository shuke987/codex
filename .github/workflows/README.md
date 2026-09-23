# Workflow Strategy

The workflows in this directory are split so that pull requests get fast, review-friendly signal while `main` still gets the full cross-platform verification pass.

## Independent exec-goal forks

Outside `openai/codex`, `blocking-ci.yml` requires the shared repository,
spelling, dependency-policy and blob-size checks plus `fork-exec-ci.yml`.
The fork job runs on standard Linux x64 hardware, builds the full CLI and its
matching code-mode host, runs all exec and code-mode-host tests, and checks
formatting, unused dependencies and scoped Clippy. It also runs after pushes
to `codex/exec-goal`. Its debug build is a CI smoke artifact, not a release.

The upstream full-platform Bazel and Rust workflows remain available through
manual dispatch. They are not part of the fork's Linux maintenance contract;
in particular, Windows voice-host builds have additional MSVC runtime license
and native-toolchain prerequisites. No cross-platform passing result should
be inferred from the Linux gate. Official repository checks remain unchanged.

The final required check verifies every active dependency. Only the explicitly
inactive profile may be skipped; a skipped, cancelled or failed Linux test job
still fails the fork's merge gate.

## Pull Requests

- Required checks run against GitHub's synthetic merge commit, not the pull
  request head alone. This includes changes already on `main` and catches
  conflicts before they reach the branch.
- `bazel.yml` is the main pre-merge verification path for Rust code.
  It runs Bazel `test` and Bazel `clippy` on the supported Bazel targets,
  including the generated Rust test binaries needed to lint inline `#[cfg(test)]`
  code.
- `rust-ci.yml` keeps the Cargo-native PR checks intentionally small:
  - `cargo fmt --check`
  - `cargo shear`
  - `argument-comment-lint` on Linux, macOS, and Windows
  - `tools/argument-comment-lint` package tests when the lint or its workflow wiring changes

## Post-Merge On `main`

- `bazel.yml` also runs on pushes to `main`.
  This re-verifies the merged Bazel path and helps keep the BuildBuddy caches warm.
- `rust-ci-full.yml` is the full Cargo-native verification workflow.
  It keeps the heavier checks off the PR path while still validating them after merge:
  - the full Cargo `clippy` matrix
  - the full Cargo `nextest` matrix via per-platform archive-backed shards
  - Windows ARM64 nextest archives cross-compiled on Windows x64, then replayed on native Windows ARM64 shards
  - release-profile Cargo builds
  - cross-platform `argument-comment-lint`
  - Linux remote-env tests

## Rule Of Thumb

- If a build/test/clippy check can be expressed in Bazel, prefer putting the PR-time version in `bazel.yml`.
- Keep `rust-ci.yml` fast enough that it usually does not dominate PR latency.
- Reserve `rust-ci-full.yml` for heavyweight Cargo-native coverage that Bazel does not replace yet.
