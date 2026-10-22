//! Minimal Art-Net: ArtDmx parsing/encoding and a background receiver that
//! keeps the latest frame of every universe.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

pub const DEFAULT_PORT: u16 = 6454;
const ID: &[u8; 8] = b"Art-Net\0";
const OP_DMX: u16 = 0x5000;
const HEADER_LEN: usize = 18;

#[derive(Debug, PartialEq, Eq)]
pub struct ArtDmx<'a> {
    pub sequence: u8,
    /// 15-bit port-address: net << 8 | subnet << 4 | universe.
    pub port_address: u16,
    pub data: &'a [u8],
}

pub fn parse(packet: &[u8]) -> Option<ArtDmx<'_>> {
    if packet.len() < HEADER_LEN || &packet[..8] != ID {
        return None;
    }
    if u16::from_le_bytes([packet[8], packet[9]]) != OP_DMX {
        return None;
    }
    let len = usize::from(u16::from_be_bytes([packet[16], packet[17]])).min(512);
    let data = packet.get(HEADER_LEN..HEADER_LEN + len)?;
    Some(ArtDmx {
        sequence: packet[12],
        port_address: u16::from_le_bytes([packet[14], packet[15] & 0x7f]),
        data,
    })
}

pub fn encode(port_address: u16, sequence: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len().min(512);
    // ArtDmx requires an even length of at least 2.
    let padded = (len + (len & 1)).max(2);
    let mut p = Vec::with_capacity(HEADER_LEN + padded);
    p.extend_from_slice(ID);
    p.extend_from_slice(&OP_DMX.to_le_bytes());
    p.extend_from_slice(&[0, 14, sequence, 0]);
    p.extend_from_slice(&(port_address & 0x7fff).to_le_bytes());
    p.extend_from_slice(&(padded as u16).to_be_bytes());
    p.extend_from_slice(&data[..len]);
    p.resize(HEADER_LEN + padded, 0);
    p
}

#[derive(Clone, Debug)]
pub struct Universe {
    pub data: [u8; 512],
    pub last_seen: Instant,
    pub packets: u64,
    pub source: SocketAddr,
}

/// Latest frame per port-address, shared between the receiver thread and readers.
#[derive(Default)]
pub struct DmxStore {
    universes: Mutex<HashMap<u16, Universe>>,
}

impl DmxStore {
    pub fn update(&self, frame: &ArtDmx, source: SocketAddr) {
        let mut map = self.universes.lock().unwrap();
        let u = map.entry(frame.port_address).or_insert_with(|| Universe {
            data: [0; 512],
            last_seen: Instant::now(),
            packets: 0,
            source,
        });
        u.data[..frame.data.len()].copy_from_slice(frame.data);
        u.last_seen = Instant::now();
        u.packets += 1;
        u.source = source;
    }

    /// Copy of one universe's channel data.
    pub fn get(&self, port_address: u16) -> Option<[u8; 512]> {
        self.universes.lock().unwrap().get(&port_address).map(|u| u.data)
    }

    pub fn snapshot(&self) -> HashMap<u16, Universe> {
        self.universes.lock().unwrap().clone()
    }
}

/// Whether no other program listens on `bind:port`. Binds it once without
/// address reuse: that fails while anyone else holds the port, even when they,
/// like `Receiver`, allow sharing it (and then the Art-Net would go to them).
pub fn port_is_free(bind: Ipv4Addr, port: u16) -> bool {
    std::net::UdpSocket::bind(SocketAddrV4::new(bind, port)).is_ok()
}

/// `preferred` if it is free, else the next free port above it (or
/// `preferred` again if none is free for a while).
pub fn free_port(bind: Ipv4Addr, preferred: u16) -> u16 {
    (preferred..=preferred.saturating_add(200)).find(|&p| port_is_free(bind, p)).unwrap_or(preferred)
}

/// Like `free_port`, but gives `preferred` up to `grace` to become free first:
/// a Tapetum that is just restarting still holds it for a moment.
pub fn free_port_after(bind: Ipv4Addr, preferred: u16, grace: Duration) -> u16 {
    let until = Instant::now() + grace;
    while !port_is_free(bind, preferred) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    free_port(bind, preferred)
}

pub struct Receiver {
    pub store: Arc<DmxStore>,
    pub local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Receiver {
    /// Binds `bind:port` with address (and on Unix port) reuse so other
    /// Art-Net tools on the same machine keep working.
    pub fn start(bind: Ipv4Addr, port: u16) -> io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
        socket.set_reuse_port(true)?;
        socket.set_broadcast(true)?;
        socket.bind(&SocketAddrV4::new(bind, port).into())?;
        let socket: UdpSocket = socket.into();
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let local_addr = socket.local_addr()?;

        let store = Arc::new(DmxStore::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (store, stop) = (store.clone(), stop.clone());
            std::thread::Builder::new().name("artnet-rx".into()).spawn(move || {
                let mut buf = [0u8; 1024];
                while !stop.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buf) {
                        Ok((n, src)) => {
                            if let Some(frame) = parse(&buf[..n]) {
                                store.update(&frame, src);
                            }
                        }
                        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                        // Windows reports ICMP port-unreachable as a recv error; keep going.
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            })?
        };
        Ok(Receiver { store, local_addr, stop, thread: Some(thread) })
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let data: Vec<u8> = (0..=255).chain(0..=255).collect();
        let pkt = encode(0x0123, 7, &data);
        assert_eq!(pkt.len(), 530);
        let f = parse(&pkt).unwrap();
        assert_eq!((f.sequence, f.port_address, f.data), (7, 0x0123, &data[..]));
    }

    #[test]
    fn matches_blinderkitten_header() {
        // DMXArtNetDevice::sendDMXValuesInternal: [14] = subnet<<4|universe, [15] = net.
        let pkt = encode((1 << 8) | (2 << 4) | 3, 0, &[0; 512]);
        assert_eq!((pkt[14], pkt[15], pkt[16], pkt[17]), (0x23, 1, 2, 0));
    }

    #[test]
    fn odd_length_is_padded() {
        let pkt = encode(0, 0, &[9]);
        assert_eq!(parse(&pkt).unwrap().data, &[9, 0]);
    }

    #[test]
    fn rejects_other_opcodes() {
        let mut pkt = encode(0, 0, &[0; 2]);
        pkt[9] = 0x20; // ArtPoll
        assert!(parse(&pkt).is_none());
    }

    #[test]
    fn receives_over_loopback() {
        let rx = Receiver::start(Ipv4Addr::LOCALHOST, 0).unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.send_to(&encode(5, 1, &[42; 512]), rx.local_addr).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while rx.store.get(5).is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(rx.store.get(5).unwrap()[511], 42);
    }

    #[test]
    fn taken_port_is_skipped() {
        // Another program holding a port (here: a plain socket on a free one).
        let other = UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = other.local_addr().unwrap().port();
        assert!(!port_is_free(Ipv4Addr::UNSPECIFIED, port));
        let next = free_port(Ipv4Addr::UNSPECIFIED, port);
        assert!(next > port && port_is_free(Ipv4Addr::UNSPECIFIED, next));
        drop(other);
        assert!(port_is_free(Ipv4Addr::UNSPECIFIED, port));
        // One that shares the port, like another Tapetum: taken all the same.
        let shared = Receiver::start(Ipv4Addr::UNSPECIFIED, 0).unwrap();
        assert!(!port_is_free(Ipv4Addr::UNSPECIFIED, shared.local_addr.port()));
    }
}
