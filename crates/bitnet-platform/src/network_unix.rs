use super::*;
use crate::io_error;
use std::{
    collections::BTreeMap,
    ffi::CStr,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[cfg(target_os = "macos")]
#[path = "network_macos.rs"]
mod macos;

pub(super) fn interfaces() -> Result<Vec<NetworkInterface>, AppError> {
    unsafe {
        let mut head = ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        struct Addresses(*mut libc::ifaddrs);
        impl Drop for Addresses {
            fn drop(&mut self) {
                unsafe { libc::freeifaddrs(self.0) }
            }
        }
        let _guard = Addresses(head);
        let mut row = head;
        let mut out = BTreeMap::<String, NetworkInterface>::new();
        while !row.is_null() {
            let a = &*row;
            let name = CStr::from_ptr(a.ifa_name).to_string_lossy().into_owned();
            let index = libc::if_nametoindex(a.ifa_name);
            let item = out.entry(name.clone()).or_insert_with(|| NetworkInterface {
                id: name.clone(),
                name: name.clone(),
                index,
                ipv4: Vec::new(),
                gateway: None,
                up: a.ifa_flags & libc::IFF_UP as u32 != 0
                    && a.ifa_flags & libc::IFF_RUNNING as u32 != 0,
                wireless: wireless(&name),
                ssid: None,
                virtual_adapter: a.ifa_flags & libc::IFF_LOOPBACK as u32 != 0
                    || super::is_tunnel(&name),
            });
            if !a.ifa_addr.is_null() && (*a.ifa_addr).sa_family as i32 == libc::AF_INET {
                let ip = Ipv4Addr::from(
                    (*a.ifa_addr.cast::<libc::sockaddr_in>())
                        .sin_addr
                        .s_addr
                        .to_ne_bytes(),
                );
                if !item.ipv4.contains(&ip) {
                    item.ipv4.push(ip);
                }
            }
            row = a.ifa_next;
        }
        #[cfg(target_os = "linux")]
        {
            let mut all = routes()?;
            all.sort_by_key(|r| r.metric);
            for r in all {
                if r.mask == 0 && r.gateway != 0 {
                    if let Some(item) = out.get_mut(&r.name) {
                        if item.gateway.is_none() {
                            item.gateway = Some(Ipv4Addr::from(r.gateway.to_le_bytes()));
                        }
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        for item in out.values_mut().filter(|i| i.wireless) {
            item.ssid = linux_ssid(&item.name);
        }
        #[cfg(target_os = "macos")]
        for (index, gateway) in macos::gateways() {
            if let Some(item) = out.values_mut().find(|i| i.index == index) {
                if item.gateway.is_none() {
                    item.gateway = Some(gateway);
                }
            }
        }
        Ok(out.into_values().collect())
    }
}
#[cfg(target_os = "linux")]
fn linux_ssid(name: &str) -> Option<String> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WirelessPoint {
        pointer: *mut libc::c_void,
        length: u16,
        flags: u16,
    }
    #[repr(C)]
    union WirelessData {
        point: WirelessPoint,
        padding: [u8; 16],
    }
    #[repr(C)]
    struct WirelessRequest {
        name: [libc::c_char; libc::IFNAMSIZ],
        data: WirelessData,
    }
    if name.len() >= libc::IFNAMSIZ {
        return None;
    }
    let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return None;
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut ssid = [0u8; 33];
    let mut request = WirelessRequest {
        name: [0; libc::IFNAMSIZ],
        data: WirelessData {
            point: WirelessPoint {
                pointer: ssid.as_mut_ptr().cast(),
                length: 32,
                flags: 0,
            },
        },
    };
    for (dst, src) in request.name.iter_mut().zip(name.bytes()) {
        *dst = src as libc::c_char;
    }
    // Wireless Extensions SIOCGIWESSID is read-only; unsupported/permission-denied drivers return None.
    if unsafe { libc::ioctl(fd.as_raw_fd(), 0x8B1B as libc::c_ulong, &mut request) } < 0 {
        return None;
    }
    let length = unsafe { request.data.point.length as usize }.min(32);
    let bytes = &ssid[..length];
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.is_empty() {
        None
    } else {
        String::from_utf8(bytes.to_vec()).ok()
    }
}
fn wireless(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/sys/class/net")
            .join(name)
            .join("wireless")
            .exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = name;
        false
    }
}
#[cfg(target_os = "linux")]
struct Route {
    name: String,
    dest: u32,
    gateway: u32,
    mask: u32,
    metric: u32,
}
#[cfg(target_os = "linux")]
fn routes() -> Result<Vec<Route>, AppError> {
    let text = std::fs::read_to_string("/proc/net/route").map_err(io_error)?;
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let p = line.split_whitespace().collect::<Vec<_>>();
        if p.len() < 8 {
            continue;
        }
        let parse = |i| u32::from_str_radix(p[i], 16).ok();
        if let (Some(dest), Some(gateway), Some(flags), Some(mask), Ok(metric)) =
            (parse(1), parse(2), parse(3), parse(7), p[6].parse::<u32>())
        {
            if flags & 1 != 0 && flags & 0x200 == 0 {
                rows.push(Route {
                    name: p[0].into(),
                    dest,
                    gateway,
                    mask,
                    metric,
                });
            }
        }
    }
    Ok(rows)
}
pub(super) fn preferred_binding(target: Ipv4Addr) -> Result<Option<NetworkBinding>, AppError> {
    #[cfg(target_os = "linux")]
    {
        // UDP connect selects a source without sending traffic. Compare with the kernel route table.
        // Policy routing/tunnel ambiguity is rejected rather than choosing an arbitrary NIC.
        let destination = u32::from_le_bytes(target.octets());
        let mut candidates = routes()?
            .into_iter()
            .filter(|r| destination & r.mask == r.dest & r.mask)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|r| (std::cmp::Reverse(r.mask.count_ones()), r.metric));
        let Some(route) = candidates.first() else {
            return Ok(None);
        };
        let Some(iface) = interfaces()?
            .into_iter()
            .find(|i| i.name == route.name && i.up && !i.virtual_adapter)
        else {
            return Ok(None);
        };
        let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(io_error)?;
        if socket.connect((target, 9)).is_err() {
            return Ok(None);
        }
        let std::net::IpAddr::V4(source_ip) = socket.local_addr().map_err(io_error)?.ip() else {
            return Ok(None);
        };
        if !iface.ipv4.contains(&source_ip)
            || source_ip.is_link_local()
            || source_ip.is_loopback()
            || source_ip.is_unspecified()
        {
            return Ok(None);
        }
        Ok(Some(NetworkBinding {
            interface_id: iface.id,
            interface_index: iface.index,
            source_ip,
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(io_error)?;
        if socket.connect((target, 9)).is_err() {
            return Ok(None);
        }
        let std::net::IpAddr::V4(source_ip) = socket.local_addr().map_err(io_error)?.ip() else {
            return Ok(None);
        };
        if source_ip.is_loopback() || source_ip.is_link_local() || source_ip.is_unspecified() {
            return Ok(None);
        }
        let mut candidates = interfaces()?
            .into_iter()
            .filter(|i| i.up && !i.virtual_adapter && i.ipv4.contains(&source_ip));
        let Some(interface) = candidates.next() else {
            return Ok(None);
        };
        if candidates.next().is_some() {
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
    stop: Arc<AtomicBool>,
    _polling_keepalive: Option<watch::Sender<u64>>,
    polling_only: bool,
}
pub(super) fn polling_only(guard: &ChangeGuard) -> bool {
    guard.polling_only
}
impl Drop for ChangeGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
pub(super) fn subscribe(sender: watch::Sender<u64>) -> Result<ChangeGuard, AppError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let raw = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                libc::NETLINK_ROUTE,
            )
        };
        if raw < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as u16;
        address.nl_groups = 1 | 0x10 | 0x40 | 0x100 | 0x400;
        if unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&address as *const libc::sockaddr_nl).cast(),
                std::mem::size_of_val(&address) as u32,
            )
        } != 0
        {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let timeout = libc::timeval {
            tv_sec: 1,
            tv_usec: 0,
        };
        if unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&timeout as *const libc::timeval).cast(),
                std::mem::size_of_val(&timeout) as u32,
            )
        } != 0
        {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("bitnet-netlink".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                while !flag.load(Ordering::Acquire) {
                    let n = unsafe {
                        libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0)
                    };
                    if n > 0 {
                        sender.send_modify(|g| *g = g.wrapping_add(1));
                    } else if n == 0 {
                        break;
                    } else {
                        let cause = std::io::Error::last_os_error();
                        if !matches!(
                            cause.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) {
                            break;
                        }
                    }
                }
            })
            .map_err(io_error)?;
        Ok(ChangeGuard {
            stop,
            _polling_keepalive: None,
            polling_only: false,
        })
    }
    #[cfg(target_os = "macos")]
    {
        macos::subscribe(sender)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(ChangeGuard {
            stop: Arc::new(AtomicBool::new(false)),
            _polling_keepalive: Some(sender),
            polling_only: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_snapshot_is_structurally_valid() {
        for n in interfaces().unwrap() {
            assert!(!n.id.is_empty());
            assert!(!n.name.is_empty());
        }
    }
}
