//! ICMP echo ("ping") using the Windows IP Helper API, which needs no
//! administrator rights, unlike raw sockets.

use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY, IP_BAD_DESTINATION,
    IP_DEST_HOST_UNREACHABLE, IP_DEST_NET_UNREACHABLE, IP_GENERAL_FAILURE, IP_REQ_TIMED_OUT,
    IP_SUCCESS, IP_TTL_EXPIRED_TRANSIT,
};

use crate::stats::PingOutcome;

const PAYLOAD: [u8; 32] = [b'P'; 32];

/// An open ICMP handle. Not `Send`: create it on the thread that uses it.
pub struct Pinger {
    handle: HANDLE,
}

impl Pinger {
    pub fn new() -> Result<Pinger, String> {
        let handle = unsafe { IcmpCreateFile() };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            let err = unsafe { GetLastError() };
            return Err(format!("IcmpCreateFile failed (error {err})"));
        }
        Ok(Pinger { handle })
    }

    pub fn ping(&self, host: &str, timeout: Duration) -> PingOutcome {
        let addr = match resolve_v4(host) {
            Ok(a) => a,
            Err(e) => return PingOutcome::Error(e),
        };

        let reply_size = std::mem::size_of::<ICMP_ECHO_REPLY>() + PAYLOAD.len() + 8;
        let mut reply_buf = vec![0u8; reply_size];
        let timeout_ms = timeout.as_millis().clamp(1, u32::MAX as u128) as u32;

        let replies = unsafe {
            IcmpSendEcho(
                self.handle,
                u32::from_ne_bytes(addr.octets()),
                PAYLOAD.as_ptr() as *const _,
                PAYLOAD.len() as u16,
                ptr::null(),
                reply_buf.as_mut_ptr() as *mut _,
                reply_size as u32,
                timeout_ms,
            )
        };

        if replies == 0 {
            let err = unsafe { GetLastError() };
            return if err == IP_REQ_TIMED_OUT {
                PingOutcome::Timeout
            } else {
                PingOutcome::Error(status_text(err))
            };
        }

        // SAFETY: IcmpSendEcho wrote at least one ICMP_ECHO_REPLY at the start of the buffer.
        let reply = unsafe { ptr::read_unaligned(reply_buf.as_ptr() as *const ICMP_ECHO_REPLY) };
        match reply.Status {
            IP_SUCCESS => PingOutcome::Reply(Duration::from_millis(reply.RoundTripTime as u64)),
            IP_REQ_TIMED_OUT => PingOutcome::Timeout,
            other => PingOutcome::Error(status_text(other)),
        }
    }
}

impl Drop for Pinger {
    fn drop(&mut self) {
        unsafe {
            IcmpCloseHandle(self.handle);
        }
    }
}

fn status_text(code: u32) -> String {
    match code {
        IP_DEST_NET_UNREACHABLE => "network unreachable".into(),
        IP_DEST_HOST_UNREACHABLE => "host unreachable".into(),
        IP_BAD_DESTINATION => "bad destination".into(),
        IP_TTL_EXPIRED_TRANSIT => "TTL expired".into(),
        IP_GENERAL_FAILURE => "general failure".into(),
        other => format!("ICMP error {other}"),
    }
}

/// Resolve a host name or dotted address to an IPv4 address.
fn resolve_v4(host: &str) -> Result<Ipv4Addr, String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => Ok(v4),
            IpAddr::V6(_) => Err("IPv6 addresses are not supported yet".into()),
        };
    }
    let addrs = (host, 0u16)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {host}: {e}"))?;
    let mut saw_v6 = false;
    for a in addrs {
        match a.ip() {
            IpAddr::V4(v4) => return Ok(v4),
            IpAddr::V6(_) => saw_v6 = true,
        }
    }
    Err(if saw_v6 {
        format!("{host} only has IPv6 addresses, which are not supported yet")
    } else {
        format!("cannot resolve {host}")
    })
}
