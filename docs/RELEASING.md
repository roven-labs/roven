# Releasing Roven

Use `Cargo.toml` as the version source. Keep `Cargo.lock`, the changelog, and
user-facing setup instructions in sync with it. The V2 project snapshot and
registration changes are breaking, so their eventual release is `2.0.0`.

## Release checklist

1. Update `version` in `Cargo.toml`.
2. Run `cargo check` to refresh the package entry in `Cargo.lock`.
3. Add the release entry to `CHANGELOG.md`.
4. Update release-facing documentation if install behavior changed.
5. Run the checks locally:

   ```powershell
   cargo fmt --all -- --check
   cargo check --all-targets
   cargo test --all-targets
   cargo clippy --all-targets -- -D warnings
   cargo build --release
   ```

6. Review the diff and create a tag matching the package version:

   ```powershell
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

The release workflow rebuilds the Windows executable, packages it with the
user documentation, writes a SHA-256 checksum, and creates the GitHub release.

## Rollback

If a release is faulty, mark its GitHub release as a pre-release or remove it,
then direct users to the previous release. Fix the problem on a new branch and
publish the next patch version. Do not reuse a published tag.
