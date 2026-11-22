//! Discovery through the system's DNS-SD service (mDNSResponder) on macOS
//! and iOS, using the `dns_sd` C API in libSystem.
//!
//! Every operation shares one connection to the daemon
//! (`kDNSServiceFlagsShareConnection`). A thread waits for the connection's
//! socket and processes replies; every call into the API, from that thread
//! or from [`Discovery::advertise`], holds one lock, so callbacks run one at
//! a time and own the browse state.
//!
//! A browse result is resolved for its port and TXT record (kept open, so a
//! code opened or closed later is seen), then its host name is looked up for
//! addresses. iOS asks the user for local-network access on first use;
//! `Info.plist` names the service type (`NSBonjourServices`).

use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_int, c_void},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

use dexdrop_core::{PeerId, SERVICE_TYPE};
use tokio::sync::mpsc;

use super::{
    Advertisement, DiscoveryEvent, PeerTxt, discovered, instance_name, parse_txt, txt_entries,
};

// ------------------------------------------------------------------ FFI

#[allow(non_camel_case_types)]
type DNSServiceRef = *mut c_void;

const FLAGS_MORE_COMING: u32 = 0x1;
const FLAGS_ADD: u32 = 0x2;
const FLAGS_SHARE_CONNECTION: u32 = 0x4000;
const PROTOCOL_IPV4: u32 = 0x01;
const PROTOCOL_IPV6: u32 = 0x02;
const ERR_NO_ERROR: i32 = 0;
const ERR_POLICY_DENIED: i32 = -65570;
const ERR_SERVICE_NOT_RUNNING: i32 = -65563;

type RegisterReply = extern "C" fn(
    DNSServiceRef,
    u32,
    i32,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_void,
);
type BrowseReply = extern "C" fn(
    DNSServiceRef,
    u32,
    u32,
    i32,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_void,
);
type ResolveReply = extern "C" fn(
    DNSServiceRef,
    u32,
    u32,
    i32,
    *const c_char,
    *const c_char,
    u16,
    u16,
    *const u8,
    *mut c_void,
);
type AddrInfoReply = extern "C" fn(
    DNSServiceRef,
    u32,
    u32,
    i32,
    *const c_char,
    *const libc::sockaddr,
    u32,
    *mut c_void,
);

unsafe extern "C" {
    fn DNSServiceCreateConnection(sd: *mut DNSServiceRef) -> i32;
    fn DNSServiceRefSockFD(sd: DNSServiceRef) -> c_int;
    fn DNSServiceProcessResult(sd: DNSServiceRef) -> i32;
    fn DNSServiceRefDeallocate(sd: DNSServiceRef);
    #[allow(clippy::too_many_arguments)]
    fn DNSServiceRegister(
        sd: *mut DNSServiceRef,
        flags: u32,
        interface: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port: u16,
        txt_len: u16,
        txt: *const c_void,
        callback: RegisterReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceUpdateRecord(
        sd: DNSServiceRef,
        record: *mut c_void,
        flags: u32,
        len: u16,
        data: *const c_void,
        ttl: u32,
    ) -> i32;
    fn DNSServiceBrowse(
        sd: *mut DNSServiceRef,
        flags: u32,
        interface: u32,
        regtype: *const c_char,
        domain: *const c_char,
        callback: BrowseReply,
        context: *mut c_void,
    ) -> i32;
    #[allow(clippy::too_many_arguments)]
    fn DNSServiceResolve(
        sd: *mut DNSServiceRef,
        flags: u32,
        interface: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        callback: ResolveReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceGetAddrInfo(
        sd: *mut DNSServiceRef,
        flags: u32,
        interface: u32,
        protocol: u32,
        hostname: *const c_char,
        callback: AddrInfoReply,
        context: *mut c_void,
    ) -> i32;
}

/// `_dexdrop._tcp` from `_dexdrop._tcp.local.`.
fn regtype() -> CString {
    let t = SERVICE_TYPE
        .trim_end_matches('.')
        .trim_end_matches(".local");
    CString::new(t).expect("no NUL in the service type")
}

fn explain(err: i32) -> String {
    match err {
        ERR_POLICY_DENIED => "Local network access is off for DexDrop. Turn it on in \
                              Settings > Privacy & Security > Local Network."
            .into(),
        ERR_SERVICE_NOT_RUNNING => "The system's Bonjour service is not running.".into(),
        other => format!("Bonjour error {other}."),
    }
}

/// A TXT record: length-prefixed `key=value` strings.
fn encode_txt(entries: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in entries {
        let mut item = format!("{key}={value}").into_bytes();
        item.truncate(255);
        out.push(item.len() as u8);
        out.extend_from_slice(&item);
    }
    out
}

fn decode_txt(txt: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = txt;
    while let Some((&len, tail)) = rest.split_first() {
        let len = (len as usize).min(tail.len());
        let (item, next) = tail.split_at(len);
        if let Some(eq) = item.iter().position(|b| *b == b'=') {
            let key = String::from_utf8_lossy(&item[..eq]).to_ascii_lowercase();
            let value = String::from_utf8_lossy(&item[eq + 1..]).into_owned();
            out.entry(key).or_insert(value);
        }
        rest = next;
    }
    out
}

/// # Safety
/// `addr` points to a socket address from the daemon.
unsafe fn socket_addr(addr: *const libc::sockaddr, port: u16) -> Option<SocketAddr> {
    if addr.is_null() {
        return None;
    }
    // SAFETY: the family says which structure the daemon passed.
    unsafe {
        match c_int::from((*addr).sa_family) {
            libc::AF_INET => {
                let v4 = &*(addr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr));
                Some(SocketAddr::new(IpAddr::V4(ip), port))
            }
            libc::AF_INET6 => {
                let v6 = &*(addr as *const libc::sockaddr_in6);
                let ip = Ipv6Addr::from(v6.sin6_addr.s6_addr);
                Some(SocketAddr::V6(SocketAddrV6::new(
                    ip,
                    port,
                    0,
                    v6.sin6_scope_id,
                )))
            }
            _ => None,
        }
    }
}

// ------------------------------------------------------------ browse state

/// One browsed service instance and its open resolve and address queries.
struct Service {
    /// Interfaces it was seen on; it is lost when none is left.
    interfaces: HashSet<u32>,
    resolve: DNSServiceRef,
    resolve_ctx: Box<OpCtx>,
    lookup: Option<(DNSServiceRef, Box<OpCtx>, String)>,
    txt: Option<PeerTxt>,
    port: u16,
    addrs: Vec<SocketAddr>,
}

/// The context a resolve or address callback gets: which service it is for.
struct OpCtx {
    state: *mut State,
    key: String,
}

/// Everything the callbacks touch. Only used under [`Discovery::conn`].
struct State {
    main: DNSServiceRef,
    own_id: PeerId,
    events: mpsc::UnboundedSender<DiscoveryEvent>,
    services: HashMap<String, Service>,
}

impl State {
    fn report(&self, key: &str) {
        let Some(service) = self.services.get(key) else {
            return;
        };
        let Some(txt) = service.txt.clone() else {
            return;
        };
        if txt.peer_id == self.own_id {
            return;
        }
        let addrs = service
            .addrs
            .iter()
            .map(|a| {
                let mut a = *a;
                a.set_port(service.port);
                a
            })
            .collect();
        if let Some(found) = discovered(txt, addrs) {
            let _ = self.events.send(DiscoveryEvent::Found(found));
        }
    }

    fn unavailable(&self, err: i32) {
        let _ = self.events.send(DiscoveryEvent::Unavailable(explain(err)));
    }

    /// Ends a service's queries and reports it lost.
    fn forget(&mut self, key: &str) {
        let Some(service) = self.services.remove(key) else {
            return;
        };
        // SAFETY: subordinate refs of the shared connection, deallocated once.
        unsafe {
            DNSServiceRefDeallocate(service.resolve);
            if let Some((lookup, _, _)) = &service.lookup {
                DNSServiceRefDeallocate(*lookup);
            }
        }
        if let Some(txt) = service.txt
            && txt.peer_id != self.own_id
        {
            let _ = self.events.send(DiscoveryEvent::Lost(txt.peer_id));
        }
    }
}

extern "C" fn on_browse(
    _sd: DNSServiceRef,
    flags: u32,
    interface: u32,
    err: i32,
    name: *const c_char,
    regtype: *const c_char,
    domain: *const c_char,
    context: *mut c_void,
) {
    // SAFETY: the context is the State, used only under the lock.
    let state = unsafe { &mut *(context as *mut State) };
    if err != ERR_NO_ERROR {
        state.unavailable(err);
        return;
    }
    // SAFETY: the daemon passes NUL-terminated strings.
    let key = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    if flags & FLAGS_ADD == 0 {
        let gone = match state.services.get_mut(&key) {
            Some(service) => {
                service.interfaces.remove(&interface);
                service.interfaces.is_empty()
            }
            None => false,
        };
        if gone {
            state.forget(&key);
        }
        return;
    }
    if let Some(service) = state.services.get_mut(&key) {
        service.interfaces.insert(interface);
        return;
    }
    let mut ctx = Box::new(OpCtx {
        state: state as *mut State,
        key: key.clone(),
    });
    let mut resolve = state.main;
    // SAFETY: a subordinate ref on the shared connection; `ctx` lives in the
    // Service as long as the ref.
    let started = unsafe {
        DNSServiceResolve(
            &mut resolve,
            FLAGS_SHARE_CONNECTION,
            interface,
            name,
            regtype,
            domain,
            on_resolve,
            &mut *ctx as *mut OpCtx as *mut c_void,
        )
    };
    if started != ERR_NO_ERROR {
        return;
    }
    state.services.insert(
        key,
        Service {
            interfaces: HashSet::from([interface]),
            resolve,
            resolve_ctx: ctx,
            lookup: None,
            txt: None,
            port: 0,
            addrs: Vec::new(),
        },
    );
}

extern "C" fn on_resolve(
    _sd: DNSServiceRef,
    _flags: u32,
    interface: u32,
    err: i32,
    _fullname: *const c_char,
    host: *const c_char,
    port: u16,
    txt_len: u16,
    txt: *const u8,
    context: *mut c_void,
) {
    if err != ERR_NO_ERROR {
        return;
    }
    // SAFETY: the OpCtx lives as long as this resolve.
    let op = unsafe { &*(context as *const OpCtx) };
    // SAFETY: its State is used only under the lock.
    let state = unsafe { &mut *op.state };
    let record = if txt.is_null() {
        &[][..]
    } else {
        // SAFETY: the daemon passes `txt_len` bytes.
        unsafe { std::slice::from_raw_parts(txt, txt_len as usize) }
    };
    let fields = decode_txt(record);
    let parsed = parse_txt(|key| fields.get(key).cloned());
    // SAFETY: a NUL-terminated host name.
    let host_name = unsafe { CStr::from_ptr(host) }
        .to_string_lossy()
        .into_owned();
    let main = state.main;
    let state_ptr = op.state;
    let key = op.key.clone();
    let Some(service) = state.services.get_mut(&key) else {
        return;
    };
    service.txt = parsed;
    service.port = u16::from_be(port);
    let same_host = matches!(&service.lookup, Some((_, _, h)) if *h == host_name);
    if !same_host {
        if let Some((old, _, _)) = service.lookup.take() {
            // SAFETY: a subordinate ref, deallocated once.
            unsafe { DNSServiceRefDeallocate(old) };
        }
        service.addrs.clear();
        let mut ctx = Box::new(OpCtx {
            state: state_ptr,
            key: key.clone(),
        });
        let mut lookup = main;
        // SAFETY: as for the resolve; `host` is NUL-terminated.
        let started = unsafe {
            DNSServiceGetAddrInfo(
                &mut lookup,
                FLAGS_SHARE_CONNECTION,
                interface,
                PROTOCOL_IPV4 | PROTOCOL_IPV6,
                host,
                on_addr,
                &mut *ctx as *mut OpCtx as *mut c_void,
            )
        };
        if started == ERR_NO_ERROR {
            service.lookup = Some((lookup, ctx, host_name));
        }
    }
    state.report(&key);
}

extern "C" fn on_addr(
    _sd: DNSServiceRef,
    flags: u32,
    _interface: u32,
    err: i32,
    _host: *const c_char,
    addr: *const libc::sockaddr,
    _ttl: u32,
    context: *mut c_void,
) {
    if err != ERR_NO_ERROR {
        return;
    }
    // SAFETY: as in `on_resolve`.
    let op = unsafe { &*(context as *const OpCtx) };
    let state = unsafe { &mut *op.state };
    // SAFETY: a socket address from the daemon.
    let Some(addr) = (unsafe { socket_addr(addr, 0) }) else {
        return;
    };
    let key = op.key.clone();
    let Some(service) = state.services.get_mut(&key) else {
        return;
    };
    if flags & FLAGS_ADD != 0 {
        if !service.addrs.contains(&addr) {
            service.addrs.push(addr);
        }
    } else {
        service.addrs.retain(|a| *a != addr);
    }
    if flags & FLAGS_MORE_COMING == 0 {
        state.report(&key);
    }
}

extern "C" fn on_register(
    _sd: DNSServiceRef,
    _flags: u32,
    err: i32,
    _name: *const c_char,
    _regtype: *const c_char,
    _domain: *const c_char,
    context: *mut c_void,
) {
    if err != ERR_NO_ERROR {
        // SAFETY: the context is the State, used only under the lock.
        let state = unsafe { &*(context as *const State) };
        state.unavailable(err);
    }
}

// ------------------------------------------------------------ the adapter

struct Conn {
    state: Box<State>,
    browse: DNSServiceRef,
    /// The registration and what it advertises.
    registered: Option<(DNSServiceRef, Advertisement)>,
}

// SAFETY: the refs are used only under the Mutex that owns them.
unsafe impl Send for Conn {}

/// Browsing and advertising through the system's Bonjour service.
pub struct Discovery {
    conn: Arc<Mutex<Conn>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    own_id: PeerId,
}

impl Discovery {
    /// Starts browsing; events go to `events`.
    pub fn start(
        own_id: PeerId,
        events: mpsc::UnboundedSender<DiscoveryEvent>,
    ) -> Result<Self, String> {
        let mut main: DNSServiceRef = ptr::null_mut();
        // SAFETY: `main` receives a new connection.
        let err = unsafe { DNSServiceCreateConnection(&mut main) };
        if err != ERR_NO_ERROR {
            return Err(explain(err));
        }
        let mut state = Box::new(State {
            main,
            own_id,
            events,
            services: HashMap::new(),
        });
        let regtype = regtype();
        let mut browse = main;
        // SAFETY: a subordinate ref; the State outlives it (both are freed
        // together in Drop, after the thread stopped).
        let err = unsafe {
            DNSServiceBrowse(
                &mut browse,
                FLAGS_SHARE_CONNECTION,
                0,
                regtype.as_ptr(),
                ptr::null(),
                on_browse,
                &mut *state as *mut State as *mut c_void,
            )
        };
        if err != ERR_NO_ERROR {
            // SAFETY: the connection made above.
            unsafe { DNSServiceRefDeallocate(main) };
            return Err(explain(err));
        }
        // SAFETY: a valid connection.
        let fd = unsafe { DNSServiceRefSockFD(main) };
        let conn = Arc::new(Mutex::new(Conn {
            state,
            browse,
            registered: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (conn, stop) = (conn.clone(), stop.clone());
            std::thread::Builder::new()
                .name("dexdrop-bonjour".into())
                .spawn(move || run(fd, conn, stop))
                .map_err(|e| e.to_string())?
        };
        Ok(Self {
            conn,
            stop,
            thread: Some(thread),
            own_id,
        })
    }

    /// Advertises this device, replacing an earlier advertisement.
    ///
    /// A change of name or open code updates the TXT record in place:
    /// withdrawing and registering again would make other devices see the
    /// device leave and come back, in either order.
    pub fn advertise(&mut self, ad: &Advertisement) -> Result<(), String> {
        let txt = encode_txt(&txt_entries(ad));
        {
            let mut conn = lock(&self.conn);
            if let Some((sd, current)) = &mut conn.registered
                && current.peer_id == ad.peer_id
                && current.port == ad.port
            {
                // SAFETY: the registration's ref, under the lock.
                let err = unsafe {
                    DNSServiceUpdateRecord(
                        *sd,
                        ptr::null_mut(),
                        0,
                        txt.len() as u16,
                        txt.as_ptr() as *const c_void,
                        0,
                    )
                };
                if err == ERR_NO_ERROR {
                    *current = ad.clone();
                    return Ok(());
                }
            }
        }
        self.withdraw();
        let name = CString::new(instance_name(&ad.peer_id)).map_err(|e| e.to_string())?;
        let regtype = regtype();
        let mut conn = lock(&self.conn);
        let main = conn.state.main;
        let context = &mut *conn.state as *mut State as *mut c_void;
        let mut sd = main;
        // SAFETY: a subordinate ref on the shared connection, under the lock.
        let err = unsafe {
            DNSServiceRegister(
                &mut sd,
                FLAGS_SHARE_CONNECTION,
                0,
                name.as_ptr(),
                regtype.as_ptr(),
                ptr::null(),
                ptr::null(),
                ad.port.to_be(),
                txt.len() as u16,
                txt.as_ptr() as *const c_void,
                on_register,
                context,
            )
        };
        if err != ERR_NO_ERROR {
            return Err(explain(err));
        }
        conn.registered = Some((sd, ad.clone()));
        Ok(())
    }

    /// Stops advertising (browsing continues).
    pub fn withdraw(&mut self) {
        let mut conn = lock(&self.conn);
        if let Some((sd, _)) = conn.registered.take() {
            // SAFETY: a subordinate ref, deallocated once, under the lock.
            unsafe { DNSServiceRefDeallocate(sd) };
        }
    }

    /// This device's peer ID.
    pub fn own_id(&self) -> PeerId {
        self.own_id
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let mut conn = lock(&self.conn);
        let keys: Vec<String> = conn.state.services.keys().cloned().collect();
        for key in keys {
            if let Some(service) = conn.state.services.remove(&key) {
                // SAFETY: subordinate refs, deallocated once.
                unsafe {
                    DNSServiceRefDeallocate(service.resolve);
                    if let Some((lookup, _, _)) = service.lookup {
                        DNSServiceRefDeallocate(lookup);
                    }
                }
                drop(service.resolve_ctx);
            }
        }
        // SAFETY: the subordinate refs, then the connection itself.
        unsafe {
            if let Some((sd, _)) = conn.registered.take() {
                DNSServiceRefDeallocate(sd);
            }
            DNSServiceRefDeallocate(conn.browse);
            DNSServiceRefDeallocate(conn.state.main);
        }
    }
}

fn lock(conn: &Mutex<Conn>) -> std::sync::MutexGuard<'_, Conn> {
    conn.lock().unwrap_or_else(|e| e.into_inner())
}

/// Waits for replies on the connection's socket and dispatches them.
fn run(fd: c_int, conn: Arc<Mutex<Conn>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut poll, 1, 250) };
        if ready <= 0 || stop.load(Ordering::SeqCst) {
            continue;
        }
        let guard = lock(&conn);
        // SAFETY: the connection, under the lock; callbacks run here.
        let err = unsafe { DNSServiceProcessResult(guard.state.main) };
        if err != ERR_NO_ERROR {
            guard.state.unavailable(err);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two instances on one Mac find each other through mDNSResponder.
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finds_another_instance() {
        let (a_tx, mut a_rx) = mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel();
        let a_id = PeerId([1; 32]);
        let b_id = PeerId([2; 32]);
        let _a = Discovery::start(a_id, a_tx).expect("browse A");
        let mut b = Discovery::start(b_id, b_tx).expect("browse B");
        b.advertise(&Advertisement {
            peer_id: b_id,
            name: "Bonjour test".into(),
            platform: "macos".into(),
            port: 4242,
            code_open: true,
        })
        .expect("register B");
        let mut renamed = false;
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            tokio::select! {
                event = a_rx.recv() => {
                    let Some(event) = event else { break };
                    if let DiscoveryEvent::Found(found) = &event
                        && found.peer_id == b_id
                    {
                        assert!(found.addrs.iter().all(|a| a.port() == 4242), "{found:?}");
                        if found.name == "Bonjour test" && !renamed {
                            // Seen: now change the name and close the code,
                            // which updates the record in place.
                            assert!(found.code_open);
                            renamed = true;
                            b.advertise(&Advertisement {
                                peer_id: b_id,
                                name: "Renamed".into(),
                                platform: "macos".into(),
                                port: 4242,
                                code_open: false,
                            })
                            .expect("update B");
                            continue;
                        }
                        if found.name == "Renamed" && !found.code_open {
                            return;
                        }
                    }
                    if let DiscoveryEvent::Lost(id) = &event {
                        assert_ne!(*id, b_id, "an update must not look like leaving");
                    }
                    seen.push(format!("A: {event:?}"));
                }
                event = b_rx.recv() => {
                    if let Some(event) = event {
                        seen.push(format!("B: {event:?}"));
                    }
                }
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        panic!("A never found B; events: {seen:#?}");
    }

    #[test]
    fn txt_records_round_trip() {
        let entries = vec![("v", "1".to_owned()), ("n", "Ana's Mac".to_owned())];
        let decoded = decode_txt(&encode_txt(&entries));
        assert_eq!(decoded["v"], "1");
        assert_eq!(decoded["n"], "Ana's Mac");
        // Truncated or malformed records never panic.
        assert!(decode_txt(&[9, b'a']).is_empty());
        assert_eq!(regtype().to_str().unwrap(), "_dexdrop._tcp");
    }
}
