//! Logical model-sized fixtures without allocating model-sized disk space.
use std::{fs::File, path::Path};

pub fn sparse_file(path: impl AsRef<Path>, bytes: u64) {
    let path = path.as_ref();
    let file = File::create(path).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::{Ioctl::FSCTL_SET_SPARSE, IO::DeviceIoControl};
        let mut returned = 0;
        // SAFETY: the file handle and output count stay valid for this synchronous call.
        // Null input means SetSparse=true; no input/output buffers or OVERLAPPED are used.
        // https://learn.microsoft.com/windows/win32/api/winioctl/ni-winioctl-fsctl_set_sparse
        let success = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_SPARSE,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(
            success,
            0,
            "mark {} sparse: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }
    file.set_len(bytes).unwrap();
    assert_eq!(file.metadata().unwrap().len(), bytes);
}
