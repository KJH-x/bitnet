//! Native read-only route-table snapshot and routing-socket notifications.
use super::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub(super) fn gateways() -> Vec<(u32, Ipv4Addr)> {
    let mut mib = [
        libc::CTL_NET,
        libc::PF_ROUTE,
        0,
        libc::AF_INET,
        libc::NET_RT_DUMP,
        0,
    ];
    let mut needed = 0usize;
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            ptr::null_mut(),
            &mut needed,
            ptr::null_mut(),
            0,
        )
    } != 0
        || needed > 4 * 1024 * 1024
    {
        return Vec::new();
    }
    let mut data = vec![0u8; needed];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            data.as_mut_ptr().cast(),
            &mut needed,
            ptr::null_mut(),
            0,
        )
    } != 0
        || needed > data.len()
    {
        return Vec::new();
    }
    parse_gateways(&data[..needed])
}
fn parse_gateways(mut data: &[u8]) -> Vec<(u32, Ipv4Addr)> {
    let mut found = Vec::new();
    let header_size = std::mem::size_of::<libc::rt_msghdr>();
    while data.len() >= header_size {
        let header = unsafe { ptr::read_unaligned(data.as_ptr().cast::<libc::rt_msghdr>()) };
        let length = header.rtm_msglen as usize;
        if length < header_size || length > data.len() {
            break;
        }
        let message = &data[header_size..length];
        data = &data[length..];
        if header.rtm_version as i32 != libc::RTM_VERSION
            || header.rtm_flags & (libc::RTF_UP | libc::RTF_GATEWAY)
                != libc::RTF_UP | libc::RTF_GATEWAY
            || header.rtm_flags & (libc::RTF_REJECT | libc::RTF_BLACKHOLE | libc::RTF_HOST) != 0
        {
            continue;
        }
        let (mut destination, mut gateway, mut nonzero_mask, mut cursor, mut valid) =
            (None, None, false, 0usize, true);
        for index in 0..libc::RTAX_MAX {
            if header.rtm_addrs & (1 << index) == 0 {
                continue;
            }
            if cursor + 2 > message.len() {
                valid = false;
                break;
            }
            let size = message[cursor] as usize;
            // Darwin route sockaddr records align to uint32_t, including zero-length masks.
            let advance = size.max(1).div_ceil(4) * 4;
            if cursor + advance > message.len() {
                valid = false;
                break;
            }
            let address = &message[cursor..cursor + size];
            if index == libc::RTAX_NETMASK {
                nonzero_mask = address.get(2..).is_some_and(|b| b.iter().any(|b| *b != 0));
            }
            if size >= 8 && address[1] as i32 == libc::AF_INET {
                let ip = Ipv4Addr::new(address[4], address[5], address[6], address[7]);
                if index == libc::RTAX_DST {
                    destination = Some(ip);
                }
                if index == libc::RTAX_GATEWAY {
                    gateway = Some(ip);
                }
            }
            cursor += advance;
        }
        if valid && destination == Some(Ipv4Addr::UNSPECIFIED) && !nonzero_mask {
            if let Some(ip) = gateway.filter(|ip| !ip.is_unspecified() && !ip.is_loopback()) {
                found.push((u32::from(header.rtm_index), ip));
            }
        }
    }
    found
}
fn fallback(sender: watch::Sender<u64>) -> ChangeGuard {
    ChangeGuard {
        stop: Arc::new(AtomicBool::new(false)),
        _polling_keepalive: Some(sender),
        polling_only: true,
    }
}
pub(super) fn subscribe(sender: watch::Sender<u64>) -> Result<ChangeGuard, AppError> {
    let raw = unsafe { libc::socket(libc::AF_ROUTE, libc::SOCK_RAW, libc::AF_UNSPEC) };
    if raw < 0 {
        return Ok(fallback(sender));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let timeout = libc::timeval {
        tv_sec: 1,
        tv_usec: 0,
    };
    if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } != 0
        || unsafe {
            libc::setsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&timeout as *const libc::timeval).cast(),
                std::mem::size_of_val(&timeout) as u32,
            )
        } != 0
    {
        return Ok(fallback(sender));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let keepalive = sender.clone();
    std::thread::Builder::new()
        .name("bitnet-route".into())
        .spawn(move || {
            let mut data = [0u8; 8192];
            while !flag.load(Ordering::Acquire) {
                let length =
                    unsafe { libc::recv(fd.as_raw_fd(), data.as_mut_ptr().cast(), data.len(), 0) };
                if length >= 4 {
                    if matches!(
                        data[3] as i32,
                        libc::RTM_ADD
                            | libc::RTM_DELETE
                            | libc::RTM_CHANGE
                            | libc::RTM_NEWADDR
                            | libc::RTM_DELADDR
                            | libc::RTM_IFINFO
                    ) {
                        sender.send_modify(|g| *g = g.wrapping_add(1));
                    }
                } else if length == 0
                    || (length < 0
                        && !matches!(
                            std::io::Error::last_os_error().kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ))
                {
                    break;
                }
            }
        })
        .map_err(io_error)?;
    Ok(ChangeGuard {
        stop,
        _polling_keepalive: Some(keepalive),
        polling_only: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_truncated_route_messages_without_panicking() {
        for size in 0..512 {
            assert!(parse_gateways(&vec![0; size]).is_empty());
        }
    }
    #[test]
    fn parses_only_default_gateway_and_rejects_each_truncated_prefix() {
        let mut header: libc::rt_msghdr = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&header);
        header.rtm_msglen = (size + 36) as u16;
        header.rtm_version = libc::RTM_VERSION as u8;
        header.rtm_flags = libc::RTF_UP | libc::RTF_GATEWAY;
        header.rtm_index = 7;
        header.rtm_addrs = libc::RTA_DST | libc::RTA_GATEWAY | libc::RTA_NETMASK;
        let mut message = unsafe {
            std::slice::from_raw_parts((&header as *const libc::rt_msghdr).cast::<u8>(), size)
        }
        .to_vec();
        message.extend([
            16,
            libc::AF_INET as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]);
        message.extend([
            16,
            libc::AF_INET as u8,
            0,
            0,
            10,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]);
        message.extend([0, 0, 0, 0]);
        assert_eq!(
            parse_gateways(&message),
            vec![(7, Ipv4Addr::new(10, 0, 0, 1))]
        );
        for prefix in 0..message.len() {
            assert!(parse_gateways(&message[..prefix]).is_empty());
        }
    }
}
