# Release workflow (Windows PC only)

Scope: **Windows 10/11 x64 desktop builds only**. Android is not part of this
workflow. The `android/` directory in this worktree stays untracked and is neither
built, packaged, tagged nor published. Nothing in this document or in the scripts
creates a commit, tag, push or GitHub release by itself.

## Approval gates

Every step that changes shared state requires explicit maintainer approval:

| Step | Automatic? | Approval needed |
|---|---|---|
| Local checks and package build | Yes | No |
| Commit of the changes to be released | No | Yes |
| Push to `origin` | No | Yes |
| Lightweight/annotated version tag | No | Yes |
| GitHub release creation, assets, prerelease flag | No | Yes |
| Promoting a release candidate to stable | No | Yes |

The scripts in `scripts/` deliberately stop at the package boundary.

## Current release state

- Stable: **v0.6.1**, `prerelease = false`, tag `v0.6.1` from the committed worktree on branch `ai/rust-player-improvements`.
- Previous stable: **v0.6.0**, `prerelease = false`, tag `v0.6.0` at commit `d9b7ffd5f79815f6fa3f099f7e0bc6ae8000cdc6`. It remains unchanged.
- Superseded preview: **v0.6.0-rc1**, `prerelease = true`, tag `v0.6.0-rc1` at commit `f0e05adca0b405af7e9012956fa34be155b56633`. It remains unchanged.
- Stable ZIP SHA-256: `e11e1247c60eb2ebd0c3e6677a89db117d4be44381aa5d71b22212957f71621f`.
- The work after `f0e05ad` is included in `v0.6.0`: identity cleanup, backup fix, benchmark tooling, parity features, MP1/MP2, ADPCM, MKA/MKV, Monkey's Audio, DSD-to-PCM, ReplayGain, tag editor, bit-perfect preset, WavPack and the hardware verification guide. The facet filter fixes are included in `v0.6.1`.

## Prepare a release

1. Set the version in `Cargo.toml`. Use `vX.Y.Z-rcN` for a preview and `vX.Y.Z` for a stable release.
2. Update `docs/CHANGELOG.md` and `docs/RELEASE-NOTES-NEXT.md`. Both are English by convention because they are GitHub-facing.
3. Review `docs/TESTING.md` and `docs/AUDIO-TESTING.md`. A stable release requires a completed listening test of the audio handoff; a release candidate may be published with that check still open, but the release body must say so.
4. Run the local package script (section below). Do not publish artifacts that were produced from a dirty or unapproved commit.

## Build the package locally

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package-release.ps1 -Version v0.6.0-rc2
```

The script runs, in order:

```text
cargo fmt --check
cargo clippy --release --locked --all-targets -- -D warnings
cargo test --release --locked
cargo build --release --locked
```

It then assembles `target/release-package/<version>/`:

- `rust-player.exe` from `target/release/`
- `THIRD-PARTY-NOTICES.md`
- `CHANGELOG.md` and `RELEASE-NOTES-NEXT.md` from `docs/`
- `Rust-Player-<version>-Windows-x64.zip`
- `Rust-Player-<version>-Windows-x64.zip.sha256` with `<hash>  <filename>`

The archive is not byte-reproducible because ZIP metadata contains timestamps;
the checksum identifies the produced file and must be uploaded alongside it.
`v0.6.0-rc1` contained only the executable; including the notices and notes in the
ZIP is the corrected convention for later packages.

## CI package artifact

`.github/workflows/windows-package.yml` runs the same script on `windows-latest`.
It triggers on pushed `v*` tags or manually via `workflow_dispatch` with a version
input, and uploads the ZIP plus `.sha256` as the `windows-x64-package` artifact.

This is a **verification artifact only**: the workflow has `contents: read`, does
not create a release and does not upload to GitHub Releases. Publishing the
artifact still requires the approval gates above.

## Verify before publishing

- Extract the ZIP on a clean Windows 10 and a Windows 11 profile and start `rust-player.exe`.
- Confirm the SHA-256 of the downloaded archive:

```powershell
Get-FileHash -Algorithm SHA256 -Path Rust-Player-v0.6.0-rc2-Windows-x64.zip
```

- Work through the manual checklist in `docs/TESTING.md`. For audio changes, follow `docs/AUDIO-TESTING.md` and record the device, driver, source format, negotiated output format and underruns.
- Check that the release body matches `docs/CHANGELOG.md` and that the notes asset matches `docs/RELEASE-NOTES-NEXT.md`.

## Publish (only after approval)

1. Commit the reviewed source and documentation changes, then push them.
2. Create the tag on the pushed commit, for example `git tag -a v0.6.0-rc2 -m "v0.6.0-rc2"`.
3. Create the GitHub release at
   `https://github.com/AikasAcc/rust-player/releases/new` with the new tag, the
   changelog text as body, and `prerelease = true` for `rc` versions.
4. Upload four assets: the ZIP, its `.sha256` file, `CHANGELOG.md` and
   `RELEASE-NOTES-NEXT.md`.
5. Verify the published asset digests against `Get-FileHash` on the local files.

The `gh` CLI is not installed in the current environment. `v0.6.0-rc1` was created
and updated through the GitHub REST API with a token obtained from the local Git
Credential Manager (`git credential fill`). No token is stored in the repository or
in these scripts.

## Promotion and rollback

- Keep `prerelease = true` until the listening test and the manual checklist pass. Promoting an `rc` to stable is a separate, explicitly approved action.
- Do not rewrite or delete a published tag. If a stable release is broken, publish a patch release and mark the old one as superseded in its description.
- For a preview, assets may be replaced and the release description corrected, but the fact and reason must be recorded in the description.
- Never move the stable link in the README to a preview tag.

## Deliberately out of scope

- Android: no build, no artifact, no tag. The untracked `android/` tree is untouched by this workflow.
- Automatic commits, pushes, merges, tags and releases.
- Signing the unsigned executable and SmartScreen reputation.
- Native DSD/DoP (DSF/DFF and DSD inside WavPack are decoded to PCM), ASIO, VST and UPnP.
