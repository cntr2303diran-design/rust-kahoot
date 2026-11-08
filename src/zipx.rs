//! Pembaca ZIP (dengan inflate/DEFLATE buatan sendiri) dan penulis ZIP (tanpa kompresi).

use crate::util::crc32;

const MAX_OUT: usize = 64 * 1024 * 1024;

// ───────────────────────── INFLATE ─────────────────────────

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn bits(&mut self, need: u32) -> Result<u32, String> {
        let mut val = self.buf;
        while self.cnt < need {
            if self.pos >= self.data.len() {
                return Err("Data terkompresi terpotong".into());
            }
            val |= (self.data[self.pos] as u32) << self.cnt;
            self.pos += 1;
            self.cnt += 8;
        }
        self.buf = if need >= 32 { 0 } else { val >> need };
        self.cnt -= need;
        Ok(val & ((1u32 << need) - 1))
    }
}

struct Huff {
    count: [u16; 16],
    symbol: Vec<u16>,
}

fn build(lengths: &[u16]) -> Huff {
    let mut count = [0u16; 16];
    for &l in lengths {
        count[l as usize] += 1;
    }
    let mut offs = [0u16; 16];
    for len in 1..15 {
        offs[len + 1] = offs[len] + count[len];
    }
    let mut symbol = vec![0u16; lengths.len()];
    for (sym, &l) in lengths.iter().enumerate() {
        if l != 0 {
            symbol[offs[l as usize] as usize] = sym as u16;
            offs[l as usize] += 1;
        }
    }
    Huff { count, symbol }
}

fn decode(b: &mut Bits, h: &Huff) -> Result<u16, String> {
    let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
    for len in 1..16 {
        code |= b.bits(1)? as i32;
        let count = h.count[len] as i32;
        if code - count < first {
            let idx = (index + (code - first)) as usize;
            return h.symbol.get(idx).copied().ok_or_else(|| "Kode Huffman rusak".to_string());
        }
        index += count;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    Err("Kode Huffman rusak".into())
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u16; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u16; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn codes(b: &mut Bits, out: &mut Vec<u8>, lc: &Huff, dc: &Huff) -> Result<(), String> {
    loop {
        let sym = decode(b, lc)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            if s >= 29 {
                return Err("Simbol panjang tidak valid".into());
            }
            let len = LBASE[s] as usize + b.bits(LEXT[s] as u32)? as usize;
            let ds = decode(b, dc)? as usize;
            if ds >= 30 {
                return Err("Simbol jarak tidak valid".into());
            }
            let dist = DBASE[ds] as usize + b.bits(DEXT[ds] as u32)? as usize;
            if dist > out.len() {
                return Err("Jarak mundur melebihi data".into());
            }
            for _ in 0..len {
                let v = out[out.len() - dist];
                out.push(v);
            }
        }
        if out.len() > MAX_OUT {
            return Err("Data hasil dekompresi terlalu besar".into());
        }
    }
}

pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut b = Bits { data, pos: 0, buf: 0, cnt: 0 };
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.buf = 0;
                b.cnt = 0;
                if b.pos + 4 > data.len() {
                    return Err("Blok stored terpotong".into());
                }
                let len = u16::from_le_bytes([data[b.pos], data[b.pos + 1]]) as usize;
                b.pos += 4;
                if b.pos + len > data.len() {
                    return Err("Blok stored terpotong".into());
                }
                out.extend_from_slice(&data[b.pos..b.pos + len]);
                b.pos += len;
            }
            1 => {
                let mut l = [0u16; 288];
                for (i, v) in l.iter_mut().enumerate() {
                    *v = match i {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let lc = build(&l);
                let dc = build(&[5u16; 30]);
                codes(&mut b, &mut out, &lc, &dc)?;
            }
            2 => {
                let nlen = b.bits(5)? as usize + 257;
                let ndist = b.bits(5)? as usize + 1;
                let ncode = b.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("Header dinamis tidak valid".into());
                }
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut lengths = [0u16; 320];
                for &o in ORDER.iter().take(ncode) {
                    lengths[o] = b.bits(3)? as u16;
                }
                let lencode = build(&lengths[..19]);
                let mut idx = 0;
                let mut lens = [0u16; 320];
                while idx < nlen + ndist {
                    let sym = decode(&mut b, &lencode)?;
                    if sym < 16 {
                        lens[idx] = sym;
                        idx += 1;
                    } else {
                        let (prev, rep) = match sym {
                            16 => {
                                if idx == 0 {
                                    return Err("Pengulangan tanpa panjang sebelumnya".into());
                                }
                                (lens[idx - 1], 3 + b.bits(2)? as usize)
                            }
                            17 => (0, 3 + b.bits(3)? as usize),
                            _ => (0, 11 + b.bits(7)? as usize),
                        };
                        if idx + rep > nlen + ndist {
                            return Err("Pengulangan melebihi batas".into());
                        }
                        for _ in 0..rep {
                            lens[idx] = prev;
                            idx += 1;
                        }
                    }
                }
                let lc = build(&lens[..nlen]);
                let dc = build(&lens[nlen..nlen + ndist]);
                codes(&mut b, &mut out, &lc, &dc)?;
            }
            _ => return Err("Tipe blok DEFLATE tidak valid".into()),
        }
        if last == 1 {
            break;
        }
    }
    Ok(out)
}

// ───────────────────────── ZIP READ ─────────────────────────

struct Entry {
    name: String,
    method: u16,
    csize: usize,
    offset: usize,
}

pub struct Zip<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn u16le(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn u32le(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl<'a> Zip<'a> {
    pub fn open(data: &'a [u8]) -> Result<Zip<'a>, String> {
        let bad = || "File ZIP/XLSX rusak".to_string();
        if data.len() < 22 {
            return Err(bad());
        }
        let min = data.len().saturating_sub(22 + 65535);
        let mut eocd = None;
        let mut i = data.len() - 22;
        loop {
            if u32le(data, i) == Some(0x06054b50) {
                eocd = Some(i);
                break;
            }
            if i == min {
                break;
            }
            i -= 1;
        }
        let e = eocd.ok_or_else(bad)?;
        let count = u16le(data, e + 10).ok_or_else(bad)? as usize;
        let mut p = u32le(data, e + 16).ok_or_else(bad)? as usize;
        let mut entries = Vec::new();
        for _ in 0..count {
            if u32le(data, p) != Some(0x02014b50) {
                return Err(bad());
            }
            let method = u16le(data, p + 10).ok_or_else(bad)?;
            let csize = u32le(data, p + 20).ok_or_else(bad)? as usize;
            let nlen = u16le(data, p + 28).ok_or_else(bad)? as usize;
            let elen = u16le(data, p + 30).ok_or_else(bad)? as usize;
            let clen = u16le(data, p + 32).ok_or_else(bad)? as usize;
            let offset = u32le(data, p + 42).ok_or_else(bad)? as usize;
            let name = data.get(p + 46..p + 46 + nlen).ok_or_else(bad)?;
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                method,
                csize,
                offset,
            });
            p += 46 + nlen + elen + clen;
        }
        Ok(Zip { data, entries })
    }

    pub fn has(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    pub fn get(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let bad = || "File ZIP/XLSX rusak".to_string();
        let Some(e) = self.entries.iter().find(|e| e.name == name) else {
            return Ok(None);
        };
        if u32le(self.data, e.offset) != Some(0x04034b50) {
            return Err(bad());
        }
        let nlen = u16le(self.data, e.offset + 26).ok_or_else(bad)? as usize;
        let elen = u16le(self.data, e.offset + 28).ok_or_else(bad)? as usize;
        let start = e.offset + 30 + nlen + elen;
        let raw = self.data.get(start..start + e.csize).ok_or_else(bad)?;
        match e.method {
            0 => Ok(Some(raw.to_vec())),
            8 => Ok(Some(inflate(raw)?)),
            m => Err(format!("Metode kompresi ZIP {m} tidak didukung")),
        }
    }
}

// ───────────────────────── ZIP WRITE ─────────────────────────

pub fn write_zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let nb = name.as_bytes();
        // local header
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x0021u16.to_le_bytes()); // date 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(nb);
        out.extend_from_slice(data);
        // central header
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x0021u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(nb);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
