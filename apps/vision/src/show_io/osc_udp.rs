//! Zero-external-dependency Open Sound Control (OSC 1.0) over local loopback / RFC1918 UDP.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

use anyhow::{bail, ensure, Result};

use crate::validate_local_socket;

pub const DEFAULT_OSC_ADDR: &str = "127.0.0.1:9000";

#[derive(Clone, Debug, PartialEq)]
pub enum OscArg {
    Int(i32),
    Float(f32),
    Str(String),
}

fn push_osc_padded(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
    buf.push(0);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn read_osc_padded(buf: &[u8], cursor: &mut usize) -> Result<String> {
    let start = *cursor;
    let nul = buf[start..]
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| anyhow::anyhow!("unterminated OSC string"))?;
    let s = std::str::from_utf8(&buf[start..start + nul])?.to_string();
    let total = ((nul + 1 + 3) / 4) * 4;
    ensure!(start + total <= buf.len(), "truncated OSC padding");
    *cursor = start + total;
    Ok(s)
}

/// Encodes an OSC 1.0 packet (`address`, `,type_tags`, and 4-byte aligned big-endian payload).
pub fn encode_osc_message(address: &str, args: &[OscArg]) -> Result<Vec<u8>> {
    ensure!(address.starts_with('/'), "OSC address must start with '/', got '{address}'");
    let mut out = Vec::with_capacity(64);
    push_osc_padded(&mut out, address.as_bytes());

    let mut tags = String::with_capacity(args.len() + 1);
    tags.push(',');
    for arg in args {
        match arg {
            OscArg::Int(_) => tags.push('i'),
            OscArg::Float(_) => tags.push('f'),
            OscArg::Str(_) => tags.push('s'),
        }
    }
    push_osc_padded(&mut out, tags.as_bytes());

    for arg in args {
        match arg {
            OscArg::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
            OscArg::Str(s) => push_osc_padded(&mut out, s.as_bytes()),
        }
    }
    Ok(out)
}

/// Decodes an OSC 1.0 packet produced by `encode_osc_message` or external sandbox gear.
pub fn decode_osc_message(pkt: &[u8]) -> Result<(String, Vec<OscArg>)> {
    let mut cursor = 0usize;
    let address = read_osc_padded(pkt, &mut cursor)?;
    let tags = read_osc_padded(pkt, &mut cursor)?;
    ensure!(tags.starts_with(','), "OSC typetag must start with ',', got '{tags}'");
    let mut args = Vec::with_capacity(tags.len() - 1);
    for ch in tags[1..].chars() {
        match ch {
            'i' => {
                ensure!(cursor + 4 <= pkt.len(), "truncated OSC int32");
                let v = i32::from_be_bytes(pkt[cursor..cursor + 4].try_into().unwrap());
                cursor += 4;
                args.push(OscArg::Int(v));
            }
            'f' => {
                ensure!(cursor + 4 <= pkt.len(), "truncated OSC float32");
                let v = f32::from_bits(u32::from_be_bytes(pkt[cursor..cursor + 4].try_into().unwrap()));
                cursor += 4;
                args.push(OscArg::Float(v));
            }
            's' => args.push(OscArg::Str(read_osc_padded(pkt, &mut cursor)?)),
            other => bail!("unsupported OSC type tag '{other}'"),
        }
    }
    Ok((address, args))
}

pub struct OscUdpBridge {
    socket: UdpSocket,
    target: SocketAddr,
}

impl OscUdpBridge {
    /// Binds a non-blocking UDP socket targeting a validated loopback or RFC1918 LAN endpoint.
    /// Loopback destinations bind strictly to loopback; validated RFC1918/ULA LAN destinations
    /// bind an ephemeral OS-routed socket so packets can traverse the private LAN interface.
    pub fn bind(target: SocketAddr) -> Result<Self> {
        let valid_target = validate_local_socket(target, "OSC UDP target")?;
        let bind_addr = match valid_target.ip() {
            IpAddr::V4(ip) if ip.is_loopback() => SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            IpAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            IpAddr::V6(ip) if ip.is_loopback() => SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
            IpAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        };
        let socket = UdpSocket::bind(bind_addr)?;
        Ok(Self { socket, target: valid_target })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    pub fn target(&self) -> SocketAddr {
        self.target
    }

    pub fn send_track_xyz(&self, track_id: u32, xyz: [f32; 3]) -> Result<usize> {
        let addr = format!("/louis/track/{track_id}/xyz");
        let pkt = encode_osc_message(
            &addr,
            &[OscArg::Float(xyz[0]), OscArg::Float(xyz[1]), OscArg::Float(xyz[2])],
        )?;
        Ok(self.socket.send_to(&pkt, self.target)?)
    }

    pub fn send_cue(&self, cue_name: &str, level: f32) -> Result<usize> {
        let sanitized = cue_name.trim().trim_start_matches('/');
        ensure!(!sanitized.is_empty(), "cue name cannot be empty");
        let addr = format!("/louis/cue/{sanitized}");
        let pkt = encode_osc_message(&addr, &[OscArg::Float(level)])?;
        Ok(self.socket.send_to(&pkt, self.target)?)
    }

    pub fn send_interlock_halt(&self, reason: &str) -> Result<usize> {
        let pkt = encode_osc_message("/louis/interlock/halt", &[OscArg::Str(reason.to_string())])?;
        Ok(self.socket.send_to(&pkt, self.target)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_osc_udp_roundtrip_and_wan_rejection() {
        assert!(OscUdpBridge::bind("1.1.1.1:9000".parse().unwrap()).is_err());

        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let rx_addr = rx.local_addr().unwrap();
        let bridge = OscUdpBridge::bind(rx_addr).unwrap();

        bridge.send_track_xyz(7, [1.25, 0.85, 3.5]).unwrap();
        let mut buf = [0u8; 256];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(n % 4, 0);
        let (addr, args) = decode_osc_message(&buf[..n]).unwrap();
        assert_eq!(addr, "/louis/track/7/xyz");
        assert_eq!(args, vec![OscArg::Float(1.25), OscArg::Float(0.85), OscArg::Float(3.5)]);

        bridge.send_interlock_halt("hazard_zone").unwrap();
        let (n2, _) = rx.recv_from(&mut buf).unwrap();
        let (addr2, args2) = decode_osc_message(&buf[..n2]).unwrap();
        assert_eq!(addr2, "/louis/interlock/halt");
        assert_eq!(args2, vec![OscArg::Str("hazard_zone".into())]);
        assert!(bridge.local_addr().unwrap().ip().is_loopback());

        // V-03: RFC1918 LAN target must bind an OS-routable unspecified socket (not 127.0.0.1)
        let lan_bridge = OscUdpBridge::bind("192.168.50.20:9000".parse().unwrap()).unwrap();
        assert!(lan_bridge.local_addr().unwrap().ip().is_unspecified());
    }
}
