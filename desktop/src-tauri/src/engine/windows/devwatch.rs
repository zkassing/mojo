//! 遥控器在线检测（对照 macos/hid.rs 的 DeviceWatcher 语义）。
//!
//! Windows 侧用轮询代替 IOKit 匹配回调：引擎线程每 3 秒枚举一次 HID 设备接口
//! （SetupAPI），设备路径里同时出现 vid_xxxx / pid_xxxx（不区分大小写）即在线。
//! BLE HID 设备只有在真正连接时才会注册接口，拔掉/休眠即消失，语义与 IOKit
//! 匹配回调等价。

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};

/// GUID_DEVINTERFACE_HID = {4D1E55B2-F16F-11CF-88CB-001111000030}
/// （即 HidD_GetHidGuid() 的返回值；写死免得再链 hid.dll）
const GUID_DEVINTERFACE_HID: GUID = GUID::from_values(
    0x4d1e55b2,
    0xf16f,
    0x11cf,
    [0x88, 0xcb, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
);

/// 目标设备当前是否在线
pub fn remote_online(vendor_id: u32, product_id: u32) -> bool {
    // 设备路径里的 id 是小写十六进制（不区分大小写比较，先转小写再 contains）
    let vid = format!("vid_{vendor_id:04x}");
    let pid = format!("pid_{product_id:04x}");
    // SAFETY: 纯查询调用，无外部可变状态
    unsafe { scan(&vid, &pid) }
}

unsafe fn scan(vid: &str, pid: &str) -> bool {
    // 只枚举当前存在（PRESENT）且带设备接口的 HID 类设备。
    // 指针参数用 .into() 传递，兼容 windows crate 各版本 Option<T>/T 两种签名。
    let Ok(devs) = SetupDiGetClassDevsW(
        Some(&GUID_DEVINTERFACE_HID as *const GUID),
        PCWSTR::null(),
        None,
        DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
    ) else {
        return false;
    };
    let mut found = false;
    let mut index: u32 = 0;
    loop {
        let mut if_data = SP_DEVICE_INTERFACE_DATA::default();
        if_data.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        // 枚举到底（ERROR_NO_MORE_ITEMS）即退出
        if SetupDiEnumDeviceInterfaces(
            devs,
            None,
            &GUID_DEVINTERFACE_HID,
            index,
            &mut if_data,
        )
        .is_err()
        {
            break;
        }
        index += 1;
        if let Some(path) = device_path(devs, &mut if_data) {
            let low = path.to_lowercase();
            if low.contains(vid) && low.contains(pid) {
                found = true;
                break;
            }
        }
    }
    let _ = SetupDiDestroyDeviceInfoList(devs);
    found
}

/// 读设备接口路径（形如 \\?\HID#VID_2717&PID_32B8&...#{...}）
unsafe fn device_path(devs: HDEVINFO, if_data: *mut SP_DEVICE_INTERFACE_DATA) -> Option<String> {
    // 第一段调用：只取所需缓冲区大小（函数按设计失败并写回 required）
    let mut required: u32 = 0;
    let _ = SetupDiGetDeviceInterfaceDetailW(
        devs,
        if_data,
        std::ptr::null_mut::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>().into(),
        0,
        (&mut required as *mut u32).into(),
        std::ptr::null_mut::<SP_DEVINFO_DATA>().into(),
    );
    if required == 0 {
        return None;
    }
    let mut buf = vec![0u8; required as usize];
    let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
    // cbSize 必须是结构体定长部分的大小（x64 = 8），不是缓冲区大小
    (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
    SetupDiGetDeviceInterfaceDetailW(
        devs,
        if_data,
        detail.into(),
        required,
        std::ptr::null_mut::<u32>().into(),
        std::ptr::null_mut::<SP_DEVINFO_DATA>().into(),
    )
    .ok()?;
    // DevicePath 是紧跟定长部分的 NUL 结尾 WCHAR 数组
    Some(pwstr_lossy((*detail).DevicePath.as_ptr()))
}

/// 读 NUL 结尾的 UTF-16 字符串
unsafe fn pwstr_lossy(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}
