//! Unpacking an update and swapping it in for the running program, on packages built here the
//! way the release workflow builds them.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use craft_proton_sync_core::update::{BUNDLE_NAME, Package, binary_name};
use craft_proton_sync_native::update::{check, fetch, stage, swap_file};

fn binary() -> &'static str {
    binary_name(std::env::consts::OS)
}

/// A release `.zip`: the program in a folder named after the package, beside other files.
fn zip_package(path: &Path, program: &[u8]) {
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("craft-proton-sync-9.9.9-windows-x64-portable/README.md", options)
        .unwrap();
    zip.write_all(b"read me").unwrap();
    zip.start_file(format!("craft-proton-sync-9.9.9-windows-x64-portable/{}", binary()), options)
        .unwrap();
    zip.write_all(program).unwrap();
    zip.finish().unwrap();
}

/// A release `.tar.gz` holding `files`, each a path and its content.
fn tar_package(path: &Path, files: &[(&str, &[u8])]) {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        File::create(path).unwrap(),
        flate2::Compression::default(),
    ));
    for (name, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, name, *content).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
}

#[test]
fn the_program_comes_out_of_a_zip() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("package.zip");
    zip_package(&archive, b"new program");
    let staged = stage(&archive, Package::Zip, &dir.path().join("staged")).unwrap();
    assert_eq!(staged.file_name().unwrap(), binary());
    assert_eq!(fs::read(&staged).unwrap(), b"new program");
    assert_eq!(
        fs::read_dir(dir.path().join("staged")).unwrap().count(),
        1,
        "only the program is unpacked"
    );
}

#[test]
fn the_program_comes_out_of_a_tar_gz() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("package.tar.gz");
    let program = format!("craft-proton-sync-9.9.9-linux-x86_64/{}", binary());
    tar_package(
        &archive,
        &[
            ("craft-proton-sync-9.9.9-linux-x86_64/README.md", b"read me"),
            (&program, b"new program"),
        ],
    );
    let staged = stage(&archive, Package::TarGz, &dir.path().join("staged")).unwrap();
    assert_eq!(fs::read(&staged).unwrap(), b"new program");
}

#[test]
fn the_whole_bundle_comes_out_of_a_macos_package() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("package.app.tar.gz");
    let program = format!("{BUNDLE_NAME}/Contents/MacOS/{}", binary());
    let plist = format!("{BUNDLE_NAME}/Contents/Info.plist");
    tar_package(&archive, &[(&plist, b"<plist/>"), (&program, b"new program")]);
    let bundle = stage(&archive, Package::AppTarGz, &dir.path().join("staged")).unwrap();
    assert_eq!(bundle.file_name().unwrap(), BUNDLE_NAME);
    assert_eq!(
        fs::read(bundle.join("Contents").join("MacOS").join(binary())).unwrap(),
        b"new program"
    );
    assert_eq!(fs::read(bundle.join("Contents").join("Info.plist")).unwrap(), b"<plist/>");
}

#[test]
fn a_package_without_the_program_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("package.tar.gz");
    tar_package(&archive, &[("something/else.txt", b"nope")]);
    assert!(
        stage(&archive, Package::TarGz, &dir.path().join("a"))
            .unwrap_err()
            .contains("no program")
    );
    assert!(
        stage(&archive, Package::AppTarGz, &dir.path().join("b"))
            .unwrap_err()
            .contains("no program")
    );
    fs::write(dir.path().join("junk.zip"), b"this is not a zip").unwrap();
    assert!(stage(&dir.path().join("junk.zip"), Package::Zip, &dir.path().join("c")).is_err());
}

#[test]
fn the_new_program_takes_the_old_ones_place() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, new) = (dir.path().join("app").join(binary()), dir.path().join("staged").join(binary()));
    for (path, content) in [(&exe, "old program"), (&new, "new program")] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    swap_file(&new, &exe).unwrap();
    assert_eq!(fs::read_to_string(&exe).unwrap(), "new program");
    // The old one is kept beside it until the next start.
    let old = exe.with_file_name(format!("{}.old", binary()));
    assert_eq!(fs::read_to_string(old).unwrap(), "old program");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o111,
            0o111,
            "the new program can be run"
        );
    }
}

#[test]
fn a_swap_that_fails_leaves_the_old_program_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join(binary());
    fs::write(&exe, "old program").unwrap();
    assert!(swap_file(&dir.path().join("missing"), &exe).is_err());
    assert_eq!(fs::read_to_string(&exe).unwrap(), "old program");
}

/// The real thing, against the project's published releases: finds the latest, downloads this
/// computer's package, checks it against the release's checksums, unpacks it and runs it.
/// Ignored: it needs the network and a published release with a package for this platform.
/// `cargo test -p craft-proton-sync-native --test update -- --ignored`
#[test]
#[ignore = "downloads the latest published release"]
fn the_latest_release_downloads_verifies_and_runs() {
    assert_eq!(
        check("999.0.0").unwrap(),
        None,
        "nothing is newer than a version from the far future"
    );
    let release = check("0.0.1").unwrap().expect("a published release");
    let dir = tempfile::tempdir().unwrap();
    let mut calls = 0;
    let (staged, _) = fetch(&release, &dir.path().join("update"), &mut |_, _| calls += 1).unwrap();
    assert!(calls > 0, "the download reported no progress");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = std::process::Command::new(&staged).arg("--version").output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains(&release.version), "{said}");
}

/// The swap happens while the old program is running: a running program can be moved aside but
/// (on Windows) not overwritten.
#[test]
fn a_running_program_can_be_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, new) = (dir.path().join(binary()), dir.path().join("new-program"));
    // Any program that keeps running will do: the shell on Windows, `sleep` elsewhere.
    let (stand_in, args): (std::path::PathBuf, &[&str]) = if cfg!(windows) {
        (std::env::var_os("ComSpec").unwrap().into(), &[])
    } else {
        ("/bin/sleep".into(), &["30"])
    };
    fs::copy(stand_in, &exe).unwrap();
    fs::write(&new, "new program").unwrap();
    let mut running = std::process::Command::new(&exe)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let swapped = swap_file(&new, &exe);
    let replaced = fs::read_to_string(&exe);
    running.kill().unwrap();
    running.wait().unwrap();
    swapped.unwrap();
    assert_eq!(replaced.unwrap(), "new program");
}
