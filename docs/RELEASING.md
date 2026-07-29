# Release and Homebrew publishing guide

This document is for maintainers. It describes the first publication and every later release.
The normal output is an immutable source archive, eight native binary archives, checksum file,
generated Homebrew formula, provenance attestation, GitHub release, and (when configured) a pull
request in the project tap.

## One-time publication setup

1. Create the canonical public GitHub repository and push the project.
2. Replace every `github.com/mr-lexus/wireseer` and repository placeholder with the canonical URL.
3. Create a public tap repository named `homebrew-tap` under the same owner when possible. Homebrew
   then exposes it as `mr-lexus/tap`.
4. Initialize that repository with `brew tap-new mr-lexus/tap`, or create the recommended `Formula/`
   directory and tap test workflows manually.
5. In the Wireseer repository, set the Actions variable `HOMEBREW_TAP_REPOSITORY` to the full
   `mr-lexus/homebrew-tap` name.
6. Add an Actions secret named `HOMEBREW_TAP_TOKEN`. Use a fine-grained token limited to that tap
   repository with contents and pull-request write access.
7. Protect the default branches and require CI/tap checks before merging.

If the tap variable is absent, releases still succeed and attach `wireseer.rb`; a maintainer can
copy it to `Formula/wireseer.rb` manually. This makes the first release possible before tap
automation is enabled.

## Publication channels

The release workflow prepares two usable channels immediately after the canonical GitHub repository
exists:

- **GitHub Releases** is the upstream binary channel. It carries every native archive, the source
  package, `SHA256SUMS`, the generated formula, and provenance attestations.
- **The project Homebrew tap** is the primary package-manager channel for macOS and Linux. The
  release opens a formula PR automatically; after its tap checks pass, merging/pulling it makes
  `brew install mr-lexus/tap/wireseer` available. The tap can publish optimized bottles through the
  standard `brew test-bot`/`brew pr-pull` workflow.

Publishing the crate on crates.io can later add `cargo install wireseer-tui` as a third source-build
channel. It requires confirming ownership of the crate name and configuring a crates.io token; it
is intentionally not performed by the current release workflow because crates.io versions are
permanent and cannot be overwritten.

## What is automated

Pushing a stable `vMAJOR.MINOR.PATCH` tag runs `.github/workflows/release.yml`. The workflow:

1. checks the tag against `Cargo.toml`, repository metadata, the changelog, formatting, tests, and
   Clippy on Rust 1.85;
2. runs `cargo package --locked`, which also verifies that the published source builds;
3. builds and smoke-tests native binaries for Intel/ARM64 macOS, GNU/musl x86-64/ARM64 Linux, and
   x86-64/ARM64 Windows on architecture-matched GitHub-hosted runners;
4. packages each binary with the README, changelog, and license;
5. renames the Cargo source package to `wireseer-VERSION.tar.gz`;
6. generates a stable formula from `packaging/homebrew/wireseer.rb.in` using that exact checksum;
7. computes one `SHA256SUMS` file over the source, formula, and all binary archives;
8. creates provenance attestations when the repository is public;
9. creates a GitHub release from the existing tag and uploads every artifact;
10. optionally opens a tap pull request containing `Formula/wireseer.rb`.

The formula builds from immutable, SHA-256-verified source and uses Homebrew's `std_cargo_args`,
which installs the `wireseer` binary into the keg using the locked dependency graph. It includes
non-interactive version and help tests. The tap workflows created by `brew tap-new` can subsequently
test the formula and publish Homebrew bottles; those bottles remain separate from upstream's generic
release archives.

## Release checklist

From a clean branch:

1. Decide the next Semantic Versioning number.
2. Update `version` in `Cargo.toml`, then run `cargo check` so `Cargo.lock` matches.
3. Move user-visible entries from `Unreleased` into a dated `## [VERSION]` changelog section and
   update its comparison links.
4. Regenerate screenshots after any visible UI or demo-data change:

   ```bash
   cargo run --example render_screenshots -- docs/screenshots
   git diff -- docs/screenshots
   ```

5. Confirm that the screenshots contain only the built-in `demo_state`: documentation-range
   `192.0.2.0/24`, locally administered dummy MAC addresses, fictional labels, and no copied logs.
6. Run the local checks:

   ```bash
   cargo fmt --all -- --check
   cargo test --all-targets --locked
   cargo clippy --all-targets --locked -- -D warnings
   cargo build --release --locked
   cargo package --locked
   ```

7. Review `SECURITY.md`, privacy/network behavior, new dependencies, and the generated package
   file list. New discovery behavior must be described explicitly in release notes.
8. Commit and push the release preparation.
9. Create and push the annotated tag only after CI passes:

   ```bash
   git tag -a v0.1.0 -m "Wireseer 0.1.0"
   git push origin v0.1.0
   ```

10. Review all GitHub release assets, compare their digests with `SHA256SUMS`, and verify one
    attestation with `gh attestation verify ASSET -R mr-lexus/wireseer`.
11. Review and merge the generated tap pull request after its `brew audit`, source build, and
    `brew test` checks succeed on supported macOS and Linux runners.
12. Verify the public path on both Apple Silicon and Intel macOS when available:

    ```bash
    brew install mr-lexus/tap/wireseer
    wireseer --version
    wireseer doctor
    brew test mr-lexus/tap/wireseer
    brew uninstall wireseer
    ```

Do not create the tag when any required check is failing. Never edit or replace an archive attached
to an existing version; publish a new patch version so the tag, URL, and checksum remain immutable.

## Test the formula before the first tag

The generator can be exercised against a local Cargo package without a release:

```bash
cargo package --allow-dirty --locked
mkdir -p target/homebrew-preview
cp target/package/wireseer-tui-0.1.0.crate target/homebrew-preview/wireseer-0.1.0.tar.gz
./scripts/generate-homebrew-formula.sh \
  0.1.0 \
  mr-lexus/wireseer \
  target/homebrew-preview/wireseer-0.1.0.tar.gz \
  target/homebrew-preview/wireseer.rb
ruby -c target/homebrew-preview/wireseer.rb
```

After the matching release URL exists, copy the formula into a local tap and run:

```bash
brew install --build-from-source mr-lexus/tap/wireseer
brew test mr-lexus/tap/wireseer
brew audit --new --formula mr-lexus/tap/wireseer
```

Homebrew core additionally requires project notability, a stable tagged release, and successful
builds across the current core CI matrix. Start with the upstream tap; consider a core submission
only after the project meets the current acceptance policy.

## Manual recovery

If GitHub release creation succeeds but tap automation fails, download `wireseer.rb` from the
release, copy it to the tap's `Formula/` directory, and open a normal pull request. Do not recompute
or hand-edit the checksum unless the formula points to a different immutable archive. Rerunning a
failed workflow is safe while the release assets have not been replaced.

## Runner and signing notes

The release matrix uses native GitHub-hosted runner labels rather than cross-linking Apple or MSVC
binaries from Linux. `windows-11-arm` and the ARM runner labels may be marked public preview by
GitHub; a failure blocks publication so a release is never silently missing an advertised target.

GitHub provenance attests the exact release files and can be verified with `gh attestation verify`.
It is not a substitute for Apple Developer ID notarization or Windows Authenticode. Version 0.1.0
does not perform either platform-signing flow because no signing identity or certificate is
configured. Checksums and provenance are therefore mandatory for direct-download instructions.
