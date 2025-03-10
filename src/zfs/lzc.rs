use crate::{zfs::{BookmarkRequest, Checksum, Compression, Copies, CreateDatasetRequest,
                  DatasetKind, DestroyTiming, Error, Result, SendFlags, SnapDir, ValidationError,
                  ZfsEngine},
            GlobalLogger};
use cstr_argument::CStrArgument;
use libnv::nvpair::NvList;
use slog::Logger;

use crate::zfs::{errors::Error::ValidationErrors,
                 properties::{AclInheritMode, AclMode, ZfsProp},
                 PathExt};
use std::{collections::HashMap,
          ffi::CString,
          os::unix::io::{AsRawFd, RawFd},
          path::PathBuf,
          ptr::null_mut};
use zfs_core_sys as sys;

#[cfg(target_os = "freebsd")]
const ECHRNG: libc::c_int = libc::ENXIO;
#[cfg(target_os = "linux")]
const ECHRNG: libc::c_int = libc::ECHRNG;

#[derive(Debug, Clone)]
pub struct ZfsLzc {
    logger: Logger,
}

impl ZfsLzc {
    /// Initialize libzfs_core backed ZfsEngine.
    /// If root logger is None, then StdLog drain used.
    pub fn new() -> Result<Self> {
        let errno = unsafe { sys::libzfs_core_init() };

        if errno != 0 {
            let io_error = std::io::Error::from_raw_os_error(errno);
            return Err(Error::LZCInitializationFailed(io_error));
        }
        let logger = GlobalLogger::get().new(o!("zetta_module" => "zfs", "zfs_impl" => "lzc"));

        Ok(ZfsLzc { logger })
    }

    pub fn logger(&self) -> &Logger { &self.logger }

    fn send(
        &self,
        path: PathBuf,
        from: Option<PathBuf>,
        fd: RawFd,
        flags: SendFlags,
    ) -> Result<()> {
        let snapshot =
            CString::new(path.to_str().unwrap()).expect("Failed to create CString from path");
        let snapshot_ptr = snapshot.as_ptr();
        let from_cstr = from.map(|f| {
            CString::new(f.to_str().unwrap()).expect("Failed to create CString from path")
        });
        let fd_raw = fd;
        let errno = if let Some(src) = from_cstr {
            unsafe { zfs_core_sys::lzc_send(snapshot_ptr, src.as_ptr(), fd_raw, flags.bits) }
        } else {
            unsafe { zfs_core_sys::lzc_send(snapshot_ptr, std::ptr::null(), fd_raw, flags.bits) }
        };

        match errno {
            0 => Ok(()),
            _ => {
                let io_error = std::io::Error::from_raw_os_error(errno);
                Err(Error::Io(io_error))
            },
        }
    }
}

impl ZfsEngine for ZfsLzc {
    fn exists<N: Into<PathBuf>>(&self, name: N) -> Result<bool> {
        let path = name.into();
        let n = path.to_str().expect("Invalid Path").into_cstr();
        let ret = unsafe { sys::lzc_exists(n.as_ref().as_ptr()) };

        if ret == 1 {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn create(&self, request: CreateDatasetRequest) -> Result<()> {
        request.validate()?;

        //let mut props = nvpair::NvList::new()?;
        let mut props = NvList::default();
        let name_c_string =
            CString::new(request.name().to_str().expect("Non UTF-8 name")).expect("NULL in name");
        // LZC wants _everything_ as u64 even booleans.
        if let Some(acl_inherit) = request.acl_inherit {
            props.insert_u64(AclInheritMode::nv_key(), acl_inherit.as_nv_value())?;
        }
        if let Some(acl_mode) = request.acl_mode {
            props.insert_u64(AclMode::nv_key(), acl_mode.as_nv_value())?;
        }
        if let Some(atime) = request.atime {
            props.insert_u64("atime", bool_to_u64(atime))?;
        }
        if let Some(checksum) = request.checksum {
            props.insert_u64(Checksum::nv_key(), checksum.as_nv_value())?;
        }
        if let Some(compression) = request.compression {
            props.insert_u64(Compression::nv_key(), compression.as_nv_value())?;
        }
        if let Some(copies) = request.copies() {
            props.insert_u64(Copies::nv_key(), copies.as_nv_value())?;
        }
        if let Some(devices) = request.devices {
            props.insert_u64("devices", bool_to_u64(devices))?;
        }
        if let Some(exec) = request.exec {
            props.insert_u64("exec", bool_to_u64(exec))?;
        }
        // saved fore mount point
        if let Some(primary_cache) = request.primary_cache {
            props.insert_u64("primarycache", primary_cache.as_nv_value())?;
        }
        if let Some(quota) = request.quota {
            props.insert_u64("quota", quota)?;
        }
        if let Some(readonly) = request.readonly {
            props.insert_u64("readonly", bool_to_u64(readonly))?;
        }
        if let Some(record_size) = request.record_size {
            props.insert_u64("recordsize", record_size)?;
        }
        if let Some(ref_quota) = request.ref_quota {
            props.insert_u64("refquota", ref_quota)?;
        }
        if let Some(ref_reservation) = request.ref_reservation {
            props.insert_u64("refreservation", ref_reservation)?;
        }
        if let Some(secondary_cache) = request.secondary_cache {
            props.insert_u64("secondarycache", secondary_cache.as_nv_value())?;
        }
        if let Some(setuid) = request.setuid {
            props.insert_u64("setuid", bool_to_u64(setuid))?;
        }
        if let Some(snap_dir) = request.snap_dir {
            props.insert_u64(SnapDir::nv_key(), snap_dir.as_nv_value())?;
        }

        if request.kind == DatasetKind::Filesystem
            && (request.volume_size.is_some() || request.volume_block_size.is_some())
        {
            return Err(Error::invalid_input());
        }

        if request.kind == DatasetKind::Volume && request.volume_size.is_none() {
            return Err(Error::invalid_input());
        }

        if let Some(vol_size) = request.volume_size {
            props.insert_u64("volsize", vol_size)?;
        }
        if let Some(vol_block_size) = request.volume_block_size {
            props.insert_u64("volblocksize", vol_block_size)?;
        }

        if let Some(xattr) = request.xattr {
            props.insert("xattr", bool_to_u64(xattr))?;
        }
        if let Some(user_props) = request.user_properties() {
            for (key, value) in user_props {
                props.insert_string(key, value)?;
            }
        }
        let errno = unsafe {
            zfs_core_sys::lzc_create(
                name_c_string.as_ref().as_ptr(),
                request.kind().as_c_uint(),
                props.as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };

        match errno {
            0 => Ok(()),
            _ => {
                let io_error = std::io::Error::from_raw_os_error(errno);
                Err(Error::Io(io_error))
            },
        }
    }

    fn snapshot(
        &self,
        snapshots: &[PathBuf],
        user_properties: Option<HashMap<String, String>>,
    ) -> Result<()> {
        let validation_errors: Vec<ValidationError> =
            snapshots.iter().map(PathBuf::validate).filter_map(Result::err).collect();
        if !validation_errors.is_empty() {
            return Err(ValidationErrors(validation_errors));
        }

        let mut snapshots_list = NvList::default();
        let mut props = NvList::default();
        for snap in snapshots {
            snapshots_list.insert(&snap.to_string_lossy(), true)?;
        }
        let mut errors_list_ptr = null_mut();
        if let Some(user_properties) = user_properties {
            for (key, value) in user_properties {
                props.insert_string(&key, &value)?;
            }
        }
        let errno = unsafe {
            zfs_core_sys::lzc_snapshot(
                snapshots_list.as_ptr(),
                props.as_ptr(),
                &mut errors_list_ptr,
            )
        };
        if !errors_list_ptr.is_null() {
            let errors = unsafe { NvList::from_ptr(errors_list_ptr) };
            if !errors.is_empty() {
                return Err(Error::from(errors.into_hashmap()));
            }
        }
        match errno {
            0 => Ok(()),
            _ => {
                let io_error = std::io::Error::from_raw_os_error(errno);
                Err(Error::Io(io_error))
            },
        }
    }

    fn bookmark(&self, bookmarks: &[BookmarkRequest]) -> Result<()> {
        let validation_errors: Vec<ValidationError> = bookmarks
            .iter()
            .flat_map(|BookmarkRequest { snapshot, bookmark }| vec![snapshot, bookmark])
            .map(PathBuf::validate)
            .filter_map(Result::err)
            .collect();
        if !validation_errors.is_empty() {
            return Err(ValidationErrors(validation_errors));
        }

        let mut bookmarks_list = NvList::default();
        for BookmarkRequest { snapshot, bookmark } in bookmarks {
            bookmarks_list
                .insert(&bookmark.to_string_lossy(), snapshot.to_string_lossy().as_ref())?;
        }

        let mut errors_list_ptr = null_mut();
        let errno =
            unsafe { zfs_core_sys::lzc_bookmark(bookmarks_list.as_ptr(), &mut errors_list_ptr) };
        if !errors_list_ptr.is_null() {
            let errors = unsafe { NvList::from_ptr(errors_list_ptr) };
            if !errors.is_empty() {
                return Err(Error::from(errors.into_hashmap()));
            }
        }
        match errno {
            0 => Ok(()),
            _ => {
                let io_error = std::io::Error::from_raw_os_error(errno);
                Err(Error::Io(io_error))
            },
        }
    }

    fn destroy_snapshots(&self, snapshots: &[PathBuf], timing: DestroyTiming) -> Result<()> {
        let validation_errors: Vec<ValidationError> = snapshots
            .iter()
            .map(PathBuf::validate)
            .filter(Result::is_err)
            .map(Result::unwrap_err)
            .collect();
        if !validation_errors.is_empty() {
            return Err(ValidationErrors(validation_errors));
        }

        let mut snapshots_list = NvList::default();

        for snap in snapshots {
            snapshots_list.insert(&snap.to_string_lossy(), true)?;
        }

        let mut errors_list_ptr = null_mut();
        let errno = unsafe {
            zfs_core_sys::lzc_destroy_snaps(
                snapshots_list.as_ptr(),
                timing.as_c_uint(),
                &mut errors_list_ptr,
            )
        };
        if !errors_list_ptr.is_null() {
            let errors = unsafe { NvList::from_ptr(errors_list_ptr) };
            if !errors.is_empty() {
                return Err(Error::from(errors.into_hashmap()));
            }
        }
        match errno {
            0 => Ok(()),
            _ => {
                let io_error = std::io::Error::from_raw_os_error(errno);
                Err(Error::Io(io_error))
            },
        }
    }

    fn destroy_bookmarks(&self, bookmarks: &[PathBuf]) -> Result<()> {
        let validation_errors: Vec<ValidationError> = bookmarks
            .iter()
            .map(PathBuf::validate)
            .filter(Result::is_err)
            .map(Result::unwrap_err)
            .collect();
        if !validation_errors.is_empty() {
            return Err(ValidationErrors(validation_errors));
        }

        let mut bookmarks_list = NvList::default();

        for bookmark in bookmarks {
            bookmarks_list.insert_boolean(&bookmark.to_string_lossy())?;
        }

        let mut errors_list_ptr = null_mut();
        let errno = unsafe {
            zfs_core_sys::lzc_destroy_bookmarks(bookmarks_list.as_ptr(), &mut errors_list_ptr)
        };
        if !errors_list_ptr.is_null() {
            let errors = unsafe { NvList::from_ptr(errors_list_ptr) };
            if !errors.is_empty() {
                return Err(Error::from(errors.into_hashmap()));
            }
        }
        match errno {
            0 => Ok(()),
            _ => {
                let io_error = std::io::Error::from_raw_os_error(errno);
                Err(Error::Io(io_error))
            },
        }
    }

    fn send_full<N: Into<PathBuf>, FD: AsRawFd>(
        &self,
        path: N,
        fd: FD,
        flags: SendFlags,
    ) -> Result<()> {
        self.send(path.into(), None, fd.as_raw_fd(), flags)
    }

    fn send_incremental<N: Into<PathBuf>, F: Into<PathBuf>, FD: AsRawFd>(
        &self,
        path: N,
        from: F,
        fd: FD,
        flags: SendFlags,
    ) -> Result<()> {
        self.send(path.into(), Some(from.into()), fd.as_raw_fd(), flags)
    }

    fn run_channel_program<N: Into<PathBuf>>(
        &self,
        pool: N,
        program: &str,
        instr_limit: u64,
        mem_limit: u64,
        sync: bool,
        args: NvList,
    ) -> Result<NvList> {
        let pool = pool.into();
        let pool_c_string = pool.to_str().expect("Non UTF-8 pool name").into_cstr();
        let prog_c_string = program.into_cstr();

        let mut out_nvlist_ptr = null_mut();
        let errno = unsafe {
            if sync {
                zfs_core_sys::lzc_channel_program(
                    pool_c_string.as_ref().as_ptr(),
                    prog_c_string.as_ref().as_ptr(),
                    instr_limit,
                    mem_limit,
                    args.as_ptr(),
                    &mut out_nvlist_ptr,
                )
            } else {
                zfs_core_sys::lzc_channel_program_nosync(
                    pool_c_string.as_ref().as_ptr(),
                    prog_c_string.as_ref().as_ptr(),
                    instr_limit,
                    mem_limit,
                    args.as_ptr(),
                    &mut out_nvlist_ptr,
                )
            }
        };
        match errno {
            0 => Ok(unsafe { NvList::from_ptr(out_nvlist_ptr) }),
            libc::EINVAL => Err(Error::ChanProgInval(
                unsafe { NvList::from_ptr(out_nvlist_ptr) }.into_hashmap(),
            )),
            ECHRNG => Err(Error::ChanProgRuntime(
                unsafe { NvList::from_ptr(out_nvlist_ptr) }.into_hashmap(),
            )),
            _ => {
                let io_error = std::io::Error::from_raw_os_error(errno);
                Err(Error::Io(io_error))
            },
        }
    }
}

// This should be mapped to values from nvpair.
fn bool_to_u64(src: bool) -> u64 {
    if src {
        1
    } else {
        0
    }
}
