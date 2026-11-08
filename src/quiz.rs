//! Model soal, parser template Kahoot (xlsx/csv), validasi, dan pembuat template.

use crate::json::Json;
use crate::obj;
use crate::xlsx::{self, Cell, Sheet};

pub const MAX_Q_CHARS: usize = 95;
pub const MAX_A_CHARS: usize = 60;
pub const STD_TIMES: [u32; 8] = [5, 10, 20, 30, 60, 90, 120, 240];
pub const DEFAULT_TIME: u32 = 20;
pub const MAX_QUESTIONS: usize = 300;

#[derive(Clone, Debug)]
pub struct Question {
    pub text: String,
    pub answers: Vec<String>,
    pub correct: Vec<usize>, // indeks 0-based, terurut
    pub time: u32,
}

impl Question {
    pub fn to_json(&self) -> Json {
        obj! {
            "text" => &self.text,
            "answers" => &self.answers,
            "correct" => &self.correct,
            "time" => self.time,
        }
    }

    pub fn from_json(j: &Json) -> Result<Question, String> {
        let text = j.get("text").and_then(|v| v.as_str()).ok_or("Soal tanpa teks")?.to_string();
        let answers = j
            .get("answers")
            .and_then(|v| v.as_arr())
            .ok_or("Soal tanpa jawaban")?
            .iter()
            .map(|a| a.as_str().unwrap_or("").to_string())
            .collect();
        let correct = j
            .get("correct")
            .and_then(|v| v.as_arr())
            .ok_or("Soal tanpa kunci jawaban")?
            .iter()
            .filter_map(|c| c.as_u32().map(|n| n as usize))
            .collect();
        let time = j.get("time").and_then(|v| v.as_u32()).unwrap_or(DEFAULT_TIME);
        Ok(Question { text, answers, correct, time })
    }
}

pub struct Issue {
    pub row: usize,
    pub level: &'static str,
    pub msg: String,
}

pub struct Parsed {
    pub sheet: String,
    pub questions: Vec<Question>,
    pub issues: Vec<Issue>,
}

impl Parsed {
    pub fn to_json(&self) -> Json {
        obj! {
            "sheet" => &self.sheet,
            "questions" => self.questions.iter().map(|q| q.to_json()).collect::<Vec<Json>>(),
            "issues" => self.issues.iter().map(|i| obj!{"row" => i.row, "level" => i.level, "msg" => &i.msg}).collect::<Vec<Json>>(),
        }
    }
}

// ───────────────────────── Parser ─────────────────────────

#[derive(Default)]
struct Cols {
    q: Option<usize>,
    a: [Option<usize>; 4],
    time: Option<usize>,
    correct: Option<usize>,
}

fn answer_no(s: &str) -> Option<usize> {
    for n in 1..=4usize {
        for p in ["answer", "jawaban", "opsi", "pilihan"] {
            if s.starts_with(&format!("{p} {n}")) || s.starts_with(&format!("{p}{n}")) {
                return Some(n - 1);
            }
        }
    }
    None
}

fn detect_cols(row: &[String]) -> Option<Cols> {
    let mut c = Cols::default();
    for (i, cell) in row.iter().enumerate() {
        let s = cell.trim().to_lowercase();
        if s.is_empty() {
            continue;
        }
        if s.contains("correct") || s.contains("benar") || s.contains("kunci") {
            c.correct.get_or_insert(i);
        } else if s.contains("time") || s.contains("waktu") || s.contains("durasi") {
            c.time.get_or_insert(i);
        } else if let Some(n) = answer_no(&s) {
            if c.a[n].is_none() {
                c.a[n] = Some(i);
            }
        } else if s.starts_with("question") || s.starts_with("soal") || s.starts_with("pertanyaan") {
            c.q.get_or_insert(i);
        }
    }
    if c.q.is_some() && c.a[0].is_some() && c.a[1].is_some() && c.correct.is_some() {
        Some(c)
    } else {
        None
    }
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let first_line = text.lines().next().unwrap_or("");
    let delim = if first_line.matches(';').count() > first_line.matches(',').count() { ';' } else { ',' };
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut inq = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if inq {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    inq = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' {
            inq = true;
        } else if c == delim {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(c);
        }
        if rows.len() > 5000 {
            break;
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Membaca file kuis. Mendukung .xlsx (format template Kahoot) dan .csv.
pub fn parse_file(bytes: &[u8]) -> Result<Parsed, String> {
    let (sheet, grid) = if bytes.starts_with(b"PK") {
        xlsx::read_first_sheet(bytes)?
    } else if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Err("Format .xls lama tidak didukung. Simpan ulang sebagai .xlsx lalu unggah lagi.".into());
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| "File bukan .xlsx maupun CSV teks UTF-8".to_string())?;
        ("CSV".to_string(), parse_csv(text))
    };
    parse_grid(sheet, &grid)
}

pub fn parse_grid(sheet: String, grid: &[Vec<String>]) -> Result<Parsed, String> {
    let mut header = None;
    for (i, row) in grid.iter().enumerate().take(60) {
        if let Some(c) = detect_cols(row) {
            header = Some((i, c));
            break;
        }
    }
    let Some((hi, cols)) = header else {
        return Err("Baris header tidak ditemukan. Pastikan kolom \"Question\", \"Answer 1\", \"Answer 2\", dan \"Correct answer(s)\" ada, atau unduh template resmi dari tombol di atas.".into());
    };

    let mut questions = Vec::new();
    let mut issues = Vec::new();
    for (i, row) in grid.iter().enumerate().skip(hi + 1) {
        let xr = i + 1;
        let get = |col: Option<usize>| -> String {
            col.and_then(|c| row.get(c)).map(|s| s.trim().to_string()).unwrap_or_default()
        };
        let text = get(cols.q);
        let raw: Vec<String> = (0..4).map(|n| get(cols.a[n])).collect();
        let time_s = get(cols.time);
        let corr_s = get(cols.correct);
        if text.is_empty() && raw.iter().all(|a| a.is_empty()) {
            continue;
        }
        let mut ok = true;
        let mut err = |msg: String| {
            issues.push(Issue { row: xr, level: "error", msg });
        };

        if text.is_empty() {
            err("Pertanyaan kosong".into());
            ok = false;
        }
        // kompaksi jawaban (jawaban kosong dibuang, indeks kunci disesuaikan)
        let mut map: [Option<usize>; 4] = [None; 4];
        let mut answers = Vec::new();
        for (n, a) in raw.iter().enumerate() {
            if !a.is_empty() {
                map[n] = Some(answers.len());
                answers.push(a.clone());
            }
        }
        if answers.len() < 2 {
            err("Minimal harus ada 2 pilihan jawaban".into());
            ok = false;
        }
        let mut correct: Vec<usize> = Vec::new();
        if corr_s.is_empty() {
            err("Kolom jawaban benar kosong (isi 1-4, contoh: 2 atau 1,3)".into());
            ok = false;
        } else {
            for tok in corr_s.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()) {
                let n = match tok.parse::<usize>() {
                    Ok(n) => n,
                    Err(_) => match tok.to_lowercase().as_str() {
                        "a" => 1,
                        "b" => 2,
                        "c" => 3,
                        "d" => 4,
                        _ => 0,
                    },
                };
                if (1..=4).contains(&n) {
                    match map[n - 1] {
                        Some(ni) => correct.push(ni),
                        None => {
                            err(format!("Kunci jawaban menunjuk Answer {n} yang kosong"));
                            ok = false;
                        }
                    }
                } else {
                    err(format!("Nilai kunci jawaban \"{tok}\" tidak valid (harus 1-4)"));
                    ok = false;
                }
            }
        }
        correct.sort_unstable();
        correct.dedup();
        if ok && correct.is_empty() {
            err("Belum ada kunci jawaban yang valid".into());
            ok = false;
        }

        let mut time = DEFAULT_TIME;
        let mut warns: Vec<String> = Vec::new();
        if time_s.is_empty() {
            warns.push(format!("Batas waktu kosong, dipakai {DEFAULT_TIME} detik"));
        } else {
            let t = time_s
                .split(|c: char| !c.is_ascii_digit())
                .find(|t| !t.is_empty())
                .and_then(|t| t.parse::<u32>().ok());
            match t {
                Some(t) if (1..=600).contains(&t) => {
                    time = t;
                    if !STD_TIMES.contains(&t) {
                        warns.push(format!("Batas waktu {t} detik bukan pilihan standar Kahoot (5, 10, 20, 30, 60, 90, 120, 240) tapi tetap dipakai"));
                    }
                }
                _ => {
                    err(format!("Batas waktu \"{time_s}\" tidak valid (1-600 detik)"));
                    ok = false;
                }
            }
        }
        if text.chars().count() > MAX_Q_CHARS {
            warns.push(format!("Pertanyaan {} karakter, melebihi batas Kahoot ({MAX_Q_CHARS})", text.chars().count()));
        }
        for (n, a) in raw.iter().enumerate() {
            if a.chars().count() > MAX_A_CHARS {
                warns.push(format!("Answer {} {} karakter, melebihi batas Kahoot ({MAX_A_CHARS})", n + 1, a.chars().count()));
            }
        }
        for w in warns {
            issues.push(Issue { row: xr, level: "warning", msg: w });
        }
        if ok {
            if questions.len() >= MAX_QUESTIONS {
                issues.push(Issue { row: xr, level: "error", msg: format!("Melebihi batas {MAX_QUESTIONS} soal") });
                break;
            }
            questions.push(Question { text, answers, correct, time });
        }
    }
    if questions.is_empty() && issues.is_empty() {
        return Err("Tidak ada soal di bawah baris header.".into());
    }
    Ok(Parsed { sheet, questions, issues })
}

/// Validasi akhir sebelum game dibuat (juga membersihkan teks).
pub fn validate(qs: &mut [Question]) -> Result<(), String> {
    if qs.is_empty() {
        return Err("Belum ada soal".into());
    }
    if qs.len() > MAX_QUESTIONS {
        return Err(format!("Maksimal {MAX_QUESTIONS} soal"));
    }
    for (i, q) in qs.iter_mut().enumerate() {
        let n = i + 1;
        q.text = q.text.trim().to_string();
        for a in q.answers.iter_mut() {
            *a = a.trim().to_string();
        }
        if q.text.is_empty() {
            return Err(format!("Soal {n}: pertanyaan kosong"));
        }
        if q.text.chars().count() > 300 {
            return Err(format!("Soal {n}: pertanyaan terlalu panjang (maks 300 karakter)"));
        }
        if q.answers.len() < 2 || q.answers.len() > 4 {
            return Err(format!("Soal {n}: jumlah jawaban harus 2 sampai 4"));
        }
        if q.answers.iter().any(|a| a.is_empty() || a.chars().count() > 150) {
            return Err(format!("Soal {n}: jawaban kosong atau terlalu panjang"));
        }
        q.correct.sort_unstable();
        q.correct.dedup();
        if q.correct.is_empty() || q.correct.iter().any(|&c| c >= q.answers.len()) {
            return Err(format!("Soal {n}: kunci jawaban belum valid"));
        }
        if !(1..=600).contains(&q.time) {
            return Err(format!("Soal {n}: batas waktu harus 1-600 detik"));
        }
    }
    Ok(())
}

// ───────────────────────── Template ─────────────────────────

pub fn template_xlsx() -> Vec<u8> {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    rows.push(vec![Cell::title("Template Kuis (format Kahoot)")]);
    rows.push(vec![Cell::s("Isi mulai baris ke-9. Jangan ubah baris header (baris ke-8).")]);
    rows.push(vec![Cell::s("• Pertanyaan maks. 95 karakter, setiap jawaban maks. 60 karakter.")]);
    rows.push(vec![Cell::s("• Answer 1 dan 2 wajib; Answer 3 dan 4 boleh dikosongkan.")]);
    rows.push(vec![Cell::s("• Batas waktu (detik): 5, 10, 20, 30, 60, 90, 120, atau 240.")]);
    rows.push(vec![Cell::s("• Kunci jawaban: angka 1-4. Lebih dari satu jawaban benar dipisah koma, contoh: 1,3")]);
    rows.push(vec![]);
    rows.push(vec![
        Cell::hdr(""),
        Cell::hdr("Question - max 95 characters"),
        Cell::hdr("Answer 1 - max 60 characters"),
        Cell::hdr("Answer 2 - max 60 characters"),
        Cell::hdr("Answer 3 - max 60 characters"),
        Cell::hdr("Answer 4 - max 60 characters"),
        Cell::hdr("Time limit (sec) – 5, 10, 20, 30, 60, 90, 120, or 240 secs"),
        Cell::hdr("Correct answer(s) - choose at least one"),
    ]);
    let samples: [(&str, [&str; 4], u32, &str); 6] = [
        ("Apa ibu kota Indonesia?", ["Bandung", "Jakarta", "Surabaya", "Medan"], 20, "2"),
        ("Bahasa pemrograman manakah yang fokus pada keamanan memori?", ["Rust", "Python", "Ruby", "PHP"], 20, "1"),
        ("Manakah yang termasuk bilangan prima?", ["2", "4", "7", "9"], 30, "1,3"),
        ("Matahari terbit dari arah timur.", ["Benar", "Salah", "", ""], 10, "1"),
        ("Berapakah hasil dari 12 x 12?", ["122", "144", "124", "154"], 30, "2"),
        ("Planet terbesar di tata surya kita adalah...", ["Mars", "Saturnus", "Jupiter", "Bumi"], 20, "3"),
    ];
    for (i, (q, a, t, c)) in samples.iter().enumerate() {
        rows.push(vec![
            Cell::n((i + 1) as f64),
            Cell::wrap(*q),
            Cell::wrap(a[0]),
            Cell::wrap(a[1]),
            Cell::wrap(a[2]),
            Cell::wrap(a[3]),
            Cell::n(*t as f64),
            Cell::s(*c),
        ]);
    }
    let sheet = Sheet {
        name: "Kuis".into(),
        rows,
        widths: vec![6.0, 48.0, 24.0, 24.0, 24.0, 24.0, 22.0, 24.0],
        freeze_rows: 8,
        filter_header_row: None,
    };
    xlsx::write_workbook(&[sheet])
}
