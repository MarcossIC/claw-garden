//! Directories under the data directory, kept private to their owner.
//!
//! On Unix every directory clawg creates there is born 0700, and one that
//! already exists with group or other bits loses them: with the directory
//! closed, another user of the machine reaches nothing inside it, whatever
//! mode `memory.db`, its WAL, the backups or the logs were created with. On
//! Windows the files inherit the profile's ACL, as `token` does, so only the
//! creation is left.

use std::path::Path;

/// Creates `dir` and its missing parents private to the owner, and takes
/// group and other bits away from `dir` when it already has them.
///
/// Only `dir` is tightened: an existing parent (the home directory, for the
/// data directory) is not clawg's to change. A directory whose mode cannot be
/// changed, one on a file system without Unix modes or one another user owns,
/// is logged and used as it is, since refusing it would leave the user's
/// memory unreachable over a protection it never had.
#[cfg(unix)]
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    if !dir.is_dir() {
        // `mode` is masked by the umask, which can only take bits away from
        // 0700; the parents it has to create get the same mode.
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let mode = std::fs::metadata(dir)?.permissions().mode();
    if let Some(private) = tightened(mode) {
        match std::fs::set_permissions(dir, std::fs::Permissions::from_mode(private)) {
            Ok(()) => log::warn!(
                "{} was {:04o}; made it {:04o} so other users of this machine cannot read it",
                dir.display(),
                mode & 0o7777,
                private
            ),
            Err(e) => log::warn!(
                "{} is {:04o} and could not be made private to its owner, so other users of this machine may read it: {e}",
                dir.display(),
                mode & 0o7777
            ),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// The permission bits of `mode` without group and other ones, or `None`
/// when it has none to take.
#[cfg(unix)]
fn tightened(mode: u32) -> Option<u32> {
    (mode & 0o077 != 0).then_some(mode & 0o7700)
}

/// The permission bits of `path`, for the tests of every place that creates
/// a directory under the data directory.
#[cfg(all(test, unix))]
pub(crate) fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn only_group_and_other_bits_are_taken() {
        assert_eq!(tightened(0o40755), Some(0o700));
        assert_eq!(tightened(0o750), Some(0o700));
        assert_eq!(tightened(0o2775), Some(0o2700));
        assert_eq!(tightened(0o40700), None);
        assert_eq!(tightened(0o500), None);
    }

    #[test]
    fn a_missing_directory_and_its_parents_are_created_private() {
        let home = crate::test_dir::test_dir();
        set_mode(home.path(), 0o755);
        let data = home.path().join(".clawg-dev");
        let nested = data.join("workspaces").join("agent");

        ensure_private_dir(&nested).unwrap();

        assert_eq!(mode_of(&data), 0o700);
        assert_eq!(mode_of(&data.join("workspaces")), 0o700);
        assert_eq!(mode_of(&nested), 0o700);
        assert_eq!(mode_of(home.path()), 0o755);
    }

    #[test]
    fn an_open_directory_is_tightened_and_the_rest_left_alone() {
        let home = crate::test_dir::test_dir();
        set_mode(home.path(), 0o755);
        let data = home.path().join(".clawg-dev");
        std::fs::create_dir(&data).unwrap();
        set_mode(&data, 0o755);
        let db = data.join("memory.db");
        std::fs::write(&db, b"rows").unwrap();
        set_mode(&db, 0o644);

        ensure_private_dir(&data).unwrap();

        assert_eq!(mode_of(&data), 0o700);
        assert_eq!(mode_of(&db), 0o644);
        assert_eq!(std::fs::read(&db).unwrap(), b"rows");
        assert_eq!(mode_of(home.path()), 0o755);
    }

    #[test]
    fn a_private_directory_is_left_as_it_is() {
        let home = crate::test_dir::test_dir();
        let data = home.path().join(".clawg-dev");
        std::fs::create_dir(&data).unwrap();
        set_mode(&data, 0o500);

        ensure_private_dir(&data).unwrap();

        assert_eq!(mode_of(&data), 0o500);
        set_mode(&data, 0o700);
    }
}
