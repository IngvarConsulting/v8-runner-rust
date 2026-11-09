use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

/// Private scratch space for complete database snapshots. The directory is
/// created with its restricted access policy before a CFE is written to it.
pub struct PrivateTempDir {
    path: Option<PathBuf>,
}

impl PrivateTempDir {
    pub fn path(&self) -> &Path {
        self.path.as_deref().expect("private temp dir is open")
    }

    /// Report cleanup failures instead of silently leaving a full CFE behind.
    pub fn close(mut self) -> std::io::Result<()> {
        let path = self.path.as_ref().expect("private temp dir is open");
        std::fs::remove_dir_all(path)?;
        self.path = None;
        Ok(())
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

pub fn private_temp_dir(work_path: &Path, prefix: &str) -> std::io::Result<PrivateTempDir> {
    let root = temp_root(work_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = tempfile::Builder::new()
            .prefix(prefix)
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(root)?
            .keep();
        Ok(PrivateTempDir { path: Some(path) })
    }
    #[cfg(windows)]
    {
        windows_private_temp_dir(&root, prefix)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, prefix);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "private temporary directories are not supported on this platform",
        ))
    }
}

#[cfg(windows)]
fn windows_private_temp_dir(root: &Path, prefix: &str) -> std::io::Result<PrivateTempDir> {
    for _ in 0..8 {
        let path = root.join(format!("{prefix}{}", uuid::Uuid::new_v4()));
        match create_private_dir(&path) {
            Ok(()) => return Ok(PrivateTempDir { path: Some(path) }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique private snapshot directory",
    ))
}

/// Creates one directory that only its owner may open: mode 0700 on Unix, a protected
/// DACL for the owner and LocalSystem on Windows. The parent must exist; an existing
/// directory is `AlreadyExists` and keeps whatever access it had.
#[cfg(unix)]
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

/// Creates one directory that only its owner may open: mode 0700 on Unix, a protected
/// DACL for the owner and LocalSystem on Windows. The parent must exist; an existing
/// directory is `AlreadyExists` and keeps whatever access it had.
#[cfg(windows)]
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, GetVolumeInformationW, GetVolumePathNameW,
    };
    use windows_sys::Win32::System::WindowsProgramming::FS_PERSISTENT_ACLS;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // A successful CreateDirectoryW does not apply SECURITY_ATTRIBUTES on a
    // volume without persistent ACLs. Refuse before anything private is written.
    let mut volume_root = vec![0u16; 32768];
    if unsafe {
        GetVolumePathNameW(
            wide.as_ptr(),
            volume_root.as_mut_ptr(),
            volume_root.len() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut fs_flags = 0u32;
    if unsafe {
        GetVolumeInformationW(
            volume_root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut fs_flags,
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if fs_flags & FS_PERSISTENT_ACLS == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "the volume does not support persistent access controls",
        ));
    }

    with_owner_only_security(|attributes| {
        if unsafe { CreateDirectoryW(wide.as_ptr(), attributes) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    })?
}

/// Hands `create` security attributes that let only the owner and LocalSystem in: a protected
/// DACL, so no inherited ACL can widen it. The one place that spells the owner-only descriptor:
/// private directories and the live agent keeper's named pipe take it from here.
#[cfg(windows)]
pub fn with_owner_only_security<R>(
    create: impl FnOnce(&windows_sys::Win32::Security::SECURITY_ATTRIBUTES) -> R,
) -> std::io::Result<R> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is a null-terminated UTF-16 string; on success `descriptor` holds memory
    // to `LocalFree`, freed below once.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let result = create(&attributes);
    // SAFETY: allocated by the conversion above; `attributes` is not used after this.
    unsafe { LocalFree(descriptor) };
    Ok(result)
}

#[cfg(not(any(unix, windows)))]
pub fn create_private_dir(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "private directories are not supported on this platform",
    ))
}

/// Return the root temp directory inside `work_path`.
pub fn temp_root(work_path: &Path) -> std::io::Result<PathBuf> {
    let dir = work_path.join("temp");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Return the platform log directory inside `work_path`.
pub fn platform_logs_dir(work_path: &Path) -> std::io::Result<PathBuf> {
    let dir = work_path.join("logs").join("platform");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Return the temp directory for partial load/dump lists inside `work_path`.
pub fn partial_lists_dir(work_path: &Path) -> std::io::Result<PathBuf> {
    let dir = temp_root(work_path)?.join("partial-lists");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Return the EDT export directory of a tool extension inside `work_path`.
///
/// It lives outside the Designer copies of source sets, so a source
/// set may carry any name without sharing a directory with a tool extension.
pub fn tool_extension_export_dir(work_path: &Path, extension_name: &str) -> PathBuf {
    work_path.join("tool-extensions").join(extension_name)
}

/// Create a temporary text file for a partial load list inside `work_path/temp/partial-lists`.
pub fn partial_list_file(work_path: &Path) -> std::io::Result<NamedTempFile> {
    tempfile::Builder::new()
        .prefix("partial-list-")
        .suffix(".txt")
        .tempfile_in(partial_lists_dir(work_path)?)
}

/// Create a temporary text file for a partial dump object list inside
/// `work_path/temp/partial-lists`.
pub fn dump_object_list_file(work_path: &Path) -> std::io::Result<NamedTempFile> {
    tempfile::Builder::new()
        .prefix("dump-object-list-")
        .suffix(".txt")
        .tempfile_in(partial_lists_dir(work_path)?)
}

#[cfg(test)]
mod tests {
    use super::{
        dump_object_list_file, partial_list_file, partial_lists_dir, platform_logs_dir,
        private_temp_dir,
    };
    use tempfile::tempdir;

    #[test]
    fn creates_new_temp_layout_under_work_path() {
        let dir = tempdir().expect("tempdir");

        let partial_dir = partial_lists_dir(dir.path()).expect("partial dir");
        let logs_dir = platform_logs_dir(dir.path()).expect("logs dir");

        assert!(partial_dir.ends_with("temp/partial-lists"));
        assert!(logs_dir.ends_with("logs/platform"));
    }

    #[test]
    fn creates_temp_files_in_new_locations() {
        let dir = tempdir().expect("tempdir");

        let partial = partial_list_file(dir.path()).expect("partial file");
        let dump_partial = dump_object_list_file(dir.path()).expect("dump partial file");
        assert!(partial
            .path()
            .to_string_lossy()
            .contains("temp/partial-lists"));
        assert!(dump_partial
            .path()
            .to_string_lossy()
            .contains("temp/partial-lists"));
    }

    #[test]
    fn private_snapshot_directory_is_writable_and_removed() {
        let work = tempdir().expect("work");
        let private = private_temp_dir(work.path(), "applied-extension-inventory-")
            .expect("private snapshot dir");
        let path = private.path().to_owned();
        std::fs::write(path.join("applied.cfe"), b"private").expect("write private snapshot");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        private.close().expect("remove private snapshot");
        assert!(!path.exists());
    }
}
