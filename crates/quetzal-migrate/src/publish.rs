//! Destination publication never overwrites or exposes a partially built wallet.
use anyhow::{ensure, Context, Result};
use std::{
    ffi::CString,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};
pub fn destination(source: &Path, target: &Path) -> Result<PathBuf> {
    let name = target
        .file_name()
        .context("destination must name a new directory")?;
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent
        .canonicalize()
        .context("destination parent must exist")?;
    let target = parent.join(name);
    match fs::symlink_metadata(&target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
        Ok(_) => anyhow::bail!("destination already exists; migration never overwrites or merges"),
    }
    let source = source.canonicalize()?;
    ensure!(
        !(source.is_dir() && target.starts_with(&source)),
        "destination must be outside the source directory"
    );
    ensure!(target != source, "source and destination overlap");
    Ok(target)
}
pub fn publish(stage: &Path, target: &Path) -> Result<()> {
    let source = CString::new(stage.as_os_str().as_bytes())?;
    let target_c = CString::new(target.as_os_str().as_bytes())?;
    // Linux is the qualified platform. RENAME_NOREPLACE also closes the race
    // between destination preflight and publication.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("cannot publish migration without replacing destination");
    }
    fs::File::open(target.parent().context("destination parent missing")?)?
        .sync_all()
        .context("migration published but destination parent sync failed")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_destination_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        fs::write(stage.path().join("state"), b"complete").unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("sentinel"), b"original").unwrap();
        assert!(publish(stage.path(), &target).is_err());
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"original");
        assert!(stage.path().join("state").exists());
    }
    #[test]
    fn staging_is_published_once_and_source_overlap_rejected() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        assert!(destination(&source, &source.join("target")).is_err());
        let target = destination(&source, &root.path().join("target")).unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        fs::write(stage.path().join("receipt"), b"verified").unwrap();
        publish(stage.path(), &target).unwrap();
        assert_eq!(fs::read(target.join("receipt")).unwrap(), b"verified");
        assert!(destination(&source, &target).is_err());
    }
}
