//! Server HTTP + WebSocket sederhana berbasis std::net.

use crate::export;
use crate::game::{Game, HostCmd, Out, Settings, Tx};
use crate::json::{self, Json};
use crate::obj;
use crate::quiz::{self, Question};
use crate::util::{base64, sha1, url_decode};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const INDEX_HTML: &str = include_str!("../static/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const STYLE_CSS: &str = include_str!("../static/style.css");

const MAX_BODY: usize = 12 * 1024 * 1024;
const MAX_GAMES: usize = 2000;

pub struct App {
    pub games: Mutex<HashMap<String, Arc<Game>>>,
    pub next_conn: AtomicU64,
}

struct Req {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

pub fn serve(port: u16, app: Arc<App>) -> io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    // pembersih game yang sudah lama tidak dipakai
    {
        let app = app.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(300));
            let mut games = app.games.lock().unwrap_or_else(|e| e.into_inner());
            games.retain(|_, g| !g.is_stale(Duration::from_secs(6 * 3600)));
        });
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let app = app.clone();
        thread::spawn(move || {
            let _ = handle(stream, app);
        });
    }
    Ok(())
}

fn find_subslice(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn read_request(stream: &mut TcpStream) -> io::Result<Option<Req>> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        if let Some(p) = find_subslice(&buf, b"\r\n\r\n") {
            break p;
        }
        if buf.len() > 32 * 1024 {
            return Ok(None);
        }
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    let (path, qs) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let mut query = HashMap::new();
    for kv in qs.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        query.insert(url_decode(k), url_decode(v));
    }
    let clen: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if clen > MAX_BODY {
        respond(stream, 413, "text/plain; charset=utf-8", &[], b"File terlalu besar (maks 12 MB)")?;
        return Ok(None);
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < clen {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Ok(None);
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(clen);
    Ok(Some(Req { method, path, query, headers, body }))
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        101 => "Switching Protocols",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

fn respond(stream: &mut TcpStream, code: u16, ctype: &str, extra: &[(&str, String)], body: &[u8]) -> io::Result<()> {
    let mut h = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        code,
        status_text(code),
        ctype,
        body.len()
    );
    for (k, v) in extra {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str("\r\n");
    stream.write_all(h.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn json_resp(stream: &mut TcpStream, code: u16, j: &Json) -> io::Result<()> {
    respond(stream, code, "application/json; charset=utf-8", &[], j.dump().as_bytes())
}

fn err_resp(stream: &mut TcpStream, code: u16, msg: &str) -> io::Result<()> {
    json_resp(stream, code, &obj! {"error" => msg})
}

const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

fn handle(mut stream: TcpStream, app: Arc<App>) -> io::Result<()> {
    stream.set_nodelay(true).ok();
    let Some(req) = read_request(&mut stream)? else {
        return Ok(());
    };
    let seg: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    match (req.method.as_str(), seg.as_slice()) {
        ("GET", [""]) | ("GET", ["index.html"]) => {
            respond(&mut stream, 200, "text/html; charset=utf-8", &[], INDEX_HTML.as_bytes())
        }
        ("GET", ["app.js"]) => respond(&mut stream, 200, "text/javascript; charset=utf-8", &[], APP_JS.as_bytes()),
        ("GET", ["style.css"]) => respond(&mut stream, 200, "text/css; charset=utf-8", &[], STYLE_CSS.as_bytes()),
        ("GET", ["favicon.ico"]) => respond(&mut stream, 204, "image/x-icon", &[], b""),
        ("GET", ["api", "template.xlsx"]) => {
            let bytes = quiz::template_xlsx();
            let disp = "attachment; filename=\"template-kuis.xlsx\"".to_string();
            respond(&mut stream, 200, XLSX_MIME, &[("Content-Disposition", disp)], &bytes)
        }
        ("POST", ["api", "parse"]) => match quiz::parse_file(&req.body) {
            Ok(p) => {
                let mut j = p.to_json();
                let title = req
                    .query
                    .get("name")
                    .map(|n| n.rsplit_once('.').map(|(a, _)| a).unwrap_or(n).trim().to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "Kuis Baru".into());
                j.set("title", title);
                json_resp(&mut stream, 200, &j)
            }
            Err(e) => err_resp(&mut stream, 400, &e),
        },
        ("POST", ["api", "games"]) => create_game(&mut stream, &req, &app),
        ("GET", ["api", "games", pin]) => {
            let g = app.games.lock().unwrap_or_else(|e| e.into_inner()).get(*pin).cloned();
            match g {
                Some(g) => json_resp(&mut stream, 200, &g.exists_json()),
                None => json_resp(&mut stream, 404, &obj! {"exists" => false}),
            }
        }
        ("GET", ["api", "games", pin, "export.xlsx"]) => {
            let g = app.games.lock().unwrap_or_else(|e| e.into_inner()).get(*pin).cloned();
            match g {
                Some(g) if req.query.get("token").map(|t| t == &g.host_token).unwrap_or(false) => {
                    let bytes = export::report_xlsx(&g.report());
                    let disp = format!("attachment; filename=\"hasil-kuis-{}.xlsx\"", g.pin);
                    respond(&mut stream, 200, XLSX_MIME, &[("Content-Disposition", disp)], &bytes)
                }
                Some(_) => err_resp(&mut stream, 403, "Token host tidak valid"),
                None => err_resp(&mut stream, 404, "Game tidak ditemukan"),
            }
        }
        ("GET", ["ws"]) => {
            if req.headers.get("upgrade").map(|u| u.to_lowercase() == "websocket").unwrap_or(false) {
                handle_ws(stream, req, app)
            } else {
                err_resp(&mut stream, 400, "Butuh koneksi WebSocket")
            }
        }
        _ => respond(&mut stream, 404, "text/plain; charset=utf-8", &[], b"Tidak ditemukan"),
    }
}

fn create_game(stream: &mut TcpStream, req: &Req, app: &Arc<App>) -> io::Result<()> {
    let body = String::from_utf8_lossy(&req.body).into_owned();
    let j = match json::parse(&body) {
        Ok(j) => j,
        Err(e) => return err_resp(stream, 400, &e),
    };
    let Some(arr) = j.get("questions").and_then(|v| v.as_arr()) else {
        return err_resp(stream, 400, "Daftar soal tidak ada");
    };
    let mut questions: Vec<Question> = Vec::new();
    for q in arr {
        match Question::from_json(q) {
            Ok(q) => questions.push(q),
            Err(e) => return err_resp(stream, 400, &e),
        }
    }
    if let Err(e) = quiz::validate(&mut questions) {
        return err_resp(stream, 400, &e);
    }
    let title: String = j
        .get("title")
        .and_then(|v| v.as_str())
        .map(|t| t.trim().chars().take(100).collect::<String>())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Kuis Tanpa Judul".into());
    let settings = j.get("settings").map(Settings::from_json).unwrap_or_default();

    let mut games = app.games.lock().unwrap_or_else(|e| e.into_inner());
    if games.len() >= MAX_GAMES {
        return err_resp(stream, 503, "Server penuh, coba lagi nanti");
    }
    let mut rng = crate::util::Rng::new();
    let pin = loop {
        let p = format!("{:06}", 100000 + rng.below(900000));
        if !games.contains_key(&p) {
            break p;
        }
    };
    let game = Game::new(pin.clone(), title, questions, settings);
    let token = game.host_token.clone();
    games.insert(pin.clone(), game);
    drop(games);
    json_resp(stream, 200, &obj! {"pin" => pin, "token" => token})
}

// ───────────────────────── WebSocket ─────────────────────────

fn write_frame(w: &mut TcpStream, op: u8, payload: &[u8]) -> io::Result<()> {
    let mut f = Vec::with_capacity(payload.len() + 10);
    f.push(0x80 | op);
    if payload.len() < 126 {
        f.push(payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        f.push(126);
        f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        f.push(127);
        f.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    f.extend_from_slice(payload);
    w.write_all(&f)?;
    w.flush()
}

fn read_message(s: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut msg: Vec<u8> = Vec::new();
    let mut first_op = 0u8;
    loop {
        let mut h = [0u8; 2];
        s.read_exact(&mut h)?;
        let fin = h[0] & 0x80 != 0;
        let op = h[0] & 0x0f;
        let masked = h[1] & 0x80 != 0;
        let mut len = (h[1] & 0x7f) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            s.read_exact(&mut b)?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            s.read_exact(&mut b)?;
            len = u64::from_be_bytes(b);
        }
        if len > (1 << 20) {
            return Err(bad("frame terlalu besar"));
        }
        let mut mask = [0u8; 4];
        if masked {
            s.read_exact(&mut mask)?;
        }
        let mut payload = vec![0u8; len as usize];
        s.read_exact(&mut payload)?;
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        if op >= 8 {
            return Ok((op, payload));
        }
        if op != 0 {
            first_op = op;
        }
        msg.extend_from_slice(&payload);
        if msg.len() > (1 << 20) {
            return Err(bad("pesan terlalu besar"));
        }
        if fin {
            return Ok((first_op, msg));
        }
    }
}

fn handle_ws(mut stream: TcpStream, req: Req, app: Arc<App>) -> io::Result<()> {
    let Some(key) = req.headers.get("sec-websocket-key") else {
        return err_resp(&mut stream, 400, "Sec-WebSocket-Key tidak ada");
    };
    let accept = base64(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    stream.write_all(
        format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n")
            .as_bytes(),
    )?;
    stream.flush()?;
    stream.set_read_timeout(Some(Duration::from_secs(100)))?;

    let (tx, rx) = mpsc::channel::<Out>();
    let mut wstream = stream.try_clone()?;
    let writer = thread::spawn(move || {
        for out in rx {
            let r = match out {
                Out::Text(s) => write_frame(&mut wstream, 1, s.as_bytes()),
                Out::Pong(p) => write_frame(&mut wstream, 10, &p),
                Out::Close => {
                    let _ = write_frame(&mut wstream, 8, &[]);
                    let _ = wstream.shutdown(std::net::Shutdown::Both);
                    break;
                }
            };
            if r.is_err() {
                break;
            }
        }
    });

    let pin = req.query.get("pin").cloned().unwrap_or_default();
    let role = req.query.get("role").cloned().unwrap_or_default();
    let game = app.games.lock().unwrap_or_else(|e| e.into_inner()).get(&pin).cloned();
    let conn = app.next_conn.fetch_add(1, Ordering::Relaxed);

    let Some(game) = game else {
        let _ = tx.send(Out::Text(obj! {"t" => "error", "code" => "nogame", "msg" => "Game dengan PIN itu tidak ditemukan"}.dump()));
        let _ = tx.send(Out::Close);
        drop(tx);
        let _ = writer.join();
        return Ok(());
    };

    let is_host = role == "host";
    if is_host {
        if req.query.get("token").map(|t| t == &game.host_token).unwrap_or(false) {
            game.connect_host(conn, tx.clone());
        } else {
            let _ = tx.send(Out::Text(obj! {"t" => "error", "code" => "nogame", "msg" => "Token host tidak valid"}.dump()));
            let _ = tx.send(Out::Close);
            drop(tx);
            let _ = writer.join();
            return Ok(());
        }
    }

    loop {
        let (op, data) = match read_message(&mut stream) {
            Ok(x) => x,
            Err(_) => break,
        };
        match op {
            1 => {
                let Ok(text) = String::from_utf8(data) else { continue };
                let Ok(m) = json::parse(&text) else { continue };
                client_msg(&game, conn, is_host, &tx, &m);
            }
            8 => break,
            9 => {
                let _ = tx.send(Out::Pong(data));
            }
            _ => {}
        }
    }
    game.disconnect(conn);
    drop(tx);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = writer.join();
    Ok(())
}

fn client_msg(game: &Arc<Game>, conn: u64, is_host: bool, tx: &Tx, m: &Json) {
    let t = m.get("t").and_then(|v| v.as_str()).unwrap_or("");
    let cmd_err = |e: String| {
        let _ = tx.send(Out::Text(obj! {"t" => "error", "code" => "cmd", "msg" => e}.dump()));
    };
    match t {
        "ping" => {
            let _ = tx.send(Out::Text(obj! {"t" => "pong"}.dump()));
        }
        "join" if !is_host => {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let pid = m.get("pid").and_then(|v| v.as_str());
            if let Err(e) = game.join(conn, tx.clone(), name, pid) {
                let _ = tx.send(Out::Text(obj! {"t" => "join_error", "msg" => e}.dump()));
            }
        }
        "answer" if !is_host => {
            let choices: Vec<usize> = m
                .get("choices")
                .and_then(|v| v.as_arr())
                .map(|a| a.iter().filter_map(|c| c.as_u32().map(|n| n as usize)).collect())
                .unwrap_or_default();
            game.answer(conn, choices);
        }
        "start" | "next" | "skip" | "end" | "reset" | "add_time" | "kick" | "lock" if is_host => {
            let cmd = match t {
                "start" => HostCmd::Start,
                "next" => HostCmd::Next,
                "skip" => HostCmd::Skip,
                "end" => HostCmd::End,
                "reset" => HostCmd::Reset,
                "add_time" => HostCmd::AddTime(m.get("secs").and_then(|v| v.as_u32()).unwrap_or(10)),
                "kick" => HostCmd::Kick(m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string()),
                _ => HostCmd::Lock(m.get("locked").and_then(|v| v.as_bool()).unwrap_or(false)),
            };
            if let Err(e) = game.host_cmd(cmd) {
                cmd_err(e);
            }
        }
        _ => {}
    }
}
