mod export;
mod game;
mod json;
mod quiz;
mod server;
mod util;
mod xlsx;
mod zipx;

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

fn local_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    game::start_scheduler();
    let app = Arc::new(server::App {
        games: Mutex::new(HashMap::new()),
        next_conn: AtomicU64::new(1),
    });
    println!();
    println!("  ╔══════════════════════════════════════════╗");
    println!("  ║          KUIS RUST  ·  siap dipakai       ║");
    println!("  ╚══════════════════════════════════════════╝");
    println!("  Di komputer ini : http://localhost:{port}");
    if let Some(ip) = local_ip() {
        println!("  Dari HP / laptop lain (satu Wi-Fi): http://{ip}:{port}");
    }
    println!("  Tekan Ctrl+C untuk berhenti.\n");
    if let Err(e) = server::serve(port, app) {
        eprintln!("Gagal menjalankan server di port {port}: {e}");
        std::process::exit(1);
    }
}
