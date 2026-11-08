# Kuis Rust — tiruan Kahoot dengan server Rust

Aplikasi kuis interaktif: host mengunggah file Excel berformat Kahoot, pemain bergabung memakai PIN dari HP/laptop masing-masing, dan setelah selesai host bisa melihat soal mana yang paling banyak salah serta mengunduh hasilnya ke Excel.

**Tanpa dependensi.** Server (HTTP, WebSocket, JSON, pembaca/penulis `.xlsx`) ditulis hanya dengan library standar Rust, jadi tidak perlu internet saat build.

## Menjalankan

```bash
cargo run --release
```

Lalu buka `http://localhost:3000`. Server juga mencetak alamat IP jaringan lokal, misalnya `http://192.168.1.20:3000`. **Pemain di perangkat lain harus memakai alamat IP itu** (satu Wi-Fi), bukan `localhost`.

Ganti port dengan `PORT=8080 cargo run --release` (Windows PowerShell: `$env:PORT=8080; cargo run --release`).

## Cara pakai

1. **Host**: klik *Buat kuis*, lalu unggah file `.xlsx` (atau unduh dulu *template Excel*).
2. Periksa soal di editor (bisa diubah, ditambah, diurutkan, digandakan, dihapus). Baris Excel yang bermasalah ditampilkan lengkap dengan nomor barisnya.
3. Atur permainan (lihat tabel di bawah), klik *Buat game*, lalu bagikan **PIN 6 angka** atau tautan gabung.
4. **Pemain**: buka alamat server, masukkan PIN dan nama panggilan.
5. Host klik *Mulai*. Setelah selesai, host melihat peringkat, **analisis soal**, **detail per pemain**, dan bisa *Unduh hasil (Excel)* atau *Main lagi*.

## Format Excel (standar Kahoot)

Gunakan template dari tombol *Unduh template Excel*. Baris header (baris ke-8) berisi:

| Kolom | Isi |
|---|---|
| `Question - max 95 characters` | Teks pertanyaan |
| `Answer 1` … `Answer 4` | Pilihan jawaban (maks. 60 karakter; Answer 3 dan 4 boleh kosong) |
| `Time limit (sec)` | 5, 10, 20, 30, 60, 90, 120, atau 240 detik (angka lain 1–600 tetap diterima dengan peringatan) |
| `Correct answer(s)` | Nomor jawaban benar, mis. `2`, atau `1,3` jika benar lebih dari satu |

Parser mencari baris header otomatis, jadi file Kahoot asli dan template ini sama-sama terbaca. Header berbahasa Indonesia (`Soal`, `Jawaban 1`, `Waktu`, `Jawaban benar`) dan file `.csv` juga didukung. Format `.xls` lama tidak didukung; simpan ulang sebagai `.xlsx`.

## Pengaturan permainan

| Pengaturan | Keterangan |
|---|---|
| Batas waktu soal | Pakai waktu per baris dari Excel, atau samakan semua soal (1–600 detik) |
| Waktu membaca soal | 0–10 detik soal tampil sebelum pilihan jawaban muncul |
| Sistem poin | Standar (maks 1000, makin cepat makin besar), ganda (maks 2000), atau tetap 1000 |
| Bonus beruntun | +100 sampai +500 untuk jawaban benar berturut-turut |
| Lanjut otomatis | Manual, atau otomatis setelah 3–30 detik |
| Acak soal / acak jawaban | Urutan berbeda setiap game dan setiap *Main lagi* |
| Papan skor tiap soal | Bisa dimatikan |
| Masuk setelah game dimulai | Bisa dimatikan |
| Teks soal di HP pemain | Matikan jika soal hanya ditampilkan di proyektor (HP hanya menampilkan bentuk ▲ ◆ ● ■) |
| Maks. pemain | 1–1000 |

Kontrol host saat game berjalan: **+10 detik**, **Selesaikan sekarang**, **Lewati**, **Kunci pintu**, keluarkan pemain (klik namanya di lobby), dan **Akhiri**.

## Mengecek soal yang salah

- **Host → Analisis soal**: soal diurutkan dari yang paling sulit, persentase benar, sebaran jawaban tiap pilihan, jawaban salah yang paling sering dipilih, dan rata-rata waktu menjawab.
- **Host → Per pemain**: klik nama untuk melihat soal yang salah/tidak dijawab beserta jawaban pemain dan kunci jawabannya.
- **Pemain**: di akhir game tiap pemain bisa meninjau jawabannya sendiri.
- **Excel hasil** berisi sheet *Ringkasan*, *Peringkat*, *Analisis Soal*, *Soal yang Salah*, dan *Detail Jawaban* (semua dengan filter).

## Struktur proyek

```
src/main.rs     titik masuk
src/server.rs   HTTP + WebSocket (std::net)
src/game.rs     mesin permainan (fase, skor, timer)
src/quiz.rs     parser Excel/CSV ala Kahoot + template
src/xlsx.rs     baca/tulis .xlsx      src/zipx.rs   ZIP + inflate
src/export.rs   ekspor hasil          src/json.rs, util.rs
static/         index.html, app.js, style.css (tampilan)
```

## Catatan

- Data game disimpan di memori server; game yang tidak aktif >6 jam dibersihkan otomatis, dan semua data hilang saat server dimatikan (unduh hasilnya dulu).
- Untuk dipakai lewat internet, jalankan di belakang reverse proxy HTTPS (mis. Caddy/nginx) yang meneruskan WebSocket. Aplikasi ini tidak memiliki login; siapa pun yang mengetahui PIN bisa bergabung.

## Deploy

Aplikasi butuh server yang terus berjalan (WebSocket), jadi **tidak bisa memakai GitHub Pages**.

- **Unduh biner siap pakai:** setelah kode di-push ke GitHub, tab *Actions → Build* membuat biner untuk Windows, macOS, dan Linux (artifact). Beri tag `v1.0.0` agar otomatis masuk ke *Releases*. Jalankan biner (klik ganda di Windows), lalu buka `http://localhost:3000`.
- **Docker:** `docker build -t kuis-rust . && docker run -p 3000:3000 kuis-rust`
- **Render (gratis):** buat *New → Blueprint*, pilih repositori ini; `render.yaml` sudah disiapkan. Port dibaca dari variabel `PORT`.
- **Fly.io / Railway:** pakai `Dockerfile` yang sama.
