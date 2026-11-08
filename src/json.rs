//! JSON minimal (parser + serializer) tanpa dependensi.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, k: &str) -> Option<&Json> {
        if let Json::Obj(v) = self {
            v.iter().find(|(kk, _)| kk == k).map(|(_, v)| v)
        } else {
            None
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        if let Json::Num(n) = self { Some(*n) } else { None }
    }
    pub fn as_u32(&self) -> Option<u32> {
        self.as_f64().filter(|n| *n >= 0.0 && *n <= u32::MAX as f64).map(|n| n as u32)
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Json::Bool(b) = self { Some(*b) } else { None }
    }
    pub fn as_arr(&self) -> Option<&Vec<Json>> {
        if let Json::Arr(a) = self { Some(a) } else { None }
    }

    /// Set / ganti field pada object.
    pub fn set(&mut self, k: &str, v: impl Into<Json>) {
        if let Json::Obj(items) = self {
            let v = v.into();
            if let Some(slot) = items.iter_mut().find(|(kk, _)| kk == k) {
                slot.1 = v;
            } else {
                items.push((k.to_string(), v));
            }
        }
    }

    pub fn dump(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if !n.is_finite() {
                    out.push_str("null");
                } else if n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else {
                    out.push_str(&format!("{}", n));
                }
            }
            Json::Str(s) => write_str(s, out),
            Json::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(o) => {
                out.push('{');
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<&str> for Json {
    fn from(s: &str) -> Json { Json::Str(s.to_string()) }
}
impl From<String> for Json {
    fn from(s: String) -> Json { Json::Str(s) }
}
impl From<&String> for Json {
    fn from(s: &String) -> Json { Json::Str(s.clone()) }
}
impl From<bool> for Json {
    fn from(b: bool) -> Json { Json::Bool(b) }
}
impl From<f64> for Json {
    fn from(n: f64) -> Json { Json::Num(n) }
}
impl From<u32> for Json {
    fn from(n: u32) -> Json { Json::Num(n as f64) }
}
impl From<u64> for Json {
    fn from(n: u64) -> Json { Json::Num(n as f64) }
}
impl From<usize> for Json {
    fn from(n: usize) -> Json { Json::Num(n as f64) }
}
impl From<i64> for Json {
    fn from(n: i64) -> Json { Json::Num(n as f64) }
}
impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(v: Vec<T>) -> Json { Json::Arr(v.into_iter().map(Into::into).collect()) }
}
impl<T: Clone + Into<Json>> From<&Vec<T>> for Json {
    fn from(v: &Vec<T>) -> Json { Json::Arr(v.iter().cloned().map(Into::into).collect()) }
}
impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(o: Option<T>) -> Json {
        match o {
            Some(v) => v.into(),
            None => Json::Null,
        }
    }
}

/// Membuat object JSON: `obj!{"a" => 1, "b" => "x"}`
#[macro_export]
macro_rules! obj {
    ($($k:expr => $v:expr),* $(,)?) => {
        $crate::json::Json::Obj(vec![$(($k.to_string(), $crate::json::Json::from($v))),*])
    };
}

pub fn parse(s: &str) -> Result<Json, String> {
    let mut p = Parser { b: s.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err("JSON: sisa karakter setelah nilai".into());
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 64 {
            return Err("JSON terlalu dalam".into());
        }
        match self.peek() {
            None => Err("JSON berakhir tiba-tiba".into()),
            Some(b'{') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(items));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return Err("JSON: ':' diharapkan".into());
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    items.push((k, v));
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(items));
                        }
                        _ => return Err("JSON: ',' atau '}' diharapkan".into()),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err("JSON: ',' atau ']' diharapkan".into()),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("");
                t.parse::<f64>().map(Json::Num).map_err(|_| "JSON: angka tidak valid".to_string())
            }
        }
    }

    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err("JSON: literal tidak valid".into())
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.i + 4 > self.b.len() {
            return Err("JSON: escape \\u terpotong".into());
        }
        let t = std::str::from_utf8(&self.b[self.i..self.i + 4]).map_err(|_| "JSON: hex salah")?;
        let v = u32::from_str_radix(t, 16).map_err(|_| "JSON: hex salah".to_string())?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err("JSON: string diharapkan".into());
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i).ok_or("JSON: string tidak ditutup")?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or("JSON: escape terpotong")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp)
                                && self.b[self.i..].starts_with(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                }
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err("JSON: escape tidak dikenal".into()),
                    }
                }
                c => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}
