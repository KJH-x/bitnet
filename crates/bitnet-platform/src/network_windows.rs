use super::*;
pub(super) fn polling_only(_: &ChangeGuard) -> bool {
    false
}
use crate::{error, windows::wide_string};
use bitnet_core::{DnsConfiguration, DnsMode, ErrorCode, Ipv4Assignment};
use std::{
    ffi::{c_void, CStr},
    mem::size_of,
    net::{IpAddr, Ipv6Addr},
    ptr,
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    NetworkManagement::{IpHelper::*, Ndis::*},
    Networking::WinSock::*,
};

unsafe fn ipv4(address: *const SOCKADDR) -> Option<Ipv4Addr> {
    if address.is_null() || (*address).sa_family != AF_INET {
        return None;
    }
    Some(Ipv4Addr::from(
        (*address.cast::<SOCKADDR_IN>())
            .sin_addr
            .S_un
            .S_addr
            .to_ne_bytes(),
    ))
}

unsafe fn socket_ip(address: SOCKET_ADDRESS) -> Option<IpAddr> {
    let ptr = address.lpSockaddr;
    let len = usize::try_from(address.iSockaddrLength).ok()?;
    if ptr.is_null() || len < size_of::<ADDRESS_FAMILY>() {
        return None;
    }
    match (*ptr).sa_family {
        AF_INET if len >= size_of::<SOCKADDR_IN>() => ipv4(ptr).map(IpAddr::V4),
        AF_INET6 if len >= size_of::<SOCKADDR_IN6>() => {
            let raw = (*ptr.cast::<SOCKADDR_IN6>()).sin6_addr.u.Byte;
            Some(IpAddr::V6(Ipv6Addr::from(raw)))
        }
        _ => None,
    }
}

fn dns_mode(id: &str, ipv6: bool) -> Option<DnsMode> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE,
            RRF_RT_REG_SZ,
        },
    };
    if id.len() != 38 || !id.starts_with('{') || !id.ends_with('}') {
        return None;
    }
    let service = if ipv6 { "Tcpip6" } else { "Tcpip" };
    let path = crate::windows::wide(&format!(
        r"SYSTEM\CurrentControlSet\Services\{service}\Parameters\Interfaces\{id}"
    ));
    let mut key: HKEY = ptr::null_mut();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if opened != ERROR_SUCCESS {
        return None;
    }
    let name = crate::windows::wide("NameServer");
    let mut value = [0_u16; 1024];
    let mut bytes = size_of::<[u16; 1024]>() as u32;
    let result = unsafe {
        let result = RegGetValueW(
            key,
            ptr::null(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut bytes,
        );
        RegCloseKey(key);
        result
    };
    match result {
        ERROR_FILE_NOT_FOUND => Some(DnsMode::Automatic),
        ERROR_SUCCESS => {
            let value = String::from_utf16_lossy(&value[..(bytes as usize / 2).min(value.len())]);
            static_dns_mode(value.trim_matches(char::from(0)).trim(), ipv6)
        }
        _ => None,
    }
}

fn static_dns_mode(value: &str, ipv6: bool) -> Option<DnsMode> {
    if value.is_empty() {
        return Some(DnsMode::Automatic);
    }
    let mut servers = value
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .peekable();
    (servers.peek().is_some()
        && servers.all(|part| {
            part.split('%')
                .next()
                .and_then(|ip| ip.parse::<IpAddr>().ok())
                .is_some_and(|address| address.is_ipv6() == ipv6)
        }))
    .then_some(DnsMode::Manual)
}

#[cfg(test)]
#[test]
fn dns_mode_distinguishes_configured_families_without_guessing() {
    assert_eq!(static_dns_mode("", false), Some(DnsMode::Automatic));
    assert_eq!(
        static_dns_mode("1.1.1.1, 8.8.4.4", false),
        Some(DnsMode::Manual)
    );
    assert_eq!(
        static_dns_mode("2001:4860:4860::8888 fe80::2%4", true),
        Some(DnsMode::Manual)
    );
    assert_eq!(static_dns_mode("2001:4860:4860::8888", false), None);
    assert_eq!(static_dns_mode("unknown", true), None);
    assert_eq!(static_dns_mode(", ", false), None);
}

pub(super) fn interfaces() -> Result<Vec<NetworkInterface>, AppError> {
    unsafe {
        let mut size = 16 * 1024u32;
        let mut storage = Vec::<u64>::new();
        let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
        let mut code = 111;
        for _ in 0..3 {
            if size > 16 * 1024 * 1024 {
                return Err(error(
                    ErrorCode::Network,
                    "Adapter snapshot exceeds safety limit",
                ));
            }
            storage.resize((size as usize).div_ceil(8), 0);
            code = GetAdaptersAddresses(
                AF_UNSPEC as u32,
                flags,
                ptr::null(),
                storage.as_mut_ptr().cast(),
                &mut size,
            );
            if code != 111 {
                break;
            }
        }
        if code != 0 {
            return Err(error(
                ErrorCode::Network,
                "Native network interface enumeration failed",
            ));
        }
        let mut out = Vec::new();
        let mut row = storage.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !row.is_null() {
            let a = &*row;
            let name = wide_string(a.FriendlyName);
            let description = wide_string(a.Description);
            let id = if a.AdapterName.is_null() {
                format!("luid-{}", a.Luid.Value)
            } else {
                CStr::from_ptr(a.AdapterName.cast())
                    .to_string_lossy()
                    .into_owned()
            };
            let mut ips = Vec::new();
            let mut ipv6 = Vec::new();
            let mut dhcp_address = false;
            let mut unicast = a.FirstUnicastAddress;
            while !unicast.is_null() {
                let u = &*unicast;
                match socket_ip(u.Address) {
                    Some(IpAddr::V4(ip)) => {
                        ips.push(ip);
                        dhcp_address |= u.PrefixOrigin == IpPrefixOriginDhcp
                            && !ip.is_unspecified()
                            && !ip.is_link_local();
                    }
                    Some(IpAddr::V6(ip)) => ipv6.push(ip),
                    None => {}
                }
                unicast = u.Next;
            }
            ipv6.sort_unstable();
            ipv6.dedup();
            let mut servers = Vec::new();
            let mut server = a.FirstDnsServerAddress;
            while !server.is_null() {
                let entry = &*server;
                if let Some(ip) = socket_ip(entry.Address) {
                    if !servers.contains(&ip) {
                        servers.push(ip);
                    }
                }
                server = entry.Next;
            }
            let mut gateway = None;
            let mut g = a.FirstGatewayAddress;
            while !g.is_null() {
                gateway = ipv4((*g).Address.lpSockaddr);
                if gateway.is_some() {
                    break;
                }
                g = (*g).Next;
            }
            let wireless = a.IfType == IF_TYPE_IEEE80211;
            let ssid = if wireless { wifi_ssid(&a.Luid) } else { None };
            let description = description.to_ascii_lowercase();
            let mut interface_row = MIB_IF_ROW2 {
                InterfaceLuid: a.Luid,
                ..MIB_IF_ROW2::default()
            };
            // HardwareInterface is bit 0 of InterfaceAndOperStatusFlags.
            let software = GetIfEntry2(&mut interface_row) == 0
                && interface_row.InterfaceAndOperStatusFlags._bitfield & 1 == 0;
            let alternate_adapter = software
                || !matches!(a.IfType, IF_TYPE_ETHERNET_CSMACD | IF_TYPE_IEEE80211)
                || [
                    "bluetooth",
                    "wi-fi direct",
                    "vmware",
                    "virtualbox",
                    "hyper-v",
                ]
                .iter()
                .any(|kind| description.contains(kind));
            let flags = a.Anonymous2.Flags;
            let ipv4_mode = (flags & IP_ADAPTER_IPV4_ENABLED != 0).then_some({
                if flags & IP_ADAPTER_DHCP_ENABLED == 0 {
                    Ipv4Assignment::Manual
                } else if dhcp_address {
                    Ipv4Assignment::Dhcp
                } else {
                    Ipv4Assignment::AutoPending
                }
            });
            let dns = servers
                .iter()
                .filter_map(|ip| match ip {
                    IpAddr::V4(ip) => Some(*ip),
                    IpAddr::V6(_) => None,
                })
                .collect();
            let dns_config = DnsConfiguration {
                ipv4_mode: dns_mode(&id, false),
                ipv6_mode: dns_mode(&id, true),
                servers,
            };
            out.push(NetworkInterface {
                id,
                name,
                index: a.Anonymous1.Anonymous.IfIndex,
                ipv4: ips,
                gateway,
                up: a.OperStatus == IfOperStatusUp,
                wireless,
                ssid,
                virtual_adapter: matches!(
                    a.IfType,
                    IF_TYPE_TUNNEL | IF_TYPE_SOFTWARE_LOOPBACK | IF_TYPE_PPP
                ) || super::is_tunnel(&description),
                alternate_adapter,
                dns,
                dns_config: Some(dns_config),
                ipv6,
                ipv4_mode,
                ipv6_enabled: Some(flags & IP_ADAPTER_IPV6_ENABLED != 0),
            });
            row = a.Next;
        }
        out.sort_by_key(|a| a.index);
        Ok(out)
    }
}

// Resolve WLAN dynamically so Server/Core installations without Wireless LAN do not fail to load.
unsafe fn wifi_ssid(luid: &NET_LUID_LH) -> Option<String> {
    use windows_sys::Win32::{
        Foundation::{FreeLibrary, HMODULE},
        NetworkManagement::WiFi::{
            wlan_intf_opcode_current_connection, WLAN_CONNECTION_ATTRIBUTES,
        },
        System::LibraryLoader::*,
    };
    let dll = crate::windows::wide("wlanapi.dll");
    let library = LoadLibraryExW(dll.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
    if library.is_null() {
        return None;
    }
    struct Library(HMODULE);
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
    let _library = Library(library);
    type Open = unsafe extern "system" fn(u32, *const c_void, *mut u32, *mut HANDLE) -> u32;
    type Query = unsafe extern "system" fn(
        HANDLE,
        *const windows_sys::core::GUID,
        i32,
        *const c_void,
        *mut u32,
        *mut *mut c_void,
        *mut i32,
    ) -> u32;
    type Close = unsafe extern "system" fn(HANDLE, *const c_void) -> u32;
    type Free = unsafe extern "system" fn(*const c_void);
    let open: Open =
        std::mem::transmute(GetProcAddress(library, c"WlanOpenHandle".as_ptr().cast())?);
    let query: Query = std::mem::transmute(GetProcAddress(
        library,
        c"WlanQueryInterface".as_ptr().cast(),
    )?);
    let close: Close =
        std::mem::transmute(GetProcAddress(library, c"WlanCloseHandle".as_ptr().cast())?);
    let free: Free =
        std::mem::transmute(GetProcAddress(library, c"WlanFreeMemory".as_ptr().cast())?);
    let (mut version, mut handle) = (0, ptr::null_mut());
    if open(2, ptr::null(), &mut version, &mut handle) != 0 {
        return None;
    }
    let mut guid = std::mem::zeroed();
    if ConvertInterfaceLuidToGuid(luid, &mut guid) != 0 {
        close(handle, ptr::null());
        return None;
    }
    let (mut size, mut data, mut kind) = (0, ptr::null_mut(), 0);
    let code = query(
        handle,
        &guid,
        wlan_intf_opcode_current_connection,
        ptr::null(),
        &mut size,
        &mut data,
        &mut kind,
    );
    let result = if code == 0
        && !data.is_null()
        && size as usize >= std::mem::size_of::<WLAN_CONNECTION_ATTRIBUTES>()
    {
        let ssid = &(*data.cast::<WLAN_CONNECTION_ATTRIBUTES>())
            .wlanAssociationAttributes
            .dot11Ssid;
        let bytes = &ssid.ucSSID[..(ssid.uSSIDLength as usize).min(ssid.ucSSID.len())];
        Some(String::from_utf8_lossy(bytes).into_owned())
    } else {
        None
    };
    if !data.is_null() {
        free(data);
    }
    close(handle, ptr::null());
    result
}
pub(super) fn preferred_binding(target: Ipv4Addr) -> Result<Option<NetworkBinding>, AppError> {
    unsafe {
        let mut destination: SOCKADDR_INET = std::mem::zeroed();
        destination.Ipv4.sin_family = AF_INET;
        destination.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes(target.octets());
        let mut route: MIB_IPFORWARD_ROW2 = std::mem::zeroed();
        let mut source: SOCKADDR_INET = std::mem::zeroed();
        if GetBestRoute2(
            ptr::null(),
            0,
            ptr::null(),
            &destination,
            0,
            &mut route,
            &mut source,
        ) != 0
        {
            return Ok(None);
        }
        let Some(source_ip) = ipv4((&source as *const SOCKADDR_INET).cast()) else {
            return Ok(None);
        };
        let Some(interface) = interfaces()?.into_iter().find(|a| {
            a.index == route.InterfaceIndex
                && a.up
                && !a.virtual_adapter
                && a.ipv4.contains(&source_ip)
        }) else {
            return Ok(None);
        };
        if source_ip.is_unspecified() || source_ip.is_loopback() || source_ip.is_link_local() {
            return Ok(None);
        }
        Ok(Some(NetworkBinding {
            interface_id: interface.id,
            interface_index: interface.index,
            source_ip,
        }))
    }
}

pub(super) struct ChangeGuard {
    handle: HANDLE,
    sender: *mut watch::Sender<u64>,
}
// The registration API serializes cancellation with completion of callbacks.
unsafe impl Send for ChangeGuard {}
unsafe impl Sync for ChangeGuard {}
unsafe extern "system" fn changed(
    context: *const c_void,
    _: *const MIB_IPINTERFACE_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    if !context.is_null() {
        let tx = &*context.cast::<watch::Sender<u64>>();
        tx.send_modify(|g| *g = g.wrapping_add(1));
    }
}
pub(super) fn subscribe(sender: watch::Sender<u64>) -> Result<ChangeGuard, AppError> {
    let sender = Box::into_raw(Box::new(sender));
    let mut handle = ptr::null_mut();
    let code = unsafe {
        NotifyIpInterfaceChange(AF_UNSPEC, Some(changed), sender.cast(), false, &mut handle)
    };
    if code != 0 {
        unsafe {
            drop(Box::from_raw(sender));
        }
        return Err(error(
            ErrorCode::Network,
            "Native network change registration failed; use polling",
        ));
    }
    Ok(ChangeGuard { handle, sender })
}
impl Drop for ChangeGuard {
    fn drop(&mut self) {
        unsafe {
            CancelMibChangeNotify2(self.handle);
            drop(Box::from_raw(self.sender));
        }
    }
}
