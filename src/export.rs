//! Ekspor hasil permainan ke Excel (.xlsx).

use crate::game::Report;
use crate::xlsx::{write_workbook, Cell, Sheet};

fn pick_text(answers: &[String], idx: &[usize]) -> String {
    idx.iter()
        .filter_map(|&i| answers.get(i).map(|a| format!("{}. {}", i + 1, a)))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn pct(a: u32, b: u32) -> f64 {
    if b == 0 {
        0.0
    } else {
        ((a as f64 / b as f64) * 1000.0).round() / 10.0
    }
}

pub fn report_xlsx(r: &Report) -> Vec<u8> {
    let nq = r.questions.len() as u32;

    // ── Ringkasan ──
    let tot_ans: u32 = r.questions.iter().map(|q| q.answered).sum();
    let tot_ok: u32 = r.questions.iter().map(|q| q.correct_n).sum();
    let tot_slots = nq * r.total_players as u32;
    let hardest = r
        .questions
        .iter()
        .filter(|_| r.total_players > 0)
        .min_by(|a, b| {
            pct(a.correct_n, r.total_players as u32)
                .partial_cmp(&pct(b.correct_n, r.total_players as u32))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let mut sum = vec![
        vec![Cell::title(format!("Hasil Kuis: {}", r.title))],
        vec![],
        vec![Cell::bold("Jumlah pemain"), Cell::n(r.total_players as f64)],
        vec![Cell::bold("Jumlah soal"), Cell::n(nq as f64)],
        vec![Cell::bold("Jawaban masuk"), Cell::n(tot_ans as f64)],
        vec![Cell::bold("Jawaban benar"), Cell::n(tot_ok as f64)],
        vec![Cell::bold("Akurasi keseluruhan (%)"), Cell::n(pct(tot_ok, tot_slots))],
    ];
    if let Some(h) = hardest {
        sum.push(vec![
            Cell::bold("Soal tersulit"),
            Cell::wrap(format!("No. {} - {} ({}% benar)", h.index + 1, h.text, pct(h.correct_n, r.total_players as u32))),
        ]);
    }
    if let Some(top) = r.players.first() {
        sum.push(vec![Cell::bold("Juara 1"), Cell::s(format!("{} ({} poin)", top.name, top.score))]);
    }
    let s_sum = Sheet { name: "Ringkasan".into(), rows: sum, widths: vec![28.0, 70.0], freeze_rows: 0, filter_header_row: None };

    // ── Peringkat ──
    let mut rk = vec![vec![
        Cell::hdr("Peringkat"), Cell::hdr("Nama"), Cell::hdr("Skor"), Cell::hdr("Benar"),
        Cell::hdr("Salah"), Cell::hdr("Tidak dijawab"), Cell::hdr("Akurasi (%)"),
    ]];
    for p in &r.players {
        rk.push(vec![
            Cell::n(p.rank as f64), Cell::s(&p.name), Cell::n(p.score as f64), Cell::n(p.correct_n as f64),
            Cell::n(p.wrong_n as f64), Cell::n(p.skipped_n as f64), Cell::n(pct(p.correct_n, nq)),
        ]);
    }
    let s_rank = Sheet {
        name: "Peringkat".into(), rows: rk, widths: vec![11.0, 26.0, 10.0, 9.0, 9.0, 15.0, 13.0],
        freeze_rows: 1, filter_header_row: Some(0),
    };

    // ── Analisis soal ──
    let mut qs = vec![vec![
        Cell::hdr("No"), Cell::hdr("Soal"), Cell::hdr("Jawaban benar"), Cell::hdr("Dijawab"), Cell::hdr("Benar"),
        Cell::hdr("Salah / tidak dijawab"), Cell::hdr("% benar"), Cell::hdr("Rata-rata waktu (detik)"),
        Cell::hdr("Dipilih: jawaban 1"), Cell::hdr("Dipilih: jawaban 2"), Cell::hdr("Dipilih: jawaban 3"), Cell::hdr("Dipilih: jawaban 4"),
    ]];
    for q in &r.questions {
        let tot = r.total_players as u32;
        let mut row = vec![
            Cell::n((q.index + 1) as f64), Cell::wrap(&q.text), Cell::wrap(pick_text(&q.answers, &q.correct)),
            Cell::n(q.answered as f64), Cell::n(q.correct_n as f64), Cell::n(tot.saturating_sub(q.correct_n) as f64),
            Cell::n(pct(q.correct_n, tot)), Cell::n((q.avg_ms as f64 / 100.0).round() / 10.0),
        ];
        for i in 0..4 {
            row.push(match q.counts.get(i) {
                Some(c) => Cell::n(*c as f64),
                None => Cell::e(),
            });
        }
        qs.push(row);
    }
    let s_q = Sheet {
        name: "Analisis Soal".into(), rows: qs,
        widths: vec![6.0, 50.0, 34.0, 10.0, 9.0, 20.0, 10.0, 22.0, 17.0, 17.0, 17.0, 17.0],
        freeze_rows: 1, filter_header_row: Some(0),
    };

    // ── Detail & soal salah ──
    let head = || vec![
        Cell::hdr("Nama"), Cell::hdr("No"), Cell::hdr("Soal"), Cell::hdr("Jawaban pemain"), Cell::hdr("Jawaban benar"),
        Cell::hdr("Status"), Cell::hdr("Poin"), Cell::hdr("Waktu (detik)"),
    ];
    let mut detail = vec![head()];
    let mut wrong = vec![head()];
    for p in &r.players {
        for q in &r.questions {
            let a = p.answers.get(q.index).and_then(|a| a.as_ref());
            let (mine, status, pts, secs) = match a {
                Some(a) => (
                    pick_text(&q.answers, &a.choices),
                    if a.ok { "Benar" } else { "Salah" },
                    a.points as f64,
                    Cell::n((a.ms as f64 / 100.0).round() / 10.0),
                ),
                None => ("(tidak dijawab)".to_string(), "Tidak dijawab", 0.0, Cell::e()),
            };
            let mk = |secs: Cell| vec![
                Cell::s(&p.name), Cell::n((q.index + 1) as f64), Cell::wrap(&q.text), Cell::wrap(&mine),
                Cell::wrap(pick_text(&q.answers, &q.correct)), Cell::s(status), Cell::n(pts), secs,
            ];
            let is_bad = status != "Benar";
            let secs2 = match a {
                Some(a) => Cell::n((a.ms as f64 / 100.0).round() / 10.0),
                None => Cell::e(),
            };
            detail.push(mk(secs));
            if is_bad {
                wrong.push(mk(secs2));
            }
        }
    }
    let widths = vec![22.0, 6.0, 46.0, 30.0, 30.0, 14.0, 8.0, 14.0];
    let s_detail = Sheet { name: "Detail Jawaban".into(), rows: detail, widths: widths.clone(), freeze_rows: 1, filter_header_row: Some(0) };
    let s_wrong = Sheet { name: "Soal yang Salah".into(), rows: wrong, widths, freeze_rows: 1, filter_header_row: Some(0) };

    write_workbook(&[s_sum, s_rank, s_q, s_wrong, s_detail])
}
