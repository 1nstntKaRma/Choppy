//! How a USB drive is connected (UsbLink.cs), asked of the USB hub it is plugged into (as Windows' own USB tools do, no
//! administrator rights needed): the speed it runs at, the most it can do, and whether the port does USB 3. A drive
//! that can do more than it runs at is held back by the port, the cable or a hub on the way.

use super::netfmt;
use super::volume_probe::{StorageDeviceNumber, disk_number};
use crate::sys::{self, Guid};
use crate::wstr::WString;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsbLink {
    pub speed: i32,         // what it runs at now (LOW .. SUPER_PLUS)
    pub capable: i32,       // the most the drive can do (as speed); speed when unknown
    pub capable_known: bool, // Windows 8+ says what the drive can do
    pub port_usb3: bool,    // the port takes USB 3 (5 Gbps or more)
    pub port_known: bool,
    pub uasp: bool,         // UASP: commands queued like a SATA disk's (many small files go faster than Bulk-Only)
    pub bcd_usb: i32,       // the USB version the drive reports right now (0x0210, 0x0320...)
}

pub const LOW: i32 = 0;
pub const FULL: i32 = 1;
pub const HIGH: i32 = 2;
pub const SUPER: i32 = 3;
pub const SUPER_PLUS: i32 = 4;

const GUID_DEVINTERFACE_DISK: Guid = Guid::from_u128(0x53f56307_b6bf_11d0_94f2_00a0c91efb8b);
const GUID_DEVINTERFACE_USB_HUB: Guid = Guid::from_u128(0xf18a0e88_c30c_11d0_8815_00a0c906bed8);
const CM_DRP_SERVICE: u32 = 0x05; // SPDRP_* + 1
const CM_DRP_ADDRESS: u32 = 0x1D;
const IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX: u32 = 0x220448; // FILE_DEVICE_USB, function 274, buffered
const IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2: u32 = 0x22045C; // function 279

impl UsbLink {
    /// The drive could run faster in a faster port (or with another cable).
    pub fn held_back(&self) -> bool {
        self.capable_known && self.capable > self.speed && self.speed >= FULL
    }

    /// "USB 10 Gbps", "USB 5 Gbps", "USB 2.0 (480 Mbps)"
    pub fn speed_text(&self) -> &'static str {
        name(self.speed)
    }

    /// Why it is held back and what helps, in one line; None when it isn't.
    pub fn advice(&self) -> Option<String> {
        if !self.held_back() {
            return None;
        }
        let s = format!("Runs at {} - the drive can do {}. ", short(self.speed), short(self.capable));
        if self.speed <= HIGH {
            return Some(
                s + if self.port_known && self.port_usb3 {
                    "The port does USB 3: the cable, or a hub or extension on the way, is USB 2."
                } else {
                    "The port is USB 2: a USB 3 port (often blue, or marked SS) makes it up to 10 times faster."
                },
            );
        }
        Some(s + "A 10 Gbps port (USB 3.2 Gen 2, often red or marked SS10) and cable make big files up to twice as fast.")
    }

    /// The link of the USB drive that holds this physical disk; None when the disk isn't on USB (or Windows doesn't say).
    pub fn for_disk(disk_number: i32) -> Option<UsbLink> {
        if disk_number < 0 {
            return None;
        }
        // tests: CHOPPY_FAKE_USB=2,3,1 = every USB drive runs at USB 2.0, can do 5 Gbps, its port does USB 3
        if let Some(fake) = std::env::var_os("CHOPPY_FAKE_USB").filter(|v| !v.is_empty()) {
            let fake = fake.to_string_lossy().to_string();
            let f: Vec<&str> = fake.split(',').collect();
            // C# int.Parse throws on a malformed value; a test setting that is wrong here fakes nothing
            let speed = netfmt::parse_int(f[0])?;
            let capable = netfmt::parse_int(f.get(1)?)?;
            return Some(UsbLink {
                speed,
                capable,
                capable_known: true,
                port_known: f.len() > 2,
                port_usb3: f.len() > 2 && f[2] == "1",
                ..Default::default()
            });
        }
        let dev_inst = disk_dev_inst(disk_number);
        if dev_inst == 0 {
            return None;
        }
        // up from the disk to the device that sits on a hub port: disk -> (USBSTOR / UASPStor) -> USB device -> hub
        let mut uasp = false;
        let mut cur = dev_inst;
        for _level in 0..8 {
            if let Some(service) = prop(cur, CM_DRP_SERVICE)
                && service.eq_ignore_case_str("UASPStor") {
                    uasp = true;
                }
            let parent = match sys::cm_get_parent(cur) {
                Ok(p) if p != 0 => p,
                _ => return None,
            };
            if let Some(hub) = hub_path(parent) {
                let port = prop_int(cur, CM_DRP_ADDRESS);
                if port <= 0 {
                    return None;
                }
                let mut link = ask(&hub, port)?;
                link.uasp = uasp;
                return Some(link);
            }
            cur = parent;
        }
        None
    }
}

pub fn name(speed: i32) -> &'static str {
    match speed {
        SUPER_PLUS => "USB 10 Gbps",
        SUPER => "USB 5 Gbps",
        HIGH => "USB 2.0 (480 Mbps)",
        FULL => "USB 1.1 (12 Mbps)",
        _ => "USB 1.0 (1.5 Mbps)",
    }
}

/// Short: "10 Gbps", "5 Gbps", "USB 2.0"
pub fn short(speed: i32) -> &'static str {
    match speed {
        SUPER_PLUS => "10 Gbps",
        SUPER => "5 Gbps",
        HIGH => "USB 2.0",
        FULL => "USB 1.1",
        _ => "USB 1.0",
    }
}

/// About the most a drive moves over this link, in MB/s (1,048,576 bytes): what is left of the line rate after
/// USB's own encoding and protocol. USB 2.0 drives measure 35-42, 5 Gbps ones 400-450, 10 Gbps ones 900-1,050.
pub fn ceiling_mbps(speed: i32) -> f64 {
    match speed {
        SUPER_PLUS => 1000.0,
        SUPER => 430.0,
        HIGH => 40.0,
        FULL => 1.0,
        _ => 0.15,
    }
}

/// The names people know the speeds by (a tooltip).
pub fn aka(speed: i32) -> &'static str {
    match speed {
        SUPER_PLUS => "USB 3.2 Gen 2 (also called USB 3.1 Gen 2), SuperSpeed+; 20 Gbps (Gen 2x2) reports the same",
        SUPER => "USB 3.2 Gen 1 (also called USB 3.0 or USB 3.1 Gen 1), SuperSpeed",
        HIGH => "USB 2.0 High Speed",
        _ => "USB 1.x",
    }
}

/// The hub's answer about one of its ports.
fn ask(hub_path: &WString, port: i32) -> Option<UsbLink> {
    let h = sys::create_file(hub_path, sys::GENERIC_WRITE, sys::FILE_SHARE_WRITE, sys::OPEN_EXISTING, 0).ok()?;
    // USB_NODE_CONNECTION_INFORMATION_EX: ConnectionIndex, the device descriptor (bcdUSB at +2), ..., Speed at 23
    let mut in_ex = [0u8; 35];
    in_ex[..4].copy_from_slice(&port.to_le_bytes());
    let mut out_ex = [0u8; 1024];
    let (ok, ret) = sys::device_io_control(&h, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX, &in_ex, &mut out_ex);
    if !ok || ret < 35 {
        return None;
    }
    if i32::from_le_bytes(out_ex[31..35].try_into().unwrap()) != 1 {
        /*DeviceConnected*/
        return None;
    }
    let mut link = UsbLink {
        bcd_usb: u16::from_le_bytes([out_ex[6], out_ex[7]]) as i32,
        speed: (out_ex[23] as i32).min(SUPER),
        ..Default::default()
    };
    link.capable = link.speed;
    // USB_NODE_CONNECTION_INFORMATION_EX_V2 (Windows 8+): which USB versions the port takes, and what the drive
    // can do and runs at (SuperSpeed or more, SuperSpeedPlus or more)
    let mut in_v2 = [0u8; 16];
    in_v2[..4].copy_from_slice(&port.to_le_bytes());
    in_v2[4..8].copy_from_slice(&16i32.to_le_bytes());
    in_v2[8..12].copy_from_slice(&7i32.to_le_bytes()); // the protocols we understand: USB 1.1, 2.0, 3.x
    let mut out_v2 = [0u8; 16];
    let (ok, ret) = sys::device_io_control(&h, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2, &in_v2, &mut out_v2);
    if ok && ret >= 16 {
        let protocols = u32::from_le_bytes(out_v2[8..12].try_into().unwrap());
        let flags = u32::from_le_bytes(out_v2[12..16].try_into().unwrap());
        link.port_known = protocols != 0;
        link.port_usb3 = (protocols & 4) != 0;
        if (flags & 4) != 0 {
            link.speed = SUPER_PLUS;
        } else if (flags & 1) != 0 {
            link.speed = link.speed.max(SUPER);
        }
        link.capable = if (flags & 8) != 0 { SUPER_PLUS } else if (flags & 2) != 0 { SUPER } else { link.speed };
        link.capable = link.capable.max(link.speed);
        link.capable_known = true;
    }
    Some(link)
}

/// The device node of the disk that answers with this number.
fn disk_dev_inst(disk: i32) -> u32 {
    for di in sys::device_interfaces(&GUID_DEVINTERFACE_DISK, 64) {
        let Ok(h) = sys::create_file(&di.path, 0, sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE, sys::OPEN_EXISTING, 0) else {
            continue;
        };
        if let Some(dn) = StorageDeviceNumber::query(&h)
            && disk_number(dn) == disk {
                return di.dev_inst;
            }
    }
    0
}

/// The path to open a hub by, when this device node is a USB hub; else None.
fn hub_path(dev_inst: u32) -> Option<WString> {
    let id = sys::cm_get_device_id(dev_inst)?;
    let buf = sys::cm_device_interface_list(&GUID_DEVINTERFACE_USB_HUB, &id)?;
    let first = WString::from_nul_terminated(&buf);
    if first.is_empty() { None } else { Some(first) }
}

fn prop(dev_inst: u32, property: u32) -> Option<WString> {
    let b = sys::cm_registry_property(dev_inst, property, 1024)?;
    if b.len() < 2 {
        return None;
    }
    let units: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
    Some(WString::from_units(units).trim_end_char('\0'))
}

fn prop_int(dev_inst: u32, property: u32) -> i32 {
    match sys::cm_registry_property(dev_inst, property, 4) {
        Some(b) if b.len() == 4 => i32::from_le_bytes(b[..4].try_into().unwrap()),
        _ => -1,
    }
}
