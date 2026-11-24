//! Device queries: DeviceIoControl, the device tree (SetupDi*, CM_*).

use core::ffi::c_void;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_DevNode_Registry_PropertyW, CM_Get_Device_IDW,
    CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW, CM_Get_Parent, DIGCF_DEVICEINTERFACE,
    DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::PCWSTR;

use super::Handle;
use crate::wstr::WString;

pub type Guid = windows::core::GUID;

/// DeviceIoControl with byte buffers, no OVERLAPPED: (it said yes, bytes it returned). The bytes returned are kept
/// when it says no too (a descriptor that didn't fit still fills what it can).
pub fn device_io_control(h: &Handle, code: u32, input: &[u8], output: &mut [u8]) -> (bool, u32) {
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h.0,
            code,
            if input.is_empty() { None } else { Some(input.as_ptr() as *const c_void) },
            input.len() as u32,
            if output.is_empty() { None } else { Some(output.as_mut_ptr() as *mut c_void) },
            output.len() as u32,
            Some(&mut ret),
            None,
        )
    }
    .is_ok();
    (ok, ret)
}

/// One present device interface of a class: the path to open it by, and its device node.
#[derive(Clone, Debug)]
pub struct DeviceInterface {
    pub path: WString,
    pub dev_inst: u32,
}

/// The present interfaces of a device class (SetupDiGetClassDevs + SetupDiEnumDeviceInterfaces), the first `max`
/// of them; one whose details can't be read is left out (as UsbLink.DiskDevInst's loop skips it).
pub fn device_interfaces(class: &Guid, max: u32) -> Vec<DeviceInterface> {
    let mut list = Vec::new();
    let Ok(set) = (unsafe { SetupDiGetClassDevsW(Some(class), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) })
    else {
        return list;
    };
    for i in 0..max {
        let mut ifd = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
        if unsafe { SetupDiEnumDeviceInterfaces(set, None, class, i, &mut ifd) }.is_err() {
            break;
        }
        let mut need = 0u32;
        let _ = unsafe { SetupDiGetDeviceInterfaceDetailW(set, &ifd, None, 0, Some(&mut need), None) };
        if need == 0 {
            continue;
        }
        // SP_DEVICE_INTERFACE_DETAIL_DATA_W: its cbSize is the fixed part (8 in a 64-bit process, 6 in a 32-bit one)
        let mut buf = vec![0u64; (need as usize).div_ceil(8)];
        let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        if unsafe { SetupDiGetDeviceInterfaceDetailW(set, &ifd, Some(detail), need, Some(&mut need), Some(&mut info)) }
            .is_err()
        {
            continue;
        }
        // the path starts at +4, NUL-terminated, inside the buffer
        let units = unsafe { std::slice::from_raw_parts((buf.as_ptr() as *const u8).add(4) as *const u16, (need as usize - 4) / 2) };
        list.push(DeviceInterface { path: WString::from_nul_terminated(units), dev_inst: info.DevInst });
    }
    unsafe {
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    list
}

/// CM_Get_Parent: Ok(parent) or Err(CONFIGRET).
pub fn cm_get_parent(dev_inst: u32) -> Result<u32, u32> {
    let mut parent = 0u32;
    let cr = unsafe { CM_Get_Parent(&mut parent, dev_inst, 0) };
    if cr.0 != 0 { Err(cr.0) } else { Ok(parent) }
}

/// CM_Get_Device_IDW with a 512-character buffer (UsbLink.HubPath).
pub fn cm_get_device_id(dev_inst: u32) -> Option<WString> {
    let mut buf = vec![0u16; 512];
    let cr = unsafe { CM_Get_Device_IDW(dev_inst, &mut buf, 0) };
    if cr.0 != 0 {
        return None;
    }
    Some(WString::from_nul_terminated(&buf))
}

/// The present interfaces of a class on one device, as Windows lists them (NUL-separated, a double NUL at the end).
/// None when it fails or the list is empty (a length of 1 or less).
pub fn cm_device_interface_list(class: &Guid, device_id: &WString) -> Option<Vec<u16>> {
    let id = device_id.to_wide_z();
    let mut len = 0u32;
    let cr = unsafe { CM_Get_Device_Interface_List_SizeW(&mut len, class, PCWSTR(id.as_ptr()), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
    if cr.0 != 0 || len <= 1 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    let cr = unsafe { CM_Get_Device_Interface_ListW(class, PCWSTR(id.as_ptr()), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
    if cr.0 != 0 {
        return None;
    }
    Some(buf)
}

/// CM_Get_DevNode_Registry_PropertyW into a buffer of `capacity` bytes: the bytes it wrote, or None when it failed.
pub fn cm_registry_property(dev_inst: u32, property: u32, capacity: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; capacity];
    let mut len = capacity as u32;
    let mut kind = 0u32;
    let cr = unsafe {
        CM_Get_DevNode_Registry_PropertyW(dev_inst, property, Some(&mut kind), Some(buf.as_mut_ptr() as *mut c_void), &mut len, 0)
    };
    if cr.0 != 0 {
        return None;
    }
    buf.truncate((len as usize).min(capacity));
    Some(buf)
}
