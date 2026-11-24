//! Volumes and drives.

use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
    GetVolumePathNameW,
};
use windows::core::PCWSTR;

use crate::wstr::WString;

/// GetVolumePathNameW with a 1,024-character buffer (as VolumeProbe's StringBuilder(1024)).
pub fn get_volume_path_name(path: &WString) -> Option<WString> {
    let p = path.to_wide_z();
    let mut buf = vec![0u16; 1024];
    unsafe { GetVolumePathNameW(PCWSTR(p.as_ptr()), &mut buf) }.ok()?;
    Some(WString::from_nul_terminated(&buf))
}

/// What GetVolumeInformationW says about a volume.
#[derive(Clone, Debug, Default)]
pub struct VolumeInformation {
    pub serial: u32,
    pub max_component: u32,
    pub flags: u32,
    pub file_system: WString,
    pub label: WString,
}

/// GetVolumeInformationW (file system name in 64 characters; the label in 261 when asked for, else not asked).
pub fn get_volume_information(root: &WString, with_label: bool) -> Option<VolumeInformation> {
    let p = root.to_wide_z();
    let mut fs = vec![0u16; 64];
    let mut label = vec![0u16; 261];
    let (mut serial, mut max_component, mut flags) = (0u32, 0u32, 0u32);
    unsafe {
        GetVolumeInformationW(
            PCWSTR(p.as_ptr()),
            if with_label { Some(&mut label) } else { None },
            Some(&mut serial),
            Some(&mut max_component),
            Some(&mut flags),
            Some(&mut fs),
        )
    }
    .ok()?;
    Some(VolumeInformation {
        serial,
        max_component,
        flags,
        file_system: WString::from_nul_terminated(&fs),
        label: if with_label { WString::from_nul_terminated(&label) } else { WString::new() },
    })
}

pub fn get_drive_type(root: &WString) -> u32 {
    let p = root.to_wide_z();
    unsafe { GetDriveTypeW(PCWSTR(p.as_ptr())) }
}

/// GetDiskFreeSpaceW: (sectors per cluster, bytes per sector, free clusters, total clusters).
pub fn get_disk_free_space(root: &WString) -> Option<(u32, u32, u32, u32)> {
    let p = root.to_wide_z();
    let (mut spc, mut bps, mut fc, mut tc) = (0u32, 0u32, 0u32, 0u32);
    unsafe { GetDiskFreeSpaceW(PCWSTR(p.as_ptr()), Some(&mut spc), Some(&mut bps), Some(&mut fc), Some(&mut tc)) }.ok()?;
    Some((spc, bps, fc, tc))
}

/// GetVolumeNameForVolumeMountPointW with a 100-character buffer ("\\?\Volume{...}\").
pub fn get_volume_name_for_mount_point(root: &WString) -> Option<WString> {
    let p = root.to_wide_z();
    let mut buf = vec![0u16; 100];
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(p.as_ptr()), &mut buf) }.ok()?;
    Some(WString::from_nul_terminated(&buf))
}

/// The drive letters in use, one bit each (A = bit 0): what DriveInfo.GetDrives lists.
pub fn get_logical_drives() -> u32 {
    unsafe { GetLogicalDrives() }
}
