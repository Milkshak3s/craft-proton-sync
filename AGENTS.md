# AGENTS.md: guide for AI agents and contributors

Craft Proton Sync keeps what people save from the **Craft App Suite** (PhotoCraft, LightCraft,
FilmCraft, DesignCraft, VectorCraft, PdfCraft, EffectCraft, WordCraft, DeckCraft, GridCraft,
CADCraft, SoundCraft) in step with **Proton Drive**. It is a small native tray app in **Rust
only**: egui/eframe, no Tauri, Electron or webview. Its layout and rules follow
[ArtCraft Launcher](https://github.com/storytold/craft-launcher).

It is an independent project: not part of ArtCraft, not affiliated with Proton AG. The product
name is always **Craft Proton Sync** in user-facing text; machine names are `craft-proton-sync`
and `craft-proton-sync-*`.

## 1. Workspace map

```text
crates/
  core      craft-proton-sync-core      pure, no I/O: where files go (layout), what a sync does
                                        (plan), the pinned Proton Drive CLI and its command lines
                                        and answers (drive), the app's own releases (update), the
                                        UI/engine protocol, settings
  native    craft-proton-sync-native    desktop backend: downloads and checks the CLI (install),
                                        runs it (drive), signs in (account), one sync (sync), the
                                        watching and scheduling thread (engine), updating the
                                        app itself (update), login item
  ui-egui   craft-proton-sync-ui-egui   the egui UI and theme
apps/
  craft-proton-sync                     desktop shell (eframe): window, tray icon
packaging/                              macOS bundle script, Linux desktop entry
```

Layering: `core` depends on no other workspace crate. `native` and `ui-egui` use `core` and not
each other: the UI talks to the engine only through `core::protocol::Backend` and to the desktop
through `ui_egui::Host`. Nothing below `apps/` opens windows or tray icons.

## 2. How it works

- **The path is the contract.** `<Documents>/ArtCraft/<App>/<file>` ⇄ `/ArtCraft/<App>/<file>`
  in Proton Drive, mirrored one to one (`core::layout`). Don't change it without a migration.
- **Proton's Drive CLI does the talking.** Proton publishes no Rust client, so the app drives
  Proton's official CLI (`core::drive`, `native::drive`): one CLI in interactive mode per sync
  (`Session`), fed one command at a time, answering in JSON. (rclone's Proton Drive backend was
  tried first and dropped: Proton rejects most of its writes.)
- **The sync is ours.** The CLI has no sync command. `core::plan` compares three pictures (both
  sides now, and both as they last agreed, kept in `sync-state.json`) and decides; `native::sync`
  carries it out through the `Remote` trait, which tests implement with a plain folder.
- **When it syncs:** a few seconds after the folder goes quiet following a change (a file
  watcher), and on a timer for changes made elsewhere. One sync at a time, on the engine thread.
- **Sign-in is Proton's.** `auth login` prints a page on `account.proton.me`; the app opens it and
  waits. The session lives in the OS secret store, kept there by the CLI.

## 3. Golden rules

### Never crash

A bad network response, a half-downloaded file, a full disk, a corrupt settings file or the CLI
misbehaving must produce an error the user can act on, never a panic.

- **Non-test code never panics.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`
  or `unimplemented!`. Every production crate root carries the `#![deny(clippy::unwrap_used, …)]`
  line; keep it.
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`.
- **Everything from the network or from the CLI's output is hostile.** Cap sizes, use `get()`
  instead of indexing, parse leniently.

### Running other people's software safely

- The CLI is downloaded **only** from `https://proton.me/download/drive/cli`, **only** the
  version pinned in `core::drive::VERSION`, and is run **only** after it matches the SHA-512
  compiled into `core::drive::ASSETS`. To update it, change the version and all six checksums in
  one change, from `https://proton.me/download/drive/cli/version.json`, run the `cli` test, and
  sync against a real account: the CLI is pre-release and its commands and JSON may change.
- Never add a second download source or an environment variable that redirects the download.
- Every name that goes into a CLI command line is quoted by `core::drive`, which refuses names
  that could break out of the quotes. Never build a command line anywhere else.
- Only a page on `https://account.proton.me/` is ever opened in the browser (`drive::sign_in_url`),
  and never through a shell.

### Updating the app itself

- Updates come **only** from `https://github.com/Milkshak3s/craft-proton-sync/releases`:
  `core::update::parse_release` drops any file that isn't at this project's download address for
  that tag, and a package is installed only after it matches the release's `SHA256SUMS.txt`.
  Never add another source, and never install a package without its checksum.
- Only published releases count: no drafts, no pre-releases, and never a version that isn't
  newer than the running one (no downgrades).
- Installing without asking is **opt-in** (`Settings::auto_update`, off by default). Don't turn
  it on for the user, and don't add other unprompted changes to the program.
- The old program is moved aside, not deleted, until the new one is in place, and moved back if
  the swap fails (`update::swap_file`). An update runs on the engine thread, so never mid-sync.
- Development builds don't update themselves (`update::supported`).
- The release file names in section 5 are what the updater looks for (`core::update::package`).
  Renaming one breaks updates for everyone already installed: change both together, and keep
  the old name published for a release or two.

### The user's password and files

- The app never handles the password: sign-in is on Proton's page. Never add a password field,
  and never read, copy or log the CLI's session.
- **Nothing is deleted for good.** Local deletions go to the Recycle Bin / Trash
  (`sync::recycle`), remote ones to Proton Drive's trash (`filesystem trash`, never `delete` or
  `empty-trash`). Uploads over a file are new revisions (`create-new-revision`).
- **A side that can't be read is never taken for an empty side.** A failed listing ends the sync;
  `plan::guard` and the missing-folder check in `sync.rs` refuse to empty a side that vanished.
  Don't weaken them without a test that shows what replaces them.
- A file that changed again since it was looked at is left for the next sync (`apply` re-checks).
- Never sync anywhere but the signed-in account's `/my-files/ArtCraft`.

### Look and feel

Hard square corners, 1 px hairlines instead of shadows, upper-case monospace labels and buttons,
and ArtCraft Launcher's colour tokens by name (`theme::Tokens`). Never hard-code a colour.
ArtCraft's logos, marks and fonts are trademarks or separately licensed and are **not** used here.

### Other rules

1. **Tests are the gate.** The planner in `core` gets unit tests; whole syncs run against a
   folder standing in for Proton Drive in `crates/native/tests/sync.rs`; the UI is driven
   headlessly in `crates/ui-egui/tests/ui.rs`; `crates/native/tests/cli.rs` downloads and runs
   the real CLI (ignored by default: 100 MB; CI runs it); `crates/native/tests/update.rs` unpacks
   and swaps in updates built on the spot, and has an ignored test that downloads and runs the
   latest published release (run it by hand after publishing one).
2. Nothing in CI talks to a real Proton account. A change to `native::drive`, `account.rs` or the
   pinned CLI version needs a manual sign-in and sync against a real account before it is merged;
   say in the PR that it was done.
3. Requests use a generic `Craft-Proton-Sync/<version>` User-Agent and carry nothing personal.

## 4. Before you finish a task

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p craft-proton-sync-native --test cli --locked -- --ignored
```

## 5. Releasing

The version lives only in `[workspace.package] version` in the root `Cargo.toml`. Pushing to the
`release` branch runs `.github/workflows/release.yml`, which builds every platform and creates
the **draft** GitHub Release **Craft Proton Sync v<version>** with `SHA256SUMS.txt`. Publish the
draft by hand once it has been tried.

| Platform | Files |
|---|---|
| Windows | `craft-proton-sync-<ver>-windows-{x64,arm64}-portable.zip` |
| macOS | `craft-proton-sync-<ver>-macos-universal.dmg` (ad-hoc signed, not notarized), and `…-macos-universal.app.tar.gz`, the bundle alone, for self-updates |
| Linux | `craft-proton-sync-<ver>-linux-{x86_64,aarch64}.tar.gz` |

Builds are not code-signed yet; installers (MSI, AppImage, .deb, .rpm) are not built yet.
