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

## Fork release candidates

`fork-release-candidate.yml` builds a Linux x64 GNU release package on Ubuntu
24.04 using standard hosted hardware. Run it manually with an optional full,
lowercase commit SHA in `source_ref`; leaving it empty resolves the current
`codex/exec-goal` head once at the start. Only commits already on that maintenance
branch are accepted. The workflow and smoke-test revision are recorded separately
from the selected source revision. Pull requests changing the workflow or its
smoke script also exercise the candidate build against the maintenance head.

GitHub requires a workflow-dispatch entrypoint on the default branch. This fork uses
`codex/exec-goal` as its default, retaining `main` for upstream tracking. The Run
workflow button becomes available after this workflow is merged there.

```bash
gh workflow run fork-release-candidate.yml --repo shuke987/codex \
  --ref codex/exec-goal -f source_ref=<full-maintenance-commit-sha>
```

The job uses locked Cargo dependencies and checksum-verified V8/resources, builds
with the release profile (debug information disabled), and assembles the canonical
CLI package, including the matching code-mode host, bwrap, ripgrep and patched zsh.
It extracts the final archive, verifies checksums, and runs an isolated local-model
smoke test: capacity failure, resume of the same thread, packaged ripgrep execution
through code mode, goal completion and a successful terminal event.

Successful runs upload a 30-day Actions artifact with the `.tar.gz`, `SHA256SUMS`
and `build-info.json`. Verify the outer checksums before extraction; the package
also contains a per-file checksum manifest. Build provenance includes source and
workflow commits, target, compiler, lockfile digest and run URL. Download via the
run's Artifacts section or `gh run download <run-id> --repo shuke987/codex`.

This is an Ubuntu 24.04 GNU candidate, not a portable musl distribution or a full
multi-platform/voice release. Real review helper acceptance, live-model tests and
production installation remain separate. The workflow creates no release/tag and
has no publishing credentials or OSS upload step.

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
