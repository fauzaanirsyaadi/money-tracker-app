# ARCHITECTURE — Money Tracker App

Status: **DISETUJUI (Fase 1)** · Tanggal: 2026-10-14 · K1–K8 disetujui pemilik proyek.
Dokumen ini adalah rencana. Setiap keputusan bertanda **[KEPUTUSAN]**, setiap hal yang belum terbukti di Android bertanda **[RISIKO]**, dan hal yang butuh persetujuan pemilik proyek ada di bagian 12.

---

## 1. Tujuan dan batasan

Aplikasi pencatatan keuangan pribadi, Android dulu, **sepenuhnya offline** (tanpa akun, server, analitik, izin INTERNET pada build rilis). Prioritas: aman → cepat → ringan → nyaman.

Titik awal yang terukur (hasil `cargo tree --target aarch64-linux-android` pada scaffold, 2026-10-14):

| Hal | Nilai |
|---|---|
| Crate Rust (transitif, target Android) | ~305 |
| Crate jaringan yang sudah ada di graph | `reqwest 0.13`, `hyper 1`, `hyper-util`, `tokio`, `http` |
| Sumber crate jaringan | `tauri` itu sendiri (fitur `json`, `stream`) dan `tauri-plugin-opener` |
| CSP scaffold | `null` (tidak ada pembatasan) |
| Plugin bawaan template | `tauri-plugin-opener` (tidak dibutuhkan, harus dihapus) |
| JDK terpasang | OpenJDK 21 (dokumen v1 menyebut JDK 17; perlu diuji) |

Implikasi: karena `reqwest`/`hyper` sudah masuk lewat `tauri`, aturan "tidak ada crate jaringan" di Fase 4 **tidak bisa** berupa larangan mutlak. Penegakannya berlapis: (a) tidak ada izin INTERNET di manifest rilis, (b) CSP memblokir origin jarak jauh, (c) tidak ada kode aplikasi yang memanggil jaringan, (d) daftar izin crate jaringan yang dibenarkan di bagian 10.

---

## 2. Prinsip dan aturan lapisan

1. `core` tanpa I/O dan tanpa dependensi jaringan; seluruh aturan bisnis dan uang diuji di sini.
2. `storage` hanya tahu SQLite/SQLCipher; tidak tahu Tauri.
3. `app` (src-tauri) hanya merakit: state, command, izin. Tidak ada aturan bisnis.
4. UI hanya memanggil command bertipe. UI tidak pernah memegang kunci, path database, atau SQL.
5. Semua agregasi laporan dihitung di SQL atau Rust, bukan di UI.
6. Jam (tanggal "hari ini") disuntikkan dari `app` ke `core` agar logika berulang dan anggaran dapat diuji deterministik.
7. Tidak ada data keuangan di log, pesan error, atau nama command.

---

## 3. Tata letak repositori dan crate

```
money-tracker-app/
├── Cargo.toml              # [workspace] members = crates/core, crates/storage, src-tauri
├── CLAUDE.md
├── docs/ARCHITECTURE.md, SECURITY.md
├── crates/
│   ├── core/               # lib: Money, Currency, Date, aturan bisnis, recurrence, validasi
│   └── storage/            # lib: SQLCipher, migrasi, repository, query laporan, backup I/O
├── src-tauri/              # crate app: AppState, command, plugin, proyek Android (gen/android)
│   └── plugins/keystore/   # plugin Tauri mobile kecil (Kotlin + Rust) untuk Android Keystore
├── src/                    # UI Svelte
└── .github/workflows/
```

Arah dependensi (hanya ke bawah):

```
src (UI) ──invoke──▶ app ──▶ storage ──▶ core
                      └──────────────────▶ core
```

| Crate | Isi | Dependensi eksternal |
|---|---|---|
| `core` | `Money`, `Currency`, `Date`, `Category`/`Account`/`Transaction` (tipe domain), `Budget` progress, `RecurrenceRule::occurrences_until`, `allocate`, validasi | `time` saja (aritmatika kalender); tanpa `serde`, tanpa `thiserror` |
| `storage` | `Db` (open/key/migrate), repository per entitas, query laporan, ekspor/impor logis | `rusqlite` (`bundled-sqlcipher*`) |
| `app` | `AppState`, command, `AppError`, plugin biometrik/keystore/dialog, DTO `serde` | `tauri`, `serde`, `serde_json`, `zeroize`, `argon2`, `chacha20poly1305`, plugin resmi |

**[KEPUTUSAN, Fase 2]** Nama paket Cargo adalah `mt-core` dan `mt-storage` (direktori tetap `crates/core` dan `crates/storage`), karena paket bernama `core` bentrok dengan crate `core` milik Rust.

**[KEPUTUSAN]** DTO untuk IPC (dengan `serde`) berada di crate `app`, bukan di `core`, agar `core` tetap tanpa dependensi dan bentuk IPC bisa berubah tanpa menyentuh domain.

**[KEPUTUSAN]** `#![forbid(unsafe_code)]` dipasang di `core`, `storage`, dan `app`. **[RISIKO — terselesaikan di Fase 2: `cargo check --target aarch64-linux-android` lulus dengan `forbid`; konfirmasi akhir saat APK dibangun]** makro `#[tauri::mobile_entry_point]` menghasilkan `extern "C"`/`no_mangle`, yang di edisi 2024 adalah atribut unsafe dan bisa memicu lint `forbid`. Jika terbukti, `app` memakai `#![deny(unsafe_code)]` dengan satu `#[allow(unsafe_code)]` sempit pada titik masuk, dicatat di SECURITY.md. Diuji di Fase 2 (hanya terlihat saat target Android dikompilasi).

---

## 4. Tipe `Money` dan aturan uang

```rust
pub struct Currency { code: [u8; 3], exponent: u8 }   // IDR=0, USD=2, JPY=0, KWD=3
pub struct Money { minor: i64, currency: Currency }   // field privat
```

- Tidak ada `f32`/`f64` di mana pun; parsing "12.345,67" dilakukan sebagai string → minor unit, tanpa float.
- Aritmatika **checked**: `checked_add`, `checked_sub`, `checked_neg`, `checked_mul_i64`; overflow = error, bukan wrap. Penjumlahan beda mata uang = error tipe.
- `allocate(&self, ratios: &[u32]) -> Vec<Money>`: metode *largest remainder*; jumlah hasil selalu persis sama dengan input, tidak ada satuan minor yang hilang.
- Pembulatan hanya di satu tempat (`round_div`, half-away-from-zero), dipakai untuk persentase anggaran.
- **Batas nilai [KEPUTUSAN]:** `|minor| ≤ 2^53 − 1`. Alasan: nilai melewati batas JSON/JS number. Di IPC, uang tetap dikirim sebagai **string desimal minor unit** (`"1250000"`), dan UI memakai `BigInt` untuk tampilan. Batas ini menjadi pagar tambahan bila ada kode UI yang keliru memakai `Number`.
- Property test (Fase 8, `proptest`): `allocate` menjaga jumlah; `a + b - b == a`; parse(format(x)) == x; tidak ada panic di batas.

Mata uang v1: lihat keputusan K1 (bagian 12).

---

## 5. Skema SQLite

Konvensi: tabel `STRICT`, `PRAGMA foreign_keys=ON`, `PRAGMA journal_mode=WAL`, `PRAGMA secure_delete=ON`. Semua jumlah `INTEGER` minor unit. Tanggal transaksi disimpan sebagai tanggal kalender lokal (`'YYYY-MM-DD'`).

```sql
CREATE TABLE settings (
  key TEXT PRIMARY KEY, value TEXT NOT NULL
) STRICT;                               -- currency, schema info, pengaturan UI

CREATE TABLE accounts (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
  kind TEXT NOT NULL CHECK (kind IN ('cash','bank','ewallet')),
  currency TEXT NOT NULL,                       -- ISO 4217
  opening_balance_minor INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1)),
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL                   -- unix detik UTC
) STRICT;

CREATE TABLE categories (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
  kind TEXT NOT NULL CHECK (kind IN ('income','expense')),
  parent_id INTEGER REFERENCES categories(id) ON DELETE RESTRICT,
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1))
) STRICT;                                       -- kedalaman maks 1 level, dijaga di core

CREATE TABLE recurring_rules (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  category_id INTEGER NOT NULL REFERENCES categories(id),
  kind TEXT NOT NULL CHECK (kind IN ('income','expense')),
  amount_minor INTEGER NOT NULL,                -- bertanda, sama seperti transactions
  note TEXT NOT NULL DEFAULT '',
  freq TEXT NOT NULL CHECK (freq IN ('daily','weekly','monthly','yearly')),
  every_n INTEGER NOT NULL DEFAULT 1 CHECK (every_n >= 1),
  anchor_day INTEGER NOT NULL CHECK (anchor_day BETWEEN 1 AND 31), -- jaga "tanggal 31" saat bulan pendek
  start_on TEXT NOT NULL,
  end_on TEXT,
  next_due TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
) STRICT;

CREATE TABLE transactions (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  category_id INTEGER REFERENCES categories(id),
  amount_minor INTEGER NOT NULL,                -- bertanda: masuk > 0, keluar < 0
  kind TEXT NOT NULL CHECK (kind IN ('income','expense','transfer_out','transfer_in')),
  note TEXT NOT NULL DEFAULT '' CHECK (length(note) <= 500),
  occurred_on TEXT NOT NULL,                    -- 'YYYY-MM-DD'
  transfer_group_id INTEGER,                    -- = id baris transfer_out
  recurring_rule_id INTEGER REFERENCES recurring_rules(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  CHECK ((kind IN ('income','transfer_in')  AND amount_minor > 0)
      OR (kind IN ('expense','transfer_out') AND amount_minor < 0)),
  CHECK ((kind IN ('transfer_out','transfer_in'))
         = (transfer_group_id IS NOT NULL AND category_id IS NULL)),
  CHECK (kind IN ('transfer_out','transfer_in') OR category_id IS NOT NULL)
) STRICT;

CREATE TABLE budgets (
  id INTEGER PRIMARY KEY,
  category_id INTEGER NOT NULL REFERENCES categories(id),
  month TEXT NOT NULL CHECK (month GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]'),
  limit_minor INTEGER NOT NULL CHECK (limit_minor > 0),
  UNIQUE (category_id, month)
) STRICT;

CREATE INDEX idx_tx_date     ON transactions(occurred_on);
CREATE INDEX idx_tx_account  ON transactions(account_id, occurred_on);
CREATE INDEX idx_tx_category ON transactions(category_id, occurred_on);
CREATE UNIQUE INDEX uq_tx_recurring
  ON transactions(recurring_rule_id, occurred_on) WHERE recurring_rule_id IS NOT NULL;
```

### Penyimpangan dari usulan awal (dokumen v1 §4.3), beserta alasannya

| Usulan | Final | Alasan |
|---|---|---|
| `accounts` tanpa saldo awal | tambah `opening_balance_minor` | "Saldo dihitung dari transaksi" tidak cukup bila pengguna mulai dengan saldo non-nol |
| `occurred_at` | `occurred_on` (tanggal kalender) | Jam tidak bernilai untuk pencatatan keuangan; menghindari bug zona waktu pada pengelompokan bulan |
| `amount_minor` + `kind` | `amount_minor` **bertanda** + `CHECK` terhadap `kind` | Saldo = `SUM(amount_minor)`; laporan tidak perlu `CASE`; basis data menolak tanda yang salah |
| `recurring_rules.template` | kolom eksplisit | Divalidasi `CHECK`/`FK`, tanpa JSON yang harus diparse ulang |
| 3 indeks satu kolom | 3 indeks, dua di antaranya komposit | Jumlah indeks sama, tetapi melayani daftar per akun/kategori terurut tanpa sort; dikonfirmasi `EXPLAIN QUERY PLAN` di Fase 8 |
| — | `uq_tx_recurring` | Membuat catch-up idempoten: menjalankan dua kali tidak menggandakan transaksi |

### Aturan penting

- **Transfer** = dua baris (`transfer_out`, `transfer_in`) dalam satu transaksi DB, `transfer_group_id` = `id` baris keluar. Dikecualikan dari laporan pengeluaran/pemasukan, termasuk dalam saldo akun. Menghapus/mengubah satu sisi memengaruhi pasangannya (dilakukan atomik di `storage`).
- **Saldo akun** = `opening_balance_minor + SUM(amount_minor)`; tidak ada kolom saldo tersimpan di v1 (menghindari drift). Dievaluasi ulang di Fase 8 berdasarkan angka.
- **Paging** memakai keyset `(occurred_on DESC, id DESC)`, bukan `OFFSET`.
- **Hapus** akun/kategori yang punya transaksi tidak diperbolehkan (`RESTRICT`); gunakan `archived`.
- **Migrasi** memakai `PRAGMA user_version` dan berkas SQL yang disematkan (`include_str!`), dijalankan berurutan dalam satu transaksi, tanpa crate migrasi tambahan. Aplikasi menolak membuka DB dengan `user_version` lebih baru dari yang dikenalnya.
- **Catch-up berulang:** saat aplikasi dibuka dan sudah tidak terkunci, `app` memanggil `core::occurrences_until(rule, today)`, lalu `storage` menyisipkan hasilnya dan memajukan `next_due` dalam **satu transaksi**. Batas atas 1.000 kemunculan per aturan per pembukaan, agar aturan lama tidak membekukan startup; sisanya dilanjutkan pada pembukaan berikutnya.

### Laporan (semua di SQL)

| Laporan | Bentuk |
|---|---|
| Pengeluaran per kategori | `SUM(-amount_minor) … WHERE kind='expense' AND occurred_on BETWEEN ? AND ? GROUP BY category_id`; induk dijumlahkan dari anak |
| Tren bulanan | `GROUP BY substr(occurred_on,1,7)`, pemasukan dan pengeluaran terpisah |
| Kekayaan bersih | per akun `opening + SUM(amount_minor)`, dijumlahkan |
| Progres anggaran | pengeluaran kategori (+anak) bulan itu vs `limit_minor` |

---

## 6. API command Tauri (kontrak UI ↔ Rust)

Konvensi IPC: uang = string minor unit; tanggal = `"YYYY-MM-DD"`; id = angka; semua command mengembalikan `Result<T, AppError>`; `AppError = { code: string, message: string }` tanpa nilai uang atau catatan pengguna.

| Grup | Command |
|---|---|
| Kunci | `lock_status`, `unlock_with_pin(pin)`, `unlock_biometric_done()`, `lock()`, `set_pin(new, old?)` |
| Akun | `accounts_list(include_archived)`, `account_create`, `account_update`, `account_archive(id, archived)` |
| Kategori | `categories_list`, `category_create`, `category_update`, `category_archive` |
| Transaksi | `transactions_list(filter, cursor, limit)`, `transaction_get`, `transaction_create`, `transaction_update`, `transaction_delete`, `transfer_create`, `transfer_delete(group_id)` |
| Anggaran | `budget_set(category, month, limit)`, `budgets_progress(month)`, `budget_delete` |
| Berulang | `recurring_list`, `recurring_create`, `recurring_update`, `recurring_delete`, `recurring_catch_up()` |
| Laporan | `report_spending_by_category(from,to)`, `report_monthly_trend(from,to)`, `report_net_worth()` |
| Backup | `backup_export(passphrase)`, `backup_import(passphrase)` (path dipilih lewat dialog sistem, bukan dari UI) |
| Pengaturan | `settings_get`, `settings_set` |

- Selama terkunci, **semua** command selain `lock_status`/`unlock_*` mengembalikan `locked` (`AppState.db: Mutex<Option<Db>>`, `None` saat terkunci).
- **Tipe di sisi UI:** ditulis manual di satu berkas `src/lib/api.ts` (satu fungsi per command). Tidak memakai generator tipe di v1 (lihat K5). Perubahan command tanpa memperbarui `api.ts` ketahuan oleh `pnpm check` bila UI memakainya.
- Kapabilitas Tauri: hanya `core:default` untuk jendela utama + izin spesifik plugin yang dipakai. `opener` dihapus.

---

## 7. Kunci, enkripsi, dan kunci aplikasi

### 7.1 Kunci database

1. Saat pertama dijalankan: 32 byte acak (CSPRNG OS) = **kunci DB**.
2. Plugin `keystore` (Kotlin) membuat kunci **AES-256-GCM di Android Keystore** (tidak dapat diekspor; mencoba StrongBox, fallback ke TEE; `setUnlockedDeviceRequired(true)`), lalu membungkus kunci DB. Hasil bungkus disimpan di direktori privat aplikasi.
3. Saat membuka: unwrap → `PRAGMA key = "x'<64 hex>'"` (kunci mentah, melewati PBKDF2 SQLCipher sehingga startup cepat) → `zeroize` salinan di memori setelah dipakai.
4. Tanpa kunci Keystore, berkas DB tidak dapat dibuka (diuji di spike Fase 3).

### 7.2 Kunci aplikasi (PIN/biometrik)

**[KEPUTUSAN, lihat K2]** Kunci aplikasi adalah **gerbang UI + gerbang pembukaan DB**, bukan turunan kriptografis kunci DB:

- DB baru dibuka **setelah** unlock berhasil; saat aplikasi ke latar belakang, koneksi ditutup dan kunci dibuang dari memori.
- Verifier PIN (Argon2id + salt) dan penghitung percobaan gagal disimpan di **berkas kecil di luar DB** (DB belum terbuka saat PIN diperiksa). Percobaan gagal diberi jeda eksponensial.
- Biometrik lewat `tauri-plugin-biometric` (BiometricPrompt, dengan fallback PIN aplikasi).
- Kunci Keystore **tidak** disyaratkan autentikasi per-pakai (`setUserAuthenticationRequired(false)`), sehingga mengganti/menambah sidik jari tidak membuat kunci tidak valid dan **tidak menyebabkan kehilangan data**.

Konsekuensi yang jujur: penyerang yang sudah mengeksekusi kode di proses aplikasi (perangkat root) dapat melewati gerbang. Itu di luar model ancaman (bagian 8). Opsi penguatan di kemudian hari: mengikat kunci DB ke autentikasi pengguna (dengan risiko invalidasi, mitigasi lewat backup).

### 7.3 Layar

- `FLAG_SECURE` di jendela aktivitas utama (v1: seluruh aplikasi, bukan hanya layar saldo — lebih sederhana dan lebih aman; bisa dilonggarkan jika mengganggu).
- Disetel dari plugin Kotlin yang sama, bukan dengan menyunting berkas yang di-generate.

---

## 8. Model ancaman

**Aset:** data transaksi/saldo, kunci DB, file backup, PIN.

| # | Ancaman | Mitigasi | Risiko sisa |
|---|---|---|---|
| T1 | HP dicuri, terkunci | Enkripsi disk Android + SQLCipher + kunci di Keystore | Rendah |
| T2 | HP dicuri/dipinjam saat tidak terkunci | Kunci aplikasi (PIN/biometrik), auto-lock saat ke latar belakang, backoff PIN | Sedang: PIN lemah |
| T3 | Aplikasi lain membaca data | Sandbox Android, direktori privat, tanpa `exported` komponen yang tak perlu | Rendah |
| T4 | Ekstraksi `adb backup`/cloud backup | `allowBackup=false`, `dataExtractionRules`/`fullBackupContent` mengecualikan semuanya | Rendah |
| T5 | Screenshot/recents/screen recording | `FLAG_SECURE` | Rendah |
| T6 | Backup bocor | Argon2id + XChaCha20-Poly1305, passphrase dari pengguna | Bergantung kekuatan passphrase |
| T7 | File backup berbahaya/korup | AEAD menolak perubahan; impor memvalidasi tiap baris lewat aturan `core`; batas ukuran; transaksi DB utuh atau tidak sama sekali | Rendah |
| T8 | Konten jarak jauh / XSS di WebView | Tanpa INTERNET, CSP ketat, tanpa konten jarak jauh, kapabilitas minimal | Rendah |
| T9 | Rantai pasok dependensi | `cargo audit`, `cargo deny`, lockfile di-commit, dependensi minimal dan terdaftar | Sedang |
| T10 | Kebocoran lewat log | Aturan: tidak ada data keuangan di log; build rilis tanpa log verbose | Rendah |
| T11 | Bug hitung uang | Tipe `Money`, aritmatika checked, property test, review adversarial | Rendah |

**Di luar lingkup:** perangkat yang di-root/dikompromikan di level OS, malware dengan eksekusi kode di proses aplikasi, serangan fisik pada memori saat aplikasi terbuka, penyerang yang memiliki perangkat terbuka *dan* PIN.

---

## 9. Format backup

Pilihan **[KEPUTUSAN, K4]**: **ekspor logis** (baris per tabel), bukan salinan berkas SQLCipher.
Alasan: salinan berkas terenkripsi dengan kunci perangkat lama yang tidak ikut terbawa; ekspor logis bebas dari versi skema, divalidasi saat impor, dan tidak pernah menaruh plaintext di disk.

```
magic "MTBK" | format_version u16 | kdf_id u8 | argon2 m,t,p (u32 each) | salt[16] | nonce[24]
| ciphertext = XChaCha20-Poly1305( key = Argon2id(passphrase, salt), AAD = seluruh header,
                                    plaintext = JSON ter-versi {schema, settings, accounts, ... } )
```

- Parameter awal Argon2id: m = 64 MiB, t = 3, p = 1 (diuji waktunya di perangkat kelas bawah; disimpan di header sehingga dapat dinaikkan).
- Plaintext hanya ada di memori; dibatasi ukuran (mis. ≤ 64 MiB) sebelum didekripsi/diurai.
- Penulisan lewat Storage Access Framework (dialog "simpan sebagai"); pembacaan lewat pemilih dokumen.
- Impor: dekripsi → validasi → tulis ke DB **baru** dalam satu transaksi → tukar atomik; DB lama tidak disentuh jika gagal.
- Pengaturan kunci aplikasi (PIN) **tidak** ikut backup dan harus diatur ulang di instalasi baru.

---

## 10. Dependensi (usulan, perlu persetujuan sebelum ditambahkan)

| Crate / komponen | Untuk apa | Lisensi | Status pemeliharaan | Catatan |
|---|---|---|---|---|
| `tauri` 2 | Kerangka aplikasi | MIT/Apache-2.0 | Aktif | Membawa `reqwest`, `hyper`, `tokio` (lihat di bawah) |
| `rusqlite` 0.40 | Akses SQLite | MIT | Aktif | Fitur: `bundled-sqlcipher-vendored-openssl` (lihat R1) |
| `time` | Aritmatika kalender di `core` | MIT/Apache-2.0 | Aktif | Alternatif: tulis sendiri (rawan bug, ditolak) |
| `argon2` 0.6 | KDF backup & verifier PIN | MIT/Apache-2.0 | Aktif (RustCrypto) | |
| `chacha20poly1305` 0.11 | AEAD backup | MIT/Apache-2.0 | Aktif (RustCrypto) | |
| `zeroize` | Membersihkan kunci dari memori | MIT/Apache-2.0 | Aktif | |
| `tauri-plugin-biometric` 2.4 | Prompt biometrik | MIT/Apache-2.0 | Resmi Tauri | Hanya mobile |
| `tauri-plugin-dialog` 2.8 | Dialog simpan/buka (SAF) | MIT/Apache-2.0 | Resmi Tauri | Ke R5 |
| `tauri-plugin-fs` 2.7 | Baca/tulis URI `content://` | MIT/Apache-2.0 | Resmi Tauri | Hanya bila R5 mengharuskan |
| `proptest` (dev) | Property test `Money` | MIT/Apache-2.0 | Aktif | Hanya dev |
| SQLCipher (via bundled) | Enkripsi DB | BSD-style (Zetetic) | Aktif | Wajib atribusi |
| OpenSSL (vendored) | Kripto SQLCipher | Apache-2.0 | Aktif | Dikompilasi per ABI |

Versi stabil plugin (2.x) dipilih; seri 3.0.0-alpha **tidak** dipakai.

**Daftar izin crate jaringan (untuk CI Fase 4):** `reqwest`, `hyper`, `hyper-util`, `http`, `tokio`, `h2`-turunannya diizinkan **hanya** sebagai dependensi transitif `tauri`. Tindakan Fase 4: (1) periksa dengan `cargo tree -e features` apakah fitur yang membawa `reqwest` dapat dimatikan; (2) bila tidak, CI menegakkan "tidak ada crate jaringan *di luar* daftar ini" dan bahwa tidak ada kode di `crates/*` maupun `src-tauri/src` yang mengimpornya.

---

## 11. Hal yang belum pasti di Android **[RISIKO]**

| # | Risiko | Cara dibuktikan | Fase |
|---|---|---|---|
| R1 | `rusqlite` `bundled-sqlcipher` biasa butuh OpenSSL sistem; di Android perlu `bundled-sqlcipher-vendored-openssl` (kompilasi OpenSSL per ABI, butuh `perl`/`make`/NDK, build lama). Gagal untuk salah satu dari 4 ABI = blocker | Spike: kompilasi dan buka DB terenkripsi di arm64, armv7, x86, x86_64 | 3 |
| R2 | Keystore dari Rust: tidak ada plugin resmi. Perlu plugin Kotlin kecil; StrongBox tidak ada di semua perangkat | Spike di perangkat asli: generate, wrap, unwrap, DB tak bisa dibuka tanpa kunci | 3 |
| R3 | Alignment halaman 16 KB untuk pustaka native (syarat Android 15+/Play) pada `.so` hasil NDK | Periksa NDK versi baru + `zipalign -P 16`/`llvm-readelf` pada APK | 2–3 |
| R4 | `forbid(unsafe_code)` vs `mobile_entry_point` | Kompilasi target Android | 2 |
| R5 | Menulis/membaca URI `content://` dari SAF lewat plugin dialog/fs: perilaku di Android harus dibuktikan | Ekspor dan impor satu berkas di perangkat asli | 7 |
| R6 | `FLAG_SECURE` dan `allowBackup=false` ada di `src-tauri/gen/android` yang bisa ditimpa regenerasi | Taruh di plugin/template; tambah cek CI atas manifest hasil merge | 4, 7 |
| R7 | Versi WebView beragam; fitur CSS/JS modern mungkin tak ada di HP lama | Tetapkan `minSdk` (usulan 26) dan uji di perangkat kelas bawah | 6, 8 |
| R8 | Gerbang kunci: memanggil biometrik dari sisi UI berarti JS dapat memanggil `unlock` tanpa prompt. Dapat diterima karena UI = kode sendiri tanpa konten jarak jauh, tetapi perlu dicek apakah plugin dapat dipicu dari Rust | Telaah plugin di Fase 7 | 7 |
| R9 | JDK 21 terpasang, dokumen menyebut 17 | Jalankan `pnpm tauri android build` di Fase 2; pasang 17 bila gagal | 2 |
| R10 | SvelteKit (dari template) menambah beban dibanding Vite+Svelte polos | Ukur bundle di Fase 6; ganti bila angkanya membenarkan | 6 |

---

## 12. Keputusan yang butuh persetujuan pemilik

| # | Pertanyaan | Rekomendasi |
|---|---|---|
| K1 | **Mata uang**: satu mata uang per aplikasi di v1, atau multi-mata-uang? | Satu mata uang (dipilih saat pertama), kolom `currency` tetap ada untuk masa depan. Multi-mata-uang butuh kurs dan mengubah laporan/anggaran secara signifikan |
| K2 | **Kunci aplikasi** bukan kriptografis (gerbang UI + DB baru dibuka setelah unlock), kunci Keystore tidak mensyaratkan autentikasi | Setuju untuk v1; menghindari kehilangan data saat biometrik berubah |
| K3 | Menambah `time` ke `core` | Setuju |
| K4 | Backup berupa ekspor logis ter-enkripsi, bukan salinan berkas DB | Setuju |
| K5 | Tipe UI ditulis manual (`api.ts`), tanpa `specta`/`ts-rs` | Setuju untuk v1 (≈25 command); tinjau ulang bila membengkak |
| K6 | `FLAG_SECURE` untuk seluruh aplikasi, bukan hanya layar saldo | Setuju |
| K7 | Ganti `bundled-sqlcipher` (usulan dokumen) dengan `bundled-sqlcipher-vendored-openssl` bila spike menunjukkan perlu | Setuju bersyarat pada hasil R1 |
| K8 | `minSdk` | 26 (Android 8.0) |

---

## 13. Rencana per fase (penyesuaian dari roadmap v1)

| Fase | Penyesuaian yang timbul dari dokumen ini |
|---|---|
| 2 | Hapus `tauri-plugin-opener` dan `greet`; susun workspace (root `Cargo.toml`); `rust-toolchain`/`rustfmt`/`clippy`/`deny.toml`; periksa R3, R4, R9; **prasyarat belum ada:** Android SDK/NDK, `ANDROID_HOME`, `NDK_HOME`, 4 target Rust Android |
| 3 | Uji `Money` dulu; skema 1 migrasi (§5); spike R1, R2; hasil dicatat di dokumen ini |
| 4 | CSP ketat + `devCsp`; cek CI manifest rilis; daftar izin crate jaringan (§10); catch-up berulang (§5) |
| 5 | Satu command-group per prompt; `api.ts` diperbarui bersamaan |
| 7 | Plugin `keystore` & `FLAG_SECURE`; R5, R6, R8; backup §9 |
| 8 | Konfirmasi indeks dengan `EXPLAIN QUERY PLAN`; evaluasi saldo tersimpan; parameter Argon2id di perangkat rendah |

---

## 14. Catatan keputusan yang ditolak

- **Enkripsi di level aplikasi** (mengenkripsi nilai per kolom): ditolak sebagai default karena merusak agregasi SQL; tetap menjadi cadangan jika R1 gagal (enkripsi seluruh berkas DB saat disimpan, DB kerja di memori — dengan konsekuensi performa/ketahanan yang harus diukur).
- **ID UUID/ULID:** ditolak untuk v1; `INTEGER PRIMARY KEY` lebih kecil dan cepat. Sinkronisasi di masa depan akan menambahkan kolom ID global lewat migrasi.
- **Layanan latar belakang untuk transaksi berulang:** ditolak (aturan mode lokal); catch-up saat buka.
- **Crate migrasi (`rusqlite_migration`, dll.):** ditolak; `user_version` + SQL tersemat cukup.
