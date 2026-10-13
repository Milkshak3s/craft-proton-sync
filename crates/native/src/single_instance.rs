//! One running copy per user: two would sync the same folder against each other.

use std::fs::{self, File, TryLockError};
use std::path::Path;

/// Held for as long as this is the running copy.
pub struct InstanceLock {
    _file: File,
}

pub enum Claim {
    Primary(InstanceLock),
    /// Another copy is already running.
    Secondary,
}

pub fn claim(path: &Path) -> Result<Claim, String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| format!("couldn't open {} ({e})", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Claim::Primary(InstanceLock { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(Claim::Secondary),
        Err(TryLockError::Error(e)) => Err(format!("couldn't lock {} ({e})", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_second_claim_is_secondary_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instance.lock");
        let first = claim(&path).unwrap();
        assert!(matches!(first, Claim::Primary(_)));
        assert!(matches!(claim(&path).unwrap(), Claim::Secondary));
        drop(first);
        assert!(matches!(claim(&path).unwrap(), Claim::Primary(_)));
    }
}
