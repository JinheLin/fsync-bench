use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMethod {
    Fsync,
    Fdatasync,
}

impl SyncMethod {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fsync => "fsync",
            Self::Fdatasync => "fdatasync",
        }
    }

    /// On Linux these map to fsync(2) and fdatasync(2). Retry only EINTR.
    pub fn sync(self, file: &File) -> io::Result<()> {
        loop {
            let result = match self {
                Self::Fsync => file.sync_all(),
                Self::Fdatasync => file.sync_data(),
            };
            match result {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}

extern "C" {
    // Explicit 64-bit offsets avoid off_t ABI differences on 32-bit Linux.
    fn fallocate64(fd: c_int, mode: c_int, offset: i64, len: i64) -> c_int;
    fn posix_fadvise64(fd: c_int, offset: i64, len: i64, advice: c_int) -> c_int;
}

/// Allocate space and set EOF now. KEEP_SIZE would let later writes grow EOF.
pub fn preallocate(file: &File, file_size: u64) -> io::Result<()> {
    let len = i64::try_from(file_size)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "fallocate size exceeds i64"))?;
    if len == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty allocation",
        ));
    }
    loop {
        // SAFETY: live descriptor, integer parameters and the declared Linux C ABI.
        if unsafe { fallocate64(file.as_raw_fd(), 0, 0, len) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Advise dropping only this file's clean pages, after its preparation sync.
pub fn discard_clean_pages(file: &File) -> io::Result<()> {
    const POSIX_FADV_DONTNEED: c_int = 4;
    // SAFETY: live descriptor; len=0 means through EOF; advice has no pointers.
    let code = unsafe { posix_fadvise64(file.as_raw_fd(), 0, 0, POSIX_FADV_DONTNEED) };
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

/// Deterministic data generation, called only outside timed loops.
pub fn fill_payload(data: &mut [u8]) {
    let mut state = 0x6a09_e667_f3bc_c909_u64;
    for byte in data {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }
}

pub fn payload(size: usize) -> Vec<u8> {
    let mut data = vec![0; size];
    fill_payload(&mut data);
    data
}

/// Owns a correctly aligned allocation; neither CLI nor a Vec can move its bytes.
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl AlignedBuffer {
    pub fn new(size: usize, alignment: usize) -> io::Result<Self> {
        let layout = Layout::from_size_align(size, alignment)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        if size == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty buffer"));
        }
        // SAFETY: layout has nonzero size and valid power-of-two alignment.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| io::Error::other("aligned buffer allocation failed"))?;
        Ok(Self { ptr, layout })
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: this object owns layout.size() initialized bytes until Drop.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.layout.size()) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: &mut self guarantees exclusive access to the owned allocation.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: ptr was allocated with this exact layout and is freed once.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

/// Removes only the directory it successfully created, never the caller's base.
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    pub fn new(base: &Path, prefix: &str) -> io::Result<Self> {
        if prefix.is_empty()
            || !prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid temporary directory prefix",
            ));
        }
        fs::create_dir_all(base)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = base.join(format!("{prefix}-{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!("Could not clean {}: {error}", self.path.display());
        }
    }
}
