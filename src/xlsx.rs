//! Baca file .xlsx (sheet pertama → grid teks) dan tulis workbook .xlsx sederhana.

use crate::zipx::{write_zip, Zip};
use std::collections::{BTreeMap, HashMap};

// ───────────────────────── XML mini ─────────────────────────

enum Tok {
    Open(String, Vec<(String, String)>, bool),
    Close(String),
    Text(String),
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        if let Some(end) = rest.find(';') {
            let ent = &rest[1..end];
            let rep = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                e if e.starts_with("#x") || e.starts_with("#X") => {
                    u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32)
                }
                e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = rep {
                out.push(c);
                rest = &rest[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

fn tokenize(s: &str) -> Vec<Tok> {
    let b = s.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            if s[i..].starts_with("<?") {
                i = s[i..].find("?>").map(|p| i + p + 2).unwrap_or(b.len());
            } else if s[i..].starts_with("<!--") {
                i = s[i..].find("-->").map(|p| i + p + 3).unwrap_or(b.len());
            } else if s[i..].starts_with("<![CDATA[") {
                let st = i + 9;
                let en = s[st..].find("]]>").map(|p| st + p).unwrap_or(b.len());
                toks.push(Tok::Text(s[st..en].to_string()));
                i = (en + 3).min(b.len());
            } else {
                // cari '>' dengan menghormati tanda kutip
                let mut j = i + 1;
                let mut quote = 0u8;
                while j < b.len() {
                    let c = b[j];
                    if quote != 0 {
                        if c == quote {
                            quote = 0;
                        }
                    } else if c == b'"' || c == b'\'' {
                        quote = c;
                    } else if c == b'>' {
                        break;
                    }
                    j += 1;
                }
                let inner = &s[i + 1..j.min(b.len())];
                i = (j + 1).min(b.len());
                if let Some(name) = inner.strip_prefix('/') {
                    toks.push(Tok::Close(name.trim().to_string()));
                } else if !inner.starts_with('!') {
                    let selfc = inner.ends_with('/');
                    let inner = inner.trim_end_matches('/');
                    let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
                    let name = inner[..name_end].to_string();
                    toks.push(Tok::Open(name, parse_attrs(&inner[name_end..]), selfc));
                }
            }
        } else {
            let end = s[i..].find('<').map(|p| i + p).unwrap_or(b.len());
            toks.push(Tok::Text(decode_entities(&s[i..end])));
            i = end;
        }
    }
    toks
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next() else { break };
        if q != '"' && q != '\'' {
            break;
        }
        let Some(end) = after[1..].find(q) else { break };
        out.push((key, decode_entities(&after[1..1 + end])));
        rest = after[end + 2..].trim_start();
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], k: &str) -> Option<&'a str> {
    attrs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str())
}

// ───────────────────────── Baca XLSX ─────────────────────────

fn col_index(letters: &str) -> usize {
    letters
        .bytes()
        .filter(|b| b.is_ascii_alphabetic())
        .fold(0usize, |acc, b| acc * 26 + (b.to_ascii_uppercase() - b'A' + 1) as usize)
        .saturating_sub(1)
}

fn parse_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits: String = r.chars().skip(letters.len()).collect();
    let row = digits.parse::<usize>().ok()?;
    if letters.is_empty() || row == 0 {
        return None;
    }
    Some((col_index(&letters), row))
}

fn norm_number(v: &str) -> String {
    let t = v.trim();
    match t.parse::<f64>() {
        Ok(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
        Ok(_) => t.to_string(),
        Err(_) => t.to_string(),
    }
}

fn shared_strings(xml: &str) -> Vec<String> {
    let toks = tokenize(xml);
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut in_t = false;
    let mut in_ph = false;
    for t in toks {
        match t {
            Tok::Open(n, _, selfc) => match n.as_str() {
                "si" => {
                    if selfc {
                        out.push(String::new());
                    } else {
                        cur = Some(String::new());
                    }
                }
                "rPh" => in_ph = !selfc,
                "t" => in_t = !selfc && !in_ph,
                _ => {}
            },
            Tok::Close(n) => match n.as_str() {
                "si" => out.push(cur.take().unwrap_or_default()),
                "rPh" => in_ph = false,
                "t" => in_t = false,
                _ => {}
            },
            Tok::Text(tx) => {
                if in_t {
                    if let Some(c) = cur.as_mut() {
                        c.push_str(&tx);
                    }
                }
            }
        }
    }
    out
}

/// Baca sheet pertama dari file xlsx menjadi grid teks (indeks baris = nomor baris Excel - 1).
pub fn read_first_sheet(data: &[u8]) -> Result<(String, Vec<Vec<String>>), String> {
    let zip = Zip::open(data)?;
    if !zip.has("xl/workbook.xml") {
        return Err("Bukan file .xlsx yang valid (xl/workbook.xml tidak ada)".into());
    }
    let wb_xml = String::from_utf8_lossy(&zip.get("xl/workbook.xml")?.unwrap_or_default()).into_owned();
    let mut sheet_name = String::from("Sheet1");
    let mut rid = None;
    for t in tokenize(&wb_xml) {
        if let Tok::Open(n, a, _) = t {
            if n == "sheet" {
                sheet_name = attr(&a, "name").unwrap_or("Sheet1").to_string();
                rid = attr(&a, "r:id").map(|s| s.to_string());
                break;
            }
        }
    }
    let mut target = String::from("xl/worksheets/sheet1.xml");
    if let (Some(rid), Some(rels)) = (rid, zip.get("xl/_rels/workbook.xml.rels")?) {
        let rels = String::from_utf8_lossy(&rels).into_owned();
        for t in tokenize(&rels) {
            if let Tok::Open(n, a, _) = t {
                if n == "Relationship" && attr(&a, "Id") == Some(rid.as_str()) {
                    if let Some(tg) = attr(&a, "Target") {
                        target = match tg.strip_prefix('/') {
                            Some(abs) => abs.to_string(),
                            None => format!("xl/{}", tg),
                        };
                    }
                    break;
                }
            }
        }
    }
    let sheet_xml = zip
        .get(&target)?
        .ok_or_else(|| format!("Sheet '{}' tidak ditemukan di dalam file", sheet_name))?;
    let sheet_xml = String::from_utf8_lossy(&sheet_xml).into_owned();
    let shared = match zip.get("xl/sharedStrings.xml")? {
        Some(b) => shared_strings(&String::from_utf8_lossy(&b)),
        None => Vec::new(),
    };

    let toks = tokenize(&sheet_xml);
    let mut cells: BTreeMap<usize, BTreeMap<usize, String>> = BTreeMap::new();
    let mut i = 0;
    let mut max_col = 0usize;
    let mut auto_row = 0usize;
    let mut auto_col = 0usize;
    while i < toks.len() {
        if let Tok::Open(name, attrs, selfc) = &toks[i] {
            if name == "row" {
                auto_row = attr(attrs, "r").and_then(|r| r.parse().ok()).unwrap_or(auto_row + 1);
                auto_col = 0;
            } else if name == "c" {
                let (col, row) = attr(attrs, "r")
                    .and_then(parse_ref)
                    .unwrap_or((auto_col, auto_row.max(1)));
                auto_col = col + 1;
                let ty = attr(attrs, "t").unwrap_or("n").to_string();
                let mut v = String::new();
                let mut inline = String::new();
                if !*selfc {
                    let mut j = i + 1;
                    let mut cur = String::new();
                    while j < toks.len() {
                        match &toks[j] {
                            Tok::Close(n) if n == "c" => break,
                            Tok::Close(_) => cur.clear(),
                            Tok::Open(n, _, sc) => cur = if *sc { String::new() } else { n.clone() },
                            Tok::Text(tx) => {
                                if cur == "v" {
                                    v.push_str(tx);
                                } else if cur == "t" {
                                    inline.push_str(tx);
                                }
                            }
                        }
                        j += 1;
                    }
                    i = j;
                }
                let val = match ty.as_str() {
                    "s" => v.trim().parse::<usize>().ok().and_then(|ix| shared.get(ix).cloned()).unwrap_or_default(),
                    "inlineStr" => inline,
                    "str" => v,
                    "b" => if v.trim() == "1" { "TRUE".into() } else { "FALSE".into() },
                    "e" => String::new(),
                    _ => norm_number(&v),
                };
                if !val.trim().is_empty() && row <= 5000 && col < 200 {
                    cells.entry(row).or_default().insert(col, val);
                    max_col = max_col.max(col);
                }
            }
        }
        i += 1;
    }
    let max_row = cells.keys().next_back().copied().unwrap_or(0);
    let mut grid = vec![vec![String::new(); max_col + 1]; max_row];
    for (r, cols) in cells {
        for (c, v) in cols {
            grid[r - 1][c] = v;
        }
    }
    Ok((sheet_name, grid))
}

// ───────────────────────── Tulis XLSX ─────────────────────────

pub enum CellV {
    Empty,
    Str(String),
    Num(f64),
}

pub struct Cell {
    pub v: CellV,
    pub style: u8,
}

pub const ST_NORMAL: u8 = 0;
pub const ST_HEADER: u8 = 1;
pub const ST_WRAP: u8 = 2;
pub const ST_BOLD: u8 = 3;
pub const ST_TITLE: u8 = 4;

impl Cell {
    pub fn e() -> Cell { Cell { v: CellV::Empty, style: 0 } }
    pub fn s(t: impl Into<String>) -> Cell { Cell { v: CellV::Str(t.into()), style: ST_NORMAL } }
    pub fn n(x: f64) -> Cell { Cell { v: CellV::Num(x), style: ST_NORMAL } }
    pub fn hdr(t: impl Into<String>) -> Cell { Cell { v: CellV::Str(t.into()), style: ST_HEADER } }
    pub fn wrap(t: impl Into<String>) -> Cell { Cell { v: CellV::Str(t.into()), style: ST_WRAP } }
    pub fn bold(t: impl Into<String>) -> Cell { Cell { v: CellV::Str(t.into()), style: ST_BOLD } }
    pub fn title(t: impl Into<String>) -> Cell { Cell { v: CellV::Str(t.into()), style: ST_TITLE } }
}

pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
    pub widths: Vec<f64>,
    /// Jumlah baris atas yang dibekukan (0 = tidak).
    pub freeze_rows: u32,
    /// Pasang autofilter pada baris header ini (indeks 0), jika Some.
    pub filter_header_row: Option<usize>,
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

pub fn col_name(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn clean_sheet_name(n: &str, idx: usize) -> String {
    let c: String = n
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { ' ' } else { c })
        .take(31)
        .collect();
    let c = c.trim().to_string();
    if c.is_empty() { format!("Sheet{}", idx + 1) } else { c }
}

fn sheet_xml(sh: &Sheet) -> String {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    );
    x.push_str("<sheetViews><sheetView workbookViewId=\"0\">");
    if sh.freeze_rows > 0 {
        x.push_str(&format!(
            "<pane ySplit=\"{n}\" topLeftCell=\"A{m}\" activePane=\"bottomLeft\" state=\"frozen\"/>",
            n = sh.freeze_rows,
            m = sh.freeze_rows + 1
        ));
    }
    x.push_str("</sheetView></sheetViews>");
    if !sh.widths.is_empty() {
        x.push_str("<cols>");
        for (i, w) in sh.widths.iter().enumerate() {
            x.push_str(&format!("<col min=\"{0}\" max=\"{0}\" width=\"{1}\" customWidth=\"1\"/>", i + 1, w));
        }
        x.push_str("</cols>");
    }
    x.push_str("<sheetData>");
    let mut max_cols = 0;
    for (ri, row) in sh.rows.iter().enumerate() {
        x.push_str(&format!("<row r=\"{}\">", ri + 1));
        for (ci, c) in row.iter().enumerate() {
            max_cols = max_cols.max(ci + 1);
            let r = format!("{}{}", col_name(ci), ri + 1);
            match &c.v {
                CellV::Empty => {
                    if c.style != 0 {
                        x.push_str(&format!("<c r=\"{}\" s=\"{}\"/>", r, c.style));
                    }
                }
                CellV::Str(s) => x.push_str(&format!(
                    "<c r=\"{}\" s=\"{}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    r, c.style, esc(s)
                )),
                CellV::Num(n) => x.push_str(&format!("<c r=\"{}\" s=\"{}\"><v>{}</v></c>", r, c.style, n)),
            }
        }
        x.push_str("</row>");
    }
    x.push_str("</sheetData>");
    if let Some(h) = sh.filter_header_row {
        if max_cols > 0 && sh.rows.len() > h {
            x.push_str(&format!(
                "<autoFilter ref=\"A{}:{}{}\"/>",
                h + 1,
                col_name(max_cols - 1),
                sh.rows.len()
            ));
        }
    }
    x.push_str("</worksheet>");
    x
}

const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"4\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><color rgb=\"FFFFFFFF\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"16\"/><color rgb=\"FF46178F\"/><name val=\"Calibri\"/></font></fonts><fills count=\"3\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF46178F\"/><bgColor indexed=\"64\"/></patternFill></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"5\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"0\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\" applyAlignment=\"1\"><alignment vertical=\"center\" wrapText=\"1\"/></xf><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyAlignment=\"1\"><alignment vertical=\"top\" wrapText=\"1\"/></xf><xf numFmtId=\"0\" fontId=\"2\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/><xf numFmtId=\"0\" fontId=\"3\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>";

pub fn write_workbook(sheets: &[Sheet]) -> Vec<u8> {
    let head = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
    let mut ct = format!("{head}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    let mut wb = format!("{head}<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>");
    let mut rels = format!("{head}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    for (i, sh) in sheets.iter().enumerate() {
        let mut name = clean_sheet_name(&sh.name, i);
        let n = used.entry(name.to_lowercase()).or_insert(0);
        *n += 1;
        if *n > 1 {
            name = format!("{} {}", name.chars().take(27).collect::<String>(), n);
        }
        let id = i + 1;
        ct.push_str(&format!("<Override PartName=\"/xl/worksheets/sheet{id}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"));
        wb.push_str(&format!("<sheet name=\"{}\" sheetId=\"{id}\" r:id=\"rId{id}\"/>", esc(&name)));
        rels.push_str(&format!("<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{id}.xml\"/>"));
        files.push((format!("xl/worksheets/sheet{id}.xml"), sheet_xml(sh).into_bytes()));
    }
    ct.push_str("</Types>");
    wb.push_str("</sheets></workbook>");
    rels.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>", sheets.len() + 1));
    let root_rels = format!("{head}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>");
    let mut all = vec![
        ("[Content_Types].xml".to_string(), ct.into_bytes()),
        ("_rels/.rels".to_string(), root_rels.into_bytes()),
        ("xl/workbook.xml".to_string(), wb.into_bytes()),
        ("xl/_rels/workbook.xml.rels".to_string(), rels.into_bytes()),
        ("xl/styles.xml".to_string(), STYLES.as_bytes().to_vec()),
    ];
    all.extend(files);
    write_zip(&all)
}
