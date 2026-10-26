//! Apply an interface constraint as well as a source address. Never fall back.
use crate::GatewayError;
use bitnet_core::NetworkBinding;
use tokio::net::TcpSocket;

pub(crate) fn apply(socket: &TcpSocket, binding: &NetworkBinding) -> Result<(), GatewayError> {
    if binding.interface_index == 0
        || binding.interface_id.is_empty()
        || binding.source_ip.is_unspecified()
        || binding.source_ip.is_multicast()
        || binding.source_ip.is_broadcast()
    {
        return Err(GatewayError::InvalidBinding);
    }
    bind_interface(socket, binding.interface_index)?;
    socket
        .bind((binding.source_ip, 0).into())
        .map_err(|_| GatewayError::InterfaceBinding)
}

#[cfg(windows)]
fn bind_interface(socket: &TcpSocket, index: u32) -> Result<(), GatewayError> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};
    use windows_sys::Win32::Networking::WinSock::{
        setsockopt, IPPROTO_IP, IP_UNICAST_IF, SOCKET_ERROR,
    };
    let mut row = MIB_IF_ROW2 {
        InterfaceIndex: index,
        ..Default::default()
    };
    // SAFETY: the row is fully initialized and the API fills this writable row.
    if unsafe { GetIfEntry2(&mut row) } != 0 {
        return Err(GatewayError::InterfaceBinding);
    }
    let option = index.to_be();
    // SAFETY: live socket and a correctly sized, readable u32 option buffer.
    let result = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            IPPROTO_IP,
            IP_UNICAST_IF,
            (&option as *const u32).cast(),
            std::mem::size_of::<u32>() as i32,
        )
    };
    if result == SOCKET_ERROR {
        Err(GatewayError::InterfaceBinding)
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn bind_interface(socket: &TcpSocket, index: u32) -> Result<(), GatewayError> {
    use std::os::fd::AsRawFd;
    // IP_UNICAST_IF is not a sufficiently portable constraint for connected
    // TCP sockets. Resolve the supplied index through the kernel and constrain
    // both directions with SO_BINDTODEVICE. Fresh sockets can be bound by an
    // unprivileged process on modern kernels (Linux commit c427bfec18f2).
    // Older/restricted kernels fail closed; never fall back to only a source IP.
    let mut name = [0 as libc::c_char; libc::IF_NAMESIZE];
    // SAFETY: if_indextoname receives a valid IF_NAMESIZE-byte output buffer.
    if unsafe { libc::if_indextoname(index, name.as_mut_ptr()) }.is_null() {
        return Err(GatewayError::InterfaceBinding);
    }
    let length = name
        .iter()
        .position(|value| *value == 0)
        .ok_or(GatewayError::InterfaceBinding)?
        + 1;
    // SAFETY: live socket and a kernel-produced, NUL-terminated interface name.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            name.as_ptr().cast(),
            length as libc::socklen_t,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(GatewayError::InterfaceBinding)
    }
}

#[cfg(target_os = "macos")]
fn bind_interface(socket: &TcpSocket, index: u32) -> Result<(), GatewayError> {
    use std::os::fd::AsRawFd;
    let option = index;
    // SAFETY: live socket and a correctly sized, readable u32 option buffer.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_BOUND_IF,
            (&option as *const u32).cast(),
            std::mem::size_of::<u32>() as libc::socklen_t,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(GatewayError::InterfaceBinding)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn bind_interface(_: &TcpSocket, _: u32) -> Result<(), GatewayError> {
    Err(GatewayError::InterfaceBinding)
}
