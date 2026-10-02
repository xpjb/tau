//! Private loopback impairment, shared by the native-link and pressure tests.
//! No host routes/qdisc, external servers, real accounts or relay traffic.
use std::{net::SocketAddr, sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering}}, time::Duration};
use super::native_trace;
use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::{TcpListener,TcpStream,UdpSocket},sync::Mutex};

#[derive(Clone,Copy,Debug)]
pub struct Profile {
    pub name:&'static str,
    pub bytes_per_second:u64,
    pub latency_ms:u64,
    pub jitter_ms:u64,
    pub loss_per_10k:u64,
    pub stall_every:u64,
    pub stall_ms:u64,
    pub queue_limit_ms:u64,
}
impl Profile {
    pub const NORMAL:Self=Self {name:"normal",bytes_per_second:4*1024*1024,latency_ms:2,jitter_ms:2,loss_per_10k:0,stall_every:0,stall_ms:0,queue_limit_ms:650};
    pub const DODGY:Self=Self {name:"dodgy",bytes_per_second:192*1024,latency_ms:35,jitter_ms:45,loss_per_10k:300,stall_every:67,stall_ms:180,queue_limit_ms:650};
    const LEGACY:Self=Self {name:"legacy",bytes_per_second:64*1024,latency_ms:35,jitter_ms:0,loss_per_10k:0,stall_every:0,stall_ms:0,queue_limit_ms:u64::MAX};
}
/// Stateless seeded choices. Packet scheduling itself is still OS-dependent;
/// a seed repeats the fault/workload configuration, not an exact packet trace.
pub fn draw(seed:u64,serial:u64)->u64 {
    let mut x=seed.wrapping_add(serial.wrapping_mul(0x9e3779b97f4a7c15));
    x=(x^(x>>30)).wrapping_mul(0xbf58476d1ce4e5b9);x=(x^(x>>27)).wrapping_mul(0x94d049bb133111eb);x^(x>>31)
}
pub struct Link {
    next:[Mutex<tokio::time::Instant>;2],
    seed:u64,
    pub profile:Profile,
    pub blackhole:AtomicBool,
    pub udp_blackhole:AtomicBool,
    pub scheduled:AtomicU64,
    pub udp:AtomicU64,
    pub dropped:AtomicU64,
    pub queue_dropped:AtomicU64,
    pub blackholed:AtomicU64,
    pub stalls:AtomicU64,
    pub tcp_connections:AtomicU64,
    pub tcp_bytes:AtomicU64,
    pub udp_bytes:AtomicU64,
    pub max_scheduled_delay_us:AtomicU64,
}
impl Link {
    pub fn new()->Arc<Self> {Self::profile(Profile::LEGACY,0)}
    pub fn profile(profile:Profile,seed:u64)->Arc<Self> {Arc::new(Self {
        next:[Mutex::new(tokio::time::Instant::now()),Mutex::new(tokio::time::Instant::now())],seed,profile,
        blackhole:AtomicBool::new(false),udp_blackhole:AtomicBool::new(false),scheduled:AtomicU64::new(0),udp:AtomicU64::new(0),dropped:AtomicU64::new(0),queue_dropped:AtomicU64::new(0),blackholed:AtomicU64::new(0),stalls:AtomicU64::new(0),tcp_connections:AtomicU64::new(0),tcp_bytes:AtomicU64::new(0),udp_bytes:AtomicU64::new(0),max_scheduled_delay_us:AtomicU64::new(0),
    })}
    async fn wait(&self,direction:usize,bytes:usize,datagram:bool)->bool {
        let serial=self.scheduled.fetch_add(1,Ordering::Relaxed)+1;
        let jitter=draw(self.seed^0x5151,serial)%(self.profile.jitter_ms+1);
        let stall=if self.profile.stall_every>0 && serial%self.profile.stall_every==0 {
            self.stalls.fetch_add(1,Ordering::Relaxed);self.profile.stall_ms
        } else {0};
        let at={let mut next=self.next[direction].lock().await;
            let now=tokio::time::Instant::now();
            let at=(*next).max(now+Duration::from_millis(self.profile.latency_ms+jitter))
                +Duration::from_secs_f64(bytes as f64/self.profile.bytes_per_second as f64)+Duration::from_millis(stall);
            if datagram && at.duration_since(now).as_millis()>u128::from(self.profile.queue_limit_ms) {
                self.queue_dropped.fetch_add(1,Ordering::Relaxed);self.dropped.fetch_add(1,Ordering::Relaxed);return false;
            }
            self.max_scheduled_delay_us.fetch_max(at.duration_since(now).as_micros() as u64,Ordering::Relaxed);
            *next=at;at
        };
        tokio::time::sleep_until(at).await;
        if datagram && (self.blackhole.load(Ordering::Relaxed) || self.udp_blackhole.load(Ordering::Relaxed)) {
            self.dropped.fetch_add(1,Ordering::Relaxed);self.blackholed.fetch_add(1,Ordering::Relaxed);return false;
        }
        // TCP backpressure instead of dropping individual stream bytes (which
        // would corrupt the protocol, not simulate a bad TCP link).
        while !datagram && self.blackhole.load(Ordering::Relaxed) {tokio::time::sleep(Duration::from_millis(10)).await;}
        true
    }
    pub fn report(&self)->serde_json::Value {
        let load=|a:&AtomicU64|a.load(Ordering::Relaxed);
        serde_json::json!({"profile":self.profile.name,"seed":self.seed,"bytes_per_second":self.profile.bytes_per_second,
            "base_latency_ms":self.profile.latency_ms,"jitter_ms":self.profile.jitter_ms,"loss_per_10k":self.profile.loss_per_10k,
            "queue_limit_ms":self.profile.queue_limit_ms,"udp_datagrams":load(&self.udp),"dropped":load(&self.dropped),
            "queue_dropped":load(&self.queue_dropped),"blackholed":load(&self.blackholed),"stalls":load(&self.stalls),
            "tcp_connections":load(&self.tcp_connections),"tcp_bytes":load(&self.tcp_bytes),"udp_bytes":load(&self.udp_bytes),
            "max_scheduled_delay_ms":load(&self.max_scheduled_delay_us) as f64/1000.})
    }
}
pub struct Proxy {pub address:SocketAddr,tasks:Vec<tokio::task::JoinHandle<()>>}
impl Drop for Proxy {fn drop(&mut self) {for task in &self.tasks {task.abort();}}}
impl Proxy {
    pub async fn new(alias:&str,tcp:SocketAddr,udp:SocketAddr,link:Arc<Link>)->Self {
        let listener=TcpListener::bind(format!("{alias}:0")).await.unwrap();let address=listener.local_addr().unwrap();
        let control=link.clone();
        let tcp_task=tokio::spawn(async move {
            let mut connections=tokio::task::JoinSet::new();
            loop {tokio::select! {
                incoming=listener.accept()=>{
                    let (socket,_)=incoming.unwrap();let link=control.clone();
                    link.tcp_connections.fetch_add(1,Ordering::Relaxed);
                    connections.spawn(async move {
                        let Ok(peer)=TcpStream::connect(tcp).await else {return;};
                        let (a,b)=socket.into_split();let(c,d)=peer.into_split();
                        async fn copy(mut from:tokio::net::tcp::OwnedReadHalf,mut to:tokio::net::tcp::OwnedWriteHalf,link:Arc<Link>,direction:usize) {
                            let mut buf=[0;4096];while let Ok(n)=from.read(&mut buf).await {
                                if n==0 {break;}link.wait(direction,n,false).await;
                                if to.write_all(&buf[..n]).await.is_err() {break;}link.tcp_bytes.fetch_add(n as u64,Ordering::Relaxed);
                            }
                        }
                        tokio::select! {_=copy(a,d,link.clone(),0)=>{},_=copy(c,b,link,1)=>{}}
                    });
                }
                _=connections.join_next(),if !connections.is_empty()=>{}
            }}
        });
        // Same advertised UDP port on private aliases; no WebSocket termination
        // or synthetic Pong. Both clients share the physical link's FIFOs.
        let socket=Arc::new(UdpSocket::bind(format!("{alias}:{}",udp.port())).await.unwrap());
        let udp_task=tokio::spawn(async move {
            let mut client=None;let mut buf=[0;65536];let mut packets=tokio::task::JoinSet::new();
            loop {tokio::select! {
                received=socket.recv_from(&mut buf)=>{
                    let (n,from)=received.unwrap();let direction=usize::from(from==udp);
                    let target=if direction==1 {let Some(client)=client else {continue;};client} else {client=Some(from);udp};
                    let serial=link.udp.fetch_add(1,Ordering::Relaxed)+1;
                    native_trace::packet("received",direction,n);
                    if link.blackhole.load(Ordering::Relaxed) || link.udp_blackhole.load(Ordering::Relaxed) {
                        native_trace::packet("blackholed",direction,n);
                        link.dropped.fetch_add(1,Ordering::Relaxed);link.blackholed.fetch_add(1,Ordering::Relaxed);continue;
                    }
                    let lost=if link.profile.name=="legacy" {serial%23==0} else {draw(link.seed,serial)%10_000<link.profile.loss_per_10k};
                    if lost || packets.len()>=512 {native_trace::packet("lost",direction,n);link.dropped.fetch_add(1,Ordering::Relaxed);continue;}
                    let bytes=buf[..n].to_vec();let socket=socket.clone();let link=link.clone();
                    packets.spawn(async move {
                        if !link.wait(direction,n,true).await {native_trace::packet("discarded",direction,n);return;}
                        if socket.send_to(&bytes,target).await.is_ok() {native_trace::packet("forwarded",direction,n);link.udp_bytes.fetch_add(n as u64,Ordering::Relaxed);}
                    });
                }
                _=packets.join_next(),if !packets.is_empty()=>{}
            }}
        });
        Self {address,tasks:vec![tcp_task,udp_task]}
    }
}
