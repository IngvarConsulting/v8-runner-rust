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

pub fn private_temp_dir(work_path: &Path) -> std::io::Result<PrivateTempDir> {
    let root = temp_root(work_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = tempfile::Builder::new()
            .prefix("applied-extension-inventory-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(root)?
            .keep();
        Ok(PrivateTempDir { path: Some(path) })
    }
    #[cfg(windows)]
    {
        windows_private_temp_dir(&root)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = root;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "private temporary directories are not supported on this platform",
        ))
    }
}

#[cfg(windows)]
fn windows_private_temp_dir(root: &Path) -> std::io::Result<PrivateTempDir> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, GetVolumeInformationW, GetVolumePathNameW,
    };
    use windows_sys::Win32::System::WindowsProgramming::FS_PERSISTENT_ACLS;

    // A successful CreateDirectoryW does not apply SECURITY_ATTRIBUTES on a
    // volume without persistent ACLs. Refuse before writing a complete CFE.
    let root_wide: Vec<u16> = root
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut volume_root = vec![0u16; 32768];
    if unsafe {
        GetVolumePathNameW(
            root_wide.as_ptr(),
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
            "snapshot volume does not support persistent access controls",
        ));
    }

    // Protected DACL: only the owner and LocalSystem can read/write the
    // directory and its children. No inherited workspace ACL can widen it.
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
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
    let result = (|| {
        for _ in 0..8 {
            let path = root.join(format!(
                "applied-extension-inventory-{}",
                uuid::Uuid::new_v4()
            ));
            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } != 0 {
                return Ok(PrivateTempDir { path: Some(path) });
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique private snapshot directory",
        ))
    })();
    unsafe { LocalFree(descriptor) };
    result
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

/// Return the reserved future EDT work directory for a source set.
pub fn reserved_source_set_dir(work_path: &Path, source_set_name: &str) -> PathBuf {
    work_path.join("designer").join(source_set_name)
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
        private_temp_dir, reserved_source_set_dir,
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
    fn reserved_source_set_path_is_not_created() {
        let dir = tempdir().expect("tempdir");
        let reserved = reserved_source_set_dir(dir.path(), "main");

        assert!(!reserved.exists());
        assert!(reserved.ends_with("designer/main"));
    }

    #[test]
    fn private_snapshot_directory_is_writable_and_removed() {
        let work = tempdir().expect("work");
        let private = private_temp_dir(work.path()).expect("private snapshot dir");
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
