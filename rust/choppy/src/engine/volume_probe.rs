//! What sits behind a path: which volume, what kind of device, and (for network) how far away (VolumeProbe.cs).

use std::time::Instant;

use super::paths;
use super::usb_link::UsbLink;
use crate::sys;
use crate::wstr::{WString, w};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeviceKind {
    #[default]
    Unknown,
    Ssd,
    Hdd,
    Usb,
    Network,
    Optical,
    Ram,
}

impl DeviceKind {
    /// DeviceKind.ToString()
    pub fn name(self) -> &'static str {
        match self {
            DeviceKind::Unknown => "Unknown",
            DeviceKind::Ssd => "Ssd",
            DeviceKind::Hdd => "Hdd",
            DeviceKind::Usb => "Usb",
            DeviceKind::Network => "Network",
            DeviceKind::Optical => "Optical",
            DeviceKind::Ram => "Ram",
        }
    }
}

/// What sits behind a path: which volume, what kind of device, and (for network) how far away.
#[derive(Clone, Debug)]
pub struct VolumeInfo {
    pub root: WString,              // "C:\", "\\server\share\"
    pub id: WString,                // used for "same volume" checks
    pub kind: DeviceKind,
    pub file_system: Option<String>,
    pub label: Option<String>,      // the drive's name ("Media"); empty when it has none
    pub rtt_ms: f64,                // network round trip of a metadata call (min of 3)
    pub disk: i32,                  // physical disk number (-1 = unknown / network)
    pub sector: i32,                // bytes per sector: unbuffered reads and writes go in multiples of it
    pub removable: bool,            // can be pulled out (USB, cards): nothing may wait in memory for it
    pub fs_flags: u32,              // the file system's FILE_* capability flags
    pub on_usb: bool,               // attached by USB (an SSD in a USB case too)
    pub usb: Option<UsbLink>,       // a USB drive: the speed it runs at and the most it can do (None = not USB, or unknown)
}

impl Default for VolumeInfo {
    fn default() -> Self {
        VolumeInfo {
            root: WString::new(),
            id: WString::new(),
            kind: DeviceKind::Unknown,
            file_system: None,
            label: None,
            rtt_ms: -1.0,
            disk: -1,
            sector: 512,
            removable: false,
            fs_flags: 0,
            on_usb: false,
            usb: None,
        }
    }
}

impl VolumeInfo {
    pub fn is_network(&self) -> bool {
        self.kind == DeviceKind::Network
    }
    pub fn is_slow_seek(&self) -> bool {
        matches!(self.kind, DeviceKind::Hdd | DeviceKind::Usb | DeviceKind::Optical)
    }

    /// The largest file this file system holds (FAT32: 4 GB less a byte); 0 = no limit that matters.
    pub fn max_file_size(&self) -> i64 {
        match &self.file_system {
            Some(fs) if WString::from(fs.as_str()).starts_with_ignore_case_str("FAT") => u32::MAX as i64,
            _ => 0,
        }
    }

    /// ReFS (a Dev Drive, for one): a copy on the same volume can share the original's blocks.
    pub fn can_clone(&self) -> bool {
        (self.fs_flags & sys::FILE_SUPPORTS_BLOCK_REFCOUNTING) != 0
    }

    pub fn kind_text(&self) -> &'static str {
        match self.kind {
            DeviceKind::Ssd => "SSD",
            DeviceKind::Hdd => "HDD",
            DeviceKind::Usb => "USB",
            DeviceKind::Network => "Network",
            DeviceKind::Optical => "Disc",
            DeviceKind::Ram => "RAM disk",
            DeviceKind::Unknown => "Disk",
        }
    }
}

/// STORAGE_DEVICE_NUMBER: the device type, its number, the partition.
#[derive(Clone, Copy, Debug, Default)]
pub struct StorageDeviceNumber {
    pub device_type: i32,
    pub device_number: i32,
}

impl StorageDeviceNumber {
    /// IOCTL_STORAGE_GET_DEVICE_NUMBER on an open volume or disk.
    pub fn query(h: &sys::Handle) -> Option<StorageDeviceNumber> {
        let mut out = [0u8; 12];
        let (ok, _) = sys::device_io_control(h, sys::IOCTL_STORAGE_GET_DEVICE_NUMBER, &[], &mut out);
        let int = |o: usize| i32::from_le_bytes(out[o..o + 4].try_into().unwrap());
        ok.then(|| StorageDeviceNumber { device_type: int(0), device_number: int(4) })
    }
}

pub const FILE_DEVICE_DISK: i32 = 7;

pub fn probe(long_path: &WString) -> VolumeInfo {
    let mut v = VolumeInfo::default();
    let short_path = paths::to_short(long_path);
    let mut root: Option<WString> = None;
    // GetVolumePathName wants an existing-ish path; walk up until it works.
    let mut probe = Some(short_path.clone());
    while let Some(p) = probe {
        if let Some(r) = sys::get_volume_path_name(&p) {
            root = Some(r);
            break;
        }
        probe = paths::parent(&p);
    }
    let mut root = root.unwrap_or_else(|| {
        if short_path.starts_with(r"\\") {
            unc_root(&short_path)
        } else if short_path.len() >= 3 {
            short_path.slice(0, 3)
        } else {
            short_path.clone()
        }
    });
    if !root.ends_with("\\") {
        root += "\\";
    }
    v.root = root.clone();

    let drive_type = sys::get_drive_type(&root);
    let unc = root.starts_with(r"\\");
    if unc || drive_type == sys::DRIVE_REMOTE {
        v.kind = DeviceKind::Network;
        v.id = w("net:") + &(if unc { root.to_lower_invariant() } else { root.to_upper_invariant() });
        v.rtt_ms = measure_rtt(long_path);
    } else {
        if let Some(vi) = sys::get_volume_information(&root, false) {
            v.id = w(&format!("vol:{:08X}", vi.serial));
            v.file_system = Some(vi.file_system.to_string_lossy());
            v.fs_flags = vi.flags;
        } else {
            v.id = w("root:") + &root.to_upper_invariant();
        }

        if drive_type == sys::DRIVE_CDROM {
            v.kind = DeviceKind::Optical;
        } else if drive_type == sys::DRIVE_RAMDISK {
            v.kind = DeviceKind::Ram;
        } else {
            let (kind, disk, removable, usb) = probe_disk(&root, drive_type);
            v.kind = kind;
            v.disk = disk;
            v.removable = removable;
            v.on_usb = usb;
            if usb {
                v.usb = UsbLink::for_disk(v.disk);
            }
        }
        if let Some((_, bps, _, _)) = sys::get_disk_free_space(&root)
            && bps >= 512 {
                v.sector = bps as i32;
            }
    }
    // the name the drive shows in Explorer ("Media", "NAS")
    if let Some(vi) = sys::get_volume_information(&root, true) {
        if v.file_system.is_none() {
            v.file_system = Some(vi.file_system.to_string_lossy());
        }
        v.label = Some(vi.label.to_string_lossy());
    }
    v
}

fn unc_root(unc: &WString) -> WString {
    // \\server\share\...
    let Some(a) = unc.index_of('\\', 2) else { return unc.clone() + "\\" };
    match unc.index_of('\\', a + 1) {
        None => unc.clone() + "\\",
        Some(b) => unc.slice(0, b + 1),
    }
}

/// True when both sit on the same physical disk (reads and writes then share one set of heads).
pub fn same_disk(a: Option<&VolumeInfo>, b: Option<&VolumeInfo>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => !a.is_network() && !b.is_network() && (a.id == b.id || (a.disk >= 0 && a.disk == b.disk)),
        _ => false,
    }
}

/// (kind, disk number, removable, attached by USB)
fn probe_disk(root: &WString, drive_type: u32) -> (DeviceKind, i32, bool, bool) {
    let mut disk = -1;
    let mut usb = false;
    let mut removable = drive_type == sys::DRIVE_REMOVABLE;
    let fallback = if drive_type == sys::DRIVE_REMOVABLE { DeviceKind::Usb } else { DeviceKind::Unknown };
    // Open the volume device itself (no access rights needed for these queries).
    let dev = if root.len() == 3 && root.at(1) == b':' as u16 {
        Some(w(r"\\.\") + &root.slice(0, 2))
    } else {
        sys::get_volume_name_for_mount_point(root).map(|vn| vn.trim_end_char('\\'))
    };
    let Some(dev) = dev else { return (fallback, disk, removable, usb) };

    let Ok(h) = sys::create_file(&dev, 0, sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE, sys::OPEN_EXISTING, 0) else {
        return (fallback, disk, removable, usb);
    };
    if let Some(dn) = StorageDeviceNumber::query(&h) {
        disk = disk_number(dn);
    }

    // STORAGE_PROPERTY_QUERY { PropertyId, QueryType, AdditionalParameters } (12 bytes); 0 = StorageDeviceProperty
    let query = |id: u32| {
        let mut q = [0u8; 12];
        q[..4].copy_from_slice(&id.to_le_bytes());
        q
    };
    // the first 36 bytes of STORAGE_DEVICE_DESCRIPTOR: BusType at +28
    let mut dd = [0u8; 36];
    let (ok, ret) = sys::device_io_control(&h, sys::IOCTL_STORAGE_QUERY_PROPERTY, &query(0), &mut dd);
    let bus_type = i32::from_le_bytes(dd[28..32].try_into().unwrap());
    // DEVICE_SEEK_PENALTY_DESCRIPTOR { Version, Size, IncursSeekPenalty } (12 bytes); 7 = StorageDeviceSeekPenaltyProperty
    let mut sp = [0u8; 12];
    let (knows_seek, _) = sys::device_io_control(&h, sys::IOCTL_STORAGE_QUERY_PROPERTY, &query(7), &mut sp);
    let incurs_seek_penalty = sp[8] != 0;
    let answered = ok || ret >= 32;
    usb = answered && bus_type == 7;
    if (answered && (bus_type == 7 /*USB*/ || bus_type == 12 /*SD*/ || bus_type == 13 /*MMC*/)) || drive_type == sys::DRIVE_REMOVABLE {
        // pluggable: a USB SSD that says it has no seek delay is copied like an SSD (still removable: nothing
        // waits in memory for it); USB hard disks, sticks and cards stay one steady stream
        removable = true;
        let kind = if knows_seek && !incurs_seek_penalty && bus_type == 7 { DeviceKind::Ssd } else { DeviceKind::Usb };
        return (kind, disk, removable, usb);
    }
    if knows_seek {
        return (if incurs_seek_penalty { DeviceKind::Hdd } else { DeviceKind::Ssd }, disk, removable, usb);
    }
    if ok && bus_type == 17 {
        /*NVMe*/
        return (DeviceKind::Ssd, disk, removable, usb);
    }
    (DeviceKind::Unknown, disk, removable, usb)
}

/// The physical disk a volume sits on, or -1. Only a disk device's number counts: numbers of other device types
/// are counted separately - Google Drive's G: answers as a file system device "number 0", the same number as a
/// real hard disk, which made a copy between the two look like one disk.
pub fn disk_number(dn: StorageDeviceNumber) -> i32 {
    if dn.device_type == FILE_DEVICE_DISK && dn.device_number >= 0 { dn.device_number } else { -1 }
}

/// Round trip of one metadata call to the share (best of 3). Tailscale to the NAS was ~9 ms.
pub fn measure_rtt(long_path: &WString) -> f64 {
    let mut p = Some(long_path.clone());
    while let Some(x) = &p {
        if paths::exists(x) {
            break;
        }
        p = paths::parent(x);
    }
    let Some(p) = p else { return -1.0 };
    let mut best = f64::MAX;
    for _ in 0..3 {
        let name = w(&format!("~choppy-rtt-{}", &super::model::new_id()[..8]));
        let sw = Instant::now();
        // a name that never exists forces a real server round trip (no cached attributes)
        let _ = sys::get_file_attributes_ex(&paths::combine(&p, &name));
        best = best.min(sw.elapsed().as_secs_f64() * 1000.0);
    }
    best
}
