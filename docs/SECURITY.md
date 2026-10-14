# Keamanan Money Tracker

Dokumen ini menjelaskan apa yang dilindungi aplikasi, apa yang **tidak**, dan kompromi yang dipilih secara sengaja. Rancangan teknis lengkap ada di [ARCHITECTURE.md](ARCHITECTURE.md). Status: pra-rilis; ditulis pada Fase 7 langkah A dan akan dilengkapi di langkah B (backup) dan C.

## Prinsip

1. **Sepenuhnya offline.** Build rilis tidak meminta izin `INTERNET`; CI menggagalkan build bila muncul (`scripts/check_offline.py`).
2. **Data tidak keluar dari perangkat** kecuali lewat ekspor terenkripsi yang dipicu pengguna (langkah B).
3. **Tidak ada telemetri, tidak ada log data keuangan.**

## Apa yang dilindungi, dan terhadap siapa

| Ancaman | Perlindungan |
|---|---|
| Orang yang memegang ponsel (tidak terkunci) dan membuka aplikasi | Kunci aplikasi: PIN (Argon2id) dan/atau biometrik. Terkunci setiap kali aplikasi ke latar belakang (pengaturan: segera / 30 dtk / 1 mnt / 5 mnt). Basis data **baru dibuka setelah buka-kunci** dan ditutup saat terkunci |
| Menebak PIN | Hitungan salah disimpan **sebelum** PIN diperiksa; setelah 4 salah beruntun: jeda 30 dtk, 1 mnt, 5 mnt, 15 mnt, lalu 60 mnt. Memulai ulang aplikasi tidak mengulang jeda dan mengubah jam sistem tidak memperpendeknya (jeda memakai jam monotonik, dimulai penuh lagi setelah proses dimulai ulang) |
| Tangkapan layar, perekam layar, pratinjau aplikasi terbaru | `FLAG_SECURE` pada seluruh aplikasi; konten ditutup begitu aplikasi disembunyikan |
| Membaca berkas basis data dari penyimpanan (ponsel lain, backup ADB, cadangan cloud) | SQLCipher (AES-256) dengan kunci acak 32 byte yang dibungkus kunci Android Keystore yang tidak dapat diekspor. `allowBackup=false` dan aturan ekstraksi data mengecualikan semuanya |
| Kunci Keystore dipakai saat perangkat terkunci | Kunci dibuat dengan `setUnlockedDeviceRequired(true)` (Android 9+) |
| Aplikasi mengirim data diam-diam | Tidak ada izin jaringan, CSP ketat, penjaga CI, `cargo deny` melarang pustaka jaringan |

## Kompromi yang disengaja: PIN adalah gerbang, bukan kunci

**Kunci basis data tidak diturunkan dari PIN dan tidak diikat ke biometrik.** Kunci acak dibungkus Keystore dan dibuka oleh aplikasi sendiri; PIN dan biometrik hanya memutuskan apakah aplikasi *mau* membukanya.

Mengapa tidak mengikatnya:

- **Biometrik per-pemakaian pada kunci.** Android membatalkan kunci seperti itu bila sidik jari atau wajah di perangkat berubah (atau data biometrik dihapus). Aplikasi tanpa cloud berarti seluruh data hilang tanpa jalan kembali.
- **PIN sebagai bahan kunci.** PIN yang lupa berarti data hilang. Untuk pencatat keuangan pribadi tanpa server, risiko kehilangan data lebih besar daripada manfaatnya, dan PIN 6 digit tidak memberi kekuatan kriptografis berarti (1 juta kemungkinan) terhadap penyerang offline.

Konsekuensi yang harus dipahami:

- Penyerang yang punya **akses root atau berkas aplikasi** dapat mengubah `lock.json` (menghapus kunci, mengatur ulang hitungan salah) lalu membuka aplikasi. PIN **tidak** melindungi dari penyerang di tingkat ini; yang melindungi adalah Keystore: berkas `money.db` tanpa kunci terbungkus tidak terbaca, dan kunci tidak keluar dari Keystore.
- Karena bukan kriptografis, **PIN lupa tidak punya jalan pemulihan di dalam aplikasi**. Jalannya adalah **backup terenkripsi** (langkah B) dan memulihkannya pada instalasi bersih. Sebelum backup tersedia, aplikasi memperingatkan pengguna saat mengatur PIN.
- Hasil biometrik adalah keputusan boolean dari sistem; aplikasi tidak memakai `CryptoObject`. Hasilnya dikirim dari kode native langsung ke Rust, sehingga skrip di WebView tidak dapat memalsukannya (hanya dapat *meminta* prompt).

## Batas yang diketahui

- **Keystore di emulator** berlevel `software`. Level `tee`/`strongbox` belum diverifikasi pada perangkat nyata.
- **Memori:** kunci basis data dan PIN dibungkus `Zeroizing` di sisi Rust, tetapi salinan yang dibuat WebView (nilai kolom PIN), pustaka SQLCipher, dan Kotlin tidak dapat dijamin terhapus. Penyerang dengan akses memori proses (root) di luar model ancaman.
- **Jam dinding** hanya dipakai untuk kunci otomatis dengan tenggang; memundurkannya justru mengunci. Memajukannya tidak mempercepat jeda tebakan PIN (jam monotonik).
- **`lock.json` rusak:** aplikasi gagal tertutup dan menampilkan penjelasan; satu-satunya jalan adalah restore dari backup atau menghapus data aplikasi.
- **Layar perangkat dan aksesibilitas:** `FLAG_SECURE` tidak mencegah pembaca layar atau kamera orang lain.
- **Build debug** dapat dilihat lewat DevTools dan dapat menonaktifkan `FLAG_SECURE` dengan ekstra intent; build rilis mengabaikan keduanya.

## Melaporkan kerentanan

Belum ada kanal publik (repositori belum dipublikasikan). Ditambahkan di Fase 9.
