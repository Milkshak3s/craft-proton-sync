//! Downloads the pinned Proton Drive CLI for this platform, checks it against the compiled-in
//! SHA-512 and runs it.
//!
//! Ignored by default because the CLI is a 100 MB download; CI runs it on every platform:
//! `cargo test -p craft-proton-sync-native --test cli -- --ignored`

use craft_proton_sync_core::drive;
use craft_proton_sync_native::install;
use craft_proton_sync_native::paths::Paths;

#[test]
#[ignore = "downloads the Proton Drive CLI"]
fn the_pinned_cli_downloads_verifies_and_runs() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().join("data"));

    let mut calls = 0;
    let cli = install::ensure(&paths, &mut |_, _| calls += 1).unwrap();
    assert!(calls > 0, "the download reported no progress");
    assert!(cli.bin().is_file());
    // Installed already: no second download.
    install::ensure(&paths, &mut |_, _| panic!("downloaded again")).unwrap();

    let version = cli.output(["version"]).unwrap();
    let said = String::from_utf8_lossy(&version.stdout);
    assert!(version.status.success(), "{said}");
    assert!(said.contains(drive::VERSION), "{said}");
}
