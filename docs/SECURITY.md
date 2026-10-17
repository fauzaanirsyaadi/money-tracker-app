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

## Data hilang atau tidak terbaca

Aplikasi **tidak pernah membuat database atau kunci baru secara diam-diam** kecuali pada first run sebenarnya (tidak ada database, kunci terbungkus, maupun `lock.json`). Bila data hilang atau tidak terbaca, layar pemulihan menjelaskan sebabnya dan menawarkan hanya dua jalan, keduanya eksplisit: pulihkan dari cadangan, atau mulai dari awal setelah konfirmasi bahwa data lama akan hilang. Kegagalan yang mungkin sementara (Keystore sibuk) hanya menawarkan "Coba lagi" dan tidak mengubah apa pun. Rinciannya, termasuk matriks keadaan, ada di ARCHITECTURE.md §25.

Layar pemulihan muncul sebelum layar PIN, tetapi **tindakan destruktifnya butuh PIN** bila `lock.json` valid dan terbaca (batas percobaan yang sama dengan membuka kunci; konfirmasi berlaku 5 menit di memori), ditambah kata `HAPUS` yang diketik untuk "mulai dari awal". Sebelum menghancurkan apa pun, aplikasi memeriksa ulang sekali: data yang ternyata terbuka, atau keadaan yang tidak bisa dipastikan, membatalkan tindakan. `lock.json` yang rusak tidak dapat diperiksa dan tidak memblokir. Pemulihan menghapus `lock.json` paling akhir, sehingga PIN dipasang ulang. Aturan lengkapnya (galat tak dikenal = sementara; hancurkan paling akhir) ada di CLAUDE.md dan ARCHITECTURE.md §26.

## Backup terenkripsi

- **Isi:** ekspor *logis* (baris JSON per baris tabel, bukan berkas SQLite), sehingga tidak bergantung pada kunci perangkat dan dapat dimigrasikan antarversi skema. **Tidak** memuat PIN, `lock.json`, `db.key`, maupun material Keystore. Saat restore, data ditulis ulang dengan kunci basis data perangkat tujuan.
- **Frasa sandi:** minimal 12 karakter (maksimal 128), bukan satu karakter berulang, dinormalkan **Unicode NFKC** sebelum diolah (format versi 2), sehingga frasa yang sama diketik dengan aksen tergabung/terurai atau huruf lebar penuh membuka cadangan yang sama. Konsekuensinya, dua frasa yang hanya berbeda pada bentuk kompatibilitas Unicode dianggap sama. **Tidak ada pemulihan frasa sandi**; UI menyatakannya dan meminta persetujuan eksplisit sebelum membuat cadangan.
- **Penyimpanan:** lewat Storage Access Framework (picker sistem). Kode Kotlin hanya menyalin berkas yang **sudah terenkripsi** antara URI pilihan pengguna dan berkas sementara di cache privat; jalur dipastikan berada di dalam cache. Rust tidak memakai `unsafe`/fd mentah.
- **Restore tidak menyentuh data aktif sebelum lolos validasi:** (1) berkas dibuka dan potongan pertama diautentikasi (frasa sandi salah terdeteksi di sini); (2) isi dibangun ke basis data **terpisah** (`restore-staged.db`) pada skema milik cadangan lalu dimigrasikan; (3) `integrity_check`, `foreign_key_check`, jumlah baris per tabel, dan saldo per rekening dibandingkan dengan catatan di cadangan; (4) pengguna melihat ringkasan dan harus mencentang konfirmasi penggantian; (5) penukaran dengan dua `rename`, basis data lama disimpan sebagai `money.db.old` sampai yang baru terbuka dan terverifikasi ulang. Bila proses mati di tengah, saat start berikutnya `money.db.old` dikembalikan (bila `money.db` hilang) atau dibuang (bila yang baru terbaca).
- **Tenggang picker:** picker sistem menjeda aplikasi. Tenggang kunci dibuka **hanya** sejak picker diluncurkan sampai ia kembali atau dibatalkan, dibatasi 60 detik dan ditutup begitu aplikasi kembali dari picker (F21), tidak disimpan di disk, sehingga proses yang mati saat picker terbuka kembali **terkunci**. Menjauh dari aplikasi lebih lama dari batas itu, atau memundurkan jam, mengunci.
- **Batas:** backup melindungi isi, bukan keberadaannya: ukuran berkas membocorkan perkiraan jumlah transaksi. Pengguna yang tahu frasa sandi dapat membuat cadangan berisi data sembarang; restore tetap menolaknya lewat batasan skema, validasi, dan pemeriksaan saldo, tetapi tidak dapat membedakan data "benar" dari data palsu yang konsisten.

## Parameter kriptografi

| Hal | Nilai |
|---|---|
| Hash PIN | Argon2id v1.3, memori **19 MiB (19 456 KiB)**, iterasi **t = 2**, paralelisme **p = 1**, keluaran 32 byte, salt acak 16 byte; disimpan sebagai string PHC di `lock.json`. Ini minimum OWASP. Waktu di perangkat kelas bawah belum diukur (Fase 8); bila terlalu lambat saat buka-kunci, turunkan memori atau t, dan ingat PIN 6 digit tetap lemah terhadap penyerang offline |
| Perbandingan | Verifikasi konstan-waktu (crate `argon2`/`password-hash`) |
| Kunci basis data | 32 byte acak (`getrandom`), dibungkus AES-256-GCM oleh kunci Keystore yang tidak dapat diekspor (`setUnlockedDeviceRequired(true)`), dipasang sebagai `PRAGMA key = "x'…'"` |
| Backup | Argon2id memori **64 MiB**, **t = 3**, **p = 1**, salt 16 byte acak; kunci 32 byte. Enkripsi XChaCha20-Poly1305 dalam konstruksi STREAM (BE32): potongan plaintext 64 KiB, prefiks nonce 19 byte acak, header 57 byte ikut diautentikasi pada setiap potongan, potongan terakhir ditandai sehingga pemotongan, penyusunan ulang, dan duplikasi terdeteksi. Batas pembacaan: m 64 KiB–256 MiB, t ≤ 10, p ≤ 4 (berkas rekayasa tidak dapat memaksa kerja berlebihan) |

## Perilaku jeda PIN setelah reboot

Hitungan salah disimpan di `lock.json` (bertahan setelah reboot). Jeda tidak disimpan sebagai waktu absolut: jam monotonik direset saat reboot dan jam dinding dapat diubah, jadi **setiap kali proses aplikasi dimulai, jeda untuk hitungan yang tersimpan dimulai penuh lagi**. Diverifikasi di emulator: 5 salah → reboot → hitungan tetap 5, sisa jeda ≈ 23 dtk sekitar 7 dtk setelah proses mulai (jeda penuh 30 dtk), dan PIN yang benar ditolak (`locked_out`). Konsekuensi: reboot tidak memotong jeda; sebaliknya, pengguna yang sah yang memulai ulang aplikasi menunggu jeda penuh lagi. Batasnya: penyerang yang punya akses root dapat menyunting `lock.json`; itu di luar model ancaman (lihat di atas).

## Kode native yang menyentuh data sensitif

`#![forbid(unsafe_code)]` berlaku di crate aplikasi (`src-tauri/src/lib.rs`) dan `unsafe_code = "forbid"` di seluruh workspace (semua crate: `mt-core`, `mt-storage`, aplikasi, dan kedua plugin mewarisi). Tidak ada kata `unsafe` dalam kode Rust kita. Kode JNI/FFI yang ada hanya milik Tauri/wry (di luar crate kita) dan pustaka SQLCipher/OpenSSL (C, lewat `rusqlite`).

| Lokasi | Data yang lewat | Catatan |
|---|---|---|
| `mt-keystore` (`KeyVault.kt` ↔ `mobile.rs`) | **Kunci DB dalam teks biasa** pada `unwrap` (hasil) dan `wrap` (masukan), berupa string hex di JSON lewat jembatan plugin Tauri | Satu-satunya titik di mana kunci melewati Kotlin/JNI. String hex tidak dapat di-zeroize (L4). Tidak dicatat di log; kesalahan dibuat generik |
| `mt-platform` (`PlatformPlugin.kt`) | Hasil biometrik (`success`/`cancelled`/`failed`), boolean tema bar sistem | Tidak ada data keuangan atau PIN |
| `MainActivity.kt` | `FLAG_SECURE`, inset jendela | Tidak ada data |
| `lock.json`, `money.db`, `db.key` | Dibaca/ditulis hanya oleh Rust (`std::fs`) | Tidak melewati Kotlin |
| PIN | WebView → IPC Tauri → Rust (`Zeroizing`) | Tidak melewati Kotlin; salinan sisi WebView tidak dapat dijamin terhapus |

Kode Kotlin tidak tercakup aturan Rust dan masuk `/security-review` (langkah akhir Fase 7). Rencana langkah B menjaga agar Kotlin hanya melihat **ciphertext** backup (menyalin berkas antara URI SAF dan berkas sementara privat), sehingga tetap tanpa `unsafe` di Rust.

## Mengapa plugin biometrik sendiri

Dokumen build semula memakai `tauri-plugin-biometric`. Plugin resmi itu mengekspos perintah ke JS dan hasilnya kembali lewat JS, sehingga skrip di WebView dapat melaporkan "berhasil" tanpa prompt. Di sini hasil prompt dibuat di kode native dan langsung dikirim ke Rust (`lock_unlock_biometric` hanya *meminta* prompt); plugin kita tidak mendaftarkan perintah JS. **Jangan menggantinya dengan plugin resmi tanpa meninjau ulang jalur ini.**

## Penanganan rahasia di memori dan keluaran

- `DbKey` adalah newtype dengan `Debug` tersamar (`DbKey(<redacted>)`), dibungkus `Zeroizing`, tanpa `Display`. Penjaga CI (`check_debug_redaction.py`) menolak `derive(Debug)` baru (termasuk lewat `cfg_attr`) dan `impl Debug` manual yang tidak menyamarkan pada tipe yang berisi kata `key`, `pin`, `hash`, `blob`, `salt`, `token`, `secret`, `passphrase`, `password`, `nonce`, atau tipe `DbKey`/`Zeroizing`.
- PIN dan frasa sandi diterima perintah langsung sebagai `Zeroizing<String>` (fitur `serde` pada `zeroize`), sehingga tidak ada salinan `String` biasa yang dibuat saat deserialisasi.
- Argon2 memakai fitur `zeroize` (blok memori kerja dihapus). `chacha20poly1305` menghapus kunci dan state sendiri saat di-drop (`ZeroizeOnDrop`).
- Tidak terjangkau: salinan di WebView, di buffer JSON IPC, dan heks kunci yang melintasi jembatan JNI (L4).

## Batas yang diketahui

- **Keystore di emulator** berlevel `software`. Level `tee`/`strongbox` belum diverifikasi pada perangkat nyata.
- **Memori:** kunci basis data dan PIN dibungkus `Zeroizing` di sisi Rust, tetapi salinan yang dibuat WebView (nilai kolom PIN), pustaka SQLCipher, dan Kotlin tidak dapat dijamin terhapus. Penyerang dengan akses memori proses (root) di luar model ancaman.
- **Jam dinding** hanya dipakai untuk kunci otomatis dengan tenggang; memundurkannya justru mengunci. Memajukannya tidak mempercepat jeda tebakan PIN (jam monotonik).
- **`lock.json` rusak:** aplikasi gagal tertutup dan menampilkan penjelasan; satu-satunya jalan adalah restore dari backup atau menghapus data aplikasi.
- **Layar perangkat dan aksesibilitas:** `FLAG_SECURE` tidak mencegah pembaca layar atau kamera orang lain.
- **Build debug** dapat dilihat lewat DevTools dan dapat menonaktifkan `FLAG_SECURE` dengan ekstra intent. Build rilis mengabaikan ekstra itu, **dibuktikan pada APK rilis** (`scripts/android_release_check.mjs`) dan DevTools tidak terkompilasi (`check_offline.py apk` memeriksa pustaka native). Pada image emulator `userdebug` WebView sendiri membuka soket DevTools untuk semua aplikasi, jadi pemeriksaan runtime hanya bermakna di perangkat build `user`.

## Melaporkan kerentanan

Belum ada kanal publik (repositori belum dipublikasikan). Ditambahkan di Fase 9.
