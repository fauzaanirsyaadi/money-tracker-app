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
-- Schema v1. See docs/ARCHITECTURE.md section 5 for the rationale.
--
-- Money columns are bounded to +/-(2^53 - 1), the same range mt-core enforces,
-- so SQL and Rust always agree. Dates must be canonical 'YYYY-MM-DD'.

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
) STRICT;

CREATE TABLE accounts (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
  kind TEXT NOT NULL CHECK (kind IN ('cash','bank','ewallet')),
  currency TEXT NOT NULL,
  opening_balance_minor INTEGER NOT NULL DEFAULT 0
    CHECK (opening_balance_minor BETWEEN -9007199254740991 AND 9007199254740991),
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1)),
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
) STRICT;

-- Two active accounts cannot share a name (case-insensitive); archiving frees it.
CREATE UNIQUE INDEX uq_accounts_active_name ON accounts(lower(name)) WHERE archived = 0;

CREATE TABLE categories (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
  kind TEXT NOT NULL CHECK (kind IN ('income','expense')),
  parent_id INTEGER REFERENCES categories(id) ON DELETE RESTRICT,
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1)),
  CHECK (parent_id IS NULL OR parent_id <> id)
) STRICT;

CREATE TABLE recurring_rules (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  category_id INTEGER NOT NULL REFERENCES categories(id),
  kind TEXT NOT NULL CHECK (kind IN ('income','expense')),
  amount_minor INTEGER NOT NULL CHECK (amount_minor BETWEEN -9007199254740991 AND 9007199254740991),
  note TEXT NOT NULL DEFAULT '',
  freq TEXT NOT NULL CHECK (freq IN ('daily','weekly','monthly','yearly')),
  every_n INTEGER NOT NULL DEFAULT 1 CHECK (every_n >= 1),
  anchor_day INTEGER NOT NULL CHECK (anchor_day BETWEEN 1 AND 31),
  start_on TEXT NOT NULL,
  end_on TEXT,
  next_due TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  CHECK ((kind = 'income' AND amount_minor > 0) OR (kind = 'expense' AND amount_minor < 0)),
  CHECK (date(start_on) IS start_on),
  CHECK (end_on IS NULL OR date(end_on) IS end_on),
  CHECK (date(next_due) IS next_due)
) STRICT;

CREATE TABLE transactions (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  category_id INTEGER REFERENCES categories(id),
  amount_minor INTEGER NOT NULL CHECK (amount_minor BETWEEN -9007199254740991 AND 9007199254740991),
  kind TEXT NOT NULL CHECK (kind IN ('income','expense','transfer_out','transfer_in')),
  note TEXT NOT NULL DEFAULT '' CHECK (length(note) <= 500),
  occurred_on TEXT NOT NULL,
  transfer_group_id INTEGER REFERENCES transactions(id),
  recurring_rule_id INTEGER REFERENCES recurring_rules(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  CHECK ((kind IN ('income','transfer_in')  AND amount_minor > 0)
      OR (kind IN ('expense','transfer_out') AND amount_minor < 0)),
  CHECK ((kind IN ('transfer_out','transfer_in'))
         = (transfer_group_id IS NOT NULL AND category_id IS NULL)),
  CHECK (kind IN ('transfer_out','transfer_in') OR category_id IS NOT NULL),
  CHECK (date(occurred_on) IS occurred_on)
) STRICT;

CREATE TABLE budgets (
  id INTEGER PRIMARY KEY,
  category_id INTEGER NOT NULL REFERENCES categories(id),
  month TEXT NOT NULL CHECK (month GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]'
                             AND date(month || '-01') IS month || '-01'),
  limit_minor INTEGER NOT NULL CHECK (limit_minor > 0 AND limit_minor <= 9007199254740991),
  UNIQUE (category_id, month)
) STRICT;

CREATE INDEX idx_tx_date     ON transactions(occurred_on);
CREATE INDEX idx_tx_account  ON transactions(account_id, occurred_on);
CREATE INDEX idx_tx_category ON transactions(category_id, occurred_on);
CREATE UNIQUE INDEX uq_tx_recurring
  ON transactions(recurring_rule_id, occurred_on) WHERE recurring_rule_id IS NOT NULL;

-- Transfers: two legs sharing transfer_group_id, which is the id of the
-- outgoing leg. The outgoing leg is inserted first with an explicit id
-- (the repository allocates it inside the same transaction); the incoming leg
-- must mirror it exactly. A transfer is deleted with one statement that
-- removes both legs, because the foreign key is checked at statement end.
CREATE UNIQUE INDEX uq_transfer_in_group
  ON transactions(transfer_group_id) WHERE kind = 'transfer_in';

CREATE TRIGGER trg_transfer_out_group AFTER INSERT ON transactions
WHEN NEW.kind = 'transfer_out' AND NEW.transfer_group_id IS NOT NEW.id
BEGIN
  SELECT RAISE(ABORT, 'transfer_out group id must equal its own id');
END;

CREATE TRIGGER trg_transfer_in_pair AFTER INSERT ON transactions
WHEN NEW.kind = 'transfer_in' AND NOT EXISTS (
  SELECT 1 FROM transactions o
  WHERE o.id = NEW.transfer_group_id
    AND o.kind = 'transfer_out'
    AND o.amount_minor = -NEW.amount_minor
    AND o.account_id <> NEW.account_id)
BEGIN
  SELECT RAISE(ABORT, 'transfer_in must mirror an outgoing leg');
END;

CREATE TRIGGER trg_transfer_legs_immutable
BEFORE UPDATE OF amount_minor, account_id, kind, transfer_group_id ON transactions
WHEN OLD.kind IN ('transfer_out','transfer_in')
BEGIN
  SELECT RAISE(ABORT, 'transfer legs cannot change; delete and recreate');
END;
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

- **Transfer** = dua baris (`transfer_out`, `transfer_in`) dalam satu transaksi DB, `transfer_group_id` = `id` baris keluar. Baris keluar disisipkan lebih dulu dengan `id` eksplisit (dialokasikan repository di transaksi yang sama); trigger memastikan baris masuk mencerminkan baris keluar (jumlah berlawanan, akun berbeda, satu per grup) dan melarang pengubahan jumlah/akun/jenis; transfer dihapus dengan satu `DELETE ... WHERE transfer_group_id = ?`. Dikecualikan dari laporan pengeluaran/pemasukan, termasuk dalam saldo akun. Menghapus/mengubah satu sisi memengaruhi pasangannya (dilakukan atomik di `storage`).
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
2. Plugin `keystore` (Kotlin) membuat kunci **AES-256-GCM di Android Keystore** (tidak dapat diekspor; mencoba StrongBox, fallback ke TEE; `setUnlockedDeviceRequired` ditunda, lihat bagian 15), lalu membungkus kunci DB. Hasil bungkus disimpan di direktori privat aplikasi.
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

**Crate jaringan (ditegakkan di CI sejak Fase 4):** `reqwest` masuk lewat `tauri` dan tidak pernah dipanggil kode aplikasi. Pembenarannya: tidak ada izin INTERNET di rilis, CSP memblokir origin jarak jauh, dan tidak ada kode kita yang memakainya. Penegakannya ada di `deny.toml` bagian `[bans]`: `reqwest`, `hyper`, `hyper-util`, `http-body-util`, `tower`, `tower-http`, `tokio-util`, `socket2`, dan `mio` hanya boleh hadir lewat induk yang disebut namanya (`wrappers`); klien HTTP, TLS, WebSocket, QUIC, dan DNS lain (`ureq`, `curl`, `isahc`, `surf`, `h2`, `rustls`, `native-tls`, `openssl`, `tungstenite`, `quinn`, `hickory-resolver`, dan seterusnya) dilarang sama sekali. Crate baru yang menarik salah satunya membuat `cargo deny check` gagal. Fitur `tauri` yang membawa `reqwest` belum dapat dimatikan (tidak ada fitur yang mengaturnya); dipantau tiap pembaruan `tauri`.

---

## 11. Hal yang belum pasti di Android **[RISIKO]**

| # | Risiko | Cara dibuktikan | Fase |
|---|---|---|---|
| R1 | `bundled-sqlcipher` biasa butuh OpenSSL sistem; di Android perlu `bundled-sqlcipher-vendored-openssl` | **Terselesaikan (Fase 3):** fitur vendored dipakai; `mt-storage` dan pustaka aplikasi ter-link untuk keempat ABI (lihat bagian 15) | 3 |
| R2 | Keystore dari Rust: perlu plugin Kotlin kecil; StrongBox tidak ada di semua perangkat | **Sebagian (Fase 3):** plugin `mt-keystore` bekerja di emulator (tingkat `software`). Tingkat perangkat keras (`tee`/`strongbox`) **belum terbukti** — perlu HP asli | 3 |
| R3 | Alignment halaman 16 KB untuk pustaka native | **Terselesaikan (Fase 3):** NDK r27 menghasilkan 4 KB; `src-tauri/build.rs` menambah `-z max-page-size=16384`; semua ABI terverifikasi `0x4000` | 2–3 |
| R4 | `forbid(unsafe_code)` vs `mobile_entry_point` | Kompilasi target Android | 2 |
| R5 | Menulis/membaca URI `content://` dari SAF lewat plugin dialog/fs: perilaku di Android harus dibuktikan | Ekspor dan impor satu berkas di perangkat asli | 7 |
| R6 | `FLAG_SECURE` dan `allowBackup=false` ada di `src-tauri/gen/android` yang bisa ditimpa regenerasi | Taruh di plugin/template; tambah cek CI atas manifest hasil merge | 4, 7 |
| R7 | Versi WebView beragam; fitur CSS/JS modern mungkin tak ada di HP lama | Tetapkan `minSdk` (usulan 26) dan uji di perangkat kelas bawah | 6, 8 |
| R8 | Gerbang kunci: memanggil biometrik dari sisi UI berarti JS dapat memanggil `unlock` tanpa prompt. Dapat diterima karena UI = kode sendiri tanpa konten jarak jauh, tetapi perlu dicek apakah plugin dapat dipicu dari Rust | Telaah plugin di Fase 7 | 7 |
| R9 | JDK 21 terpasang, dokumen menyebut 17 | **Terselesaikan (Fase 2):** build debug lokal berhasil dengan JDK 21 + Gradle 9.6.1 + NDK 27.2; CI memakai JDK 17 (belum terverifikasi) | 2 |
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

---

## 15. Hasil spike Fase 3 (2026-10-14)

Lingkungan: macOS arm64, Rust 1.98, NDK 27.2.12479018, JDK 21, `minSdk` 24 (generator Tauri; target K8 = 26 belum diterapkan), emulator Pixel 6 / Android 15 arm64 (`google_apis`).

### Yang berhasil

| Uji | Hasil |
|---|---|
| `Money`/`Currency` (`mt-core`) | 26 tes lulus: batas ±(2^53−1), overflow tanpa wrap, pembulatan setengah-menjauhi-nol, `allocate` largest-remainder, parse/format kanonik |
| Skema v1 + migrasi (`mt-storage`) | 10 tes lulus di host: `CHECK` tanda per `kind`, transfer, FK `RESTRICT`, unik berulang/anggaran, tolak skema lebih baru, berkas terenkripsi |
| SQLCipher + OpenSSL vendored, 4 ABI | `aarch64`, `armv7`, `i686`, `x86_64`: kompilasi dan link pustaka aplikasi **OK** (±40–46 dtk per ABI, API 26) |
| Alignment segmen LOAD | `0x4000` untuk keempat ABI (setelah perbaikan `build.rs`) |
| Plugin Keystore (Kotlin) | Kunci AES-256-GCM tak-dapat-diekspor; `wrap`/`unwrap` bekerja; blob 60 byte (12 IV + 32 kunci + 16 tag) |
| DB terenkripsi di perangkat | Di emulator: DB terbuka, `user_version = 1`, header **bukan** plaintext, kunci acak ditolak, blob terbungkus bukan kunci, terbuka kembali setelah proses dimulai ulang |
| Kunci tak terjangkau WebView | Plugin tidak mendaftarkan command; hanya API Rust (`MtKeystoreExt`) |

### Yang belum terbukti / gagal

| Hal | Status |
|---|---|
| Keystore berperangkat keras | Emulator melaporkan `software`. `tee`/`strongbox` hanya dapat dibuktikan di HP asli |
| Penolakan DB tanpa Keystore | Terbukti lewat kunci salah; skenario "kunci Keystore hilang/diinvalidasi" belum diuji di perangkat |
| `setUnlockedDeviceRequired(true)` | **Sengaja tidak dipakai** di spike: risiko tak terbaca pada perangkat tanpa layar kunci belum dipahami. Putuskan di Fase 7 |
| Pustaka rilis | Hanya build debug yang diukur. `.so` debug 145–164 MB, APK debug (arm64) ±285 MB. Ukuran rilis diukur di Fase 8 |
| Build 4-ABI via Gradle | Hanya `aarch64` dibangun sebagai APK; ABI lain hanya diuji sampai link |
| CI | Job `android` belum pernah berjalan; NDK 27.2 dan JDK 17 belum diuji di CI |

### Keputusan dari spike

1. `rusqlite` memakai `bundled-sqlcipher-vendored-openssl` (menjalankan K7).
2. Alignment 16 KB diatur di `src-tauri/build.rs`, bukan `.cargo/config.toml` (Tauri menyetel `CARGO_TARGET_*_RUSTFLAGS`, yang mengalahkan konfigurasi berkas).
3. Plugin Keystore tidak mengekspos command ke JS; kunci hanya dapat diminta dari Rust.
4. Kunci DB dibuat dengan `getrandom` di Rust, lalu dibungkus; berkas `db.key` ditulis dengan tulis-lalu-ganti-nama.
5. Bila `money.db` ada tetapi `db.key` hilang, aplikasi **menolak** membuat kunci baru (mencegah data tak terbaca diam-diam menimpa).
6. `cargo check --target <android>` polos kini membutuhkan toolchain NDK di `PATH` (OpenSSL dikompilasi dari sumber); build lewat `pnpm tauri android build` menyetelnya sendiri. Cek cepat Android di CI dilakukan lewat job `android`, bukan `cargo check` terpisah.
7. Perintah uji mandiri `storage_selftest` hanya ada di build debug (`cfg(debug_assertions)`), tanpa kunci atau data keuangan di keluaran; dihapus bersama pemasangan alur buka-kunci di Fase 7.

### Perbaikan setelah review adversarial (kelompok 1 dan 2)

| ID | Temuan | Perbaikan |
|---|---|---|
| H1 | Build tanpa SQLCipher membuat DB plaintext tanpa error | `Db::init` memeriksa `PRAGMA cipher_version` sebelum kunci dipasang; `StorageError::EncryptionUnavailable` |
| H2 | `allowBackup` default `true` | `allowBackup=false`, `fullBackupContent=false`, `data_extraction_rules.xml` (cloud-backup dan device-transfer: semua domain dikecualikan) |
| M1 | Jumlah uang tidak dibatasi di DB | `CHECK` ±(2^53−1) pada `amount_minor`, `opening_balance_minor`, `limit_minor` |
| M2 | Tanggal tidak divalidasi | `CHECK (date(x) IS x)`; perhatikan `IS`, karena `date()` yang tidak valid menghasilkan NULL dan `CHECK` meloloskan NULL |
| M3 | Pasangan transfer tidak dijamin | FK `transfer_group_id`, indeks unik baris masuk, 3 trigger (lihat bagian 5) |
| M4 | Overflow wrap diam-diam di rilis | `overflow-checks = true` di `[profile.release]` |
| M6 | Race pembuatan kunci | `KEY_LOCK` di `vault.rs`; tes konkurensi (gagal 15/15 tanpa lock, 0/15 dengan lock) |
| L11 | Kategori menjadi induk dirinya sendiri | `CHECK (parent_id <> id)` |

Migrasi 0001 diubah di tempat karena belum ada data di luar perangkat pengembangan. Setelah rilis pertama, perubahan skema hanya lewat migrasi baru.

### Temuan review yang belum ditangani

| ID | Temuan | Rencana |
|---|---|---|
| M5 | Kunci ada tetapi `money.db` hilang/kosong: DB baru dibuat diam-diam | Fase 7 (alur buka-kunci) |
| L1 | `Debug` pada `Money` dapat mencetak jumlah | Fase 5, saat DTO/log dirancang |
| L2 | `Currency::new` menerima eksponen yang salah untuk kode dikenal | Fase 5 |
| L3 | `unwrap` Keystore dapat membuat kunci baru bila alias hilang | Fase 7 |
| L4 | Salinan kunci dalam hex/stack tidak di-zeroize | Batas yang diketahui, dicatat |
| L5 | FileProvider membuka `external-path` dan `cache-path` | Fase 7 |
| L6 | CI: aksi tidak di-pin ke SHA, tanpa `timeout-minutes`, komentar job `android` | Setelah CI pertama berjalan |
| L7–L9 | `mt-core` belum dipakai `mt-storage`; `allow(dead_code)` tingkat modul; migrasi membaca versi sebelum `BEGIN` | Fase 5 / 7 |

---

## 16. Hasil Fase 4: penjaga mode lokal (2026-10-14)

### Aturan 5.1 dan penegakannya

| Aturan | Penegak | Diuji |
|---|---|---|
| 1. Rilis tanpa izin INTERNET | `src/main` tidak mendeklarasikannya; hanya `src/debug` yang menambahnya. `scripts/check_offline.py manifest` memakai **daftar izin yang diizinkan** (bukan hanya melarang INTERNET), serta mewajibkan `allowBackup=false`, `usesCleartextTraffic != true`, tidak `debuggable`, dan `dataExtractionRules` | Uji negatif: INTERNET disisipkan ke manifest utama → CI gagal. APK rilis arm64 asli: izin hanya `DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION` |
| 2. CSP memblokir origin jarak jauh | `app.security.csp` (lihat di bawah); `check_offline.py config` menolak `null`, wildcard, host jarak jauh, `unsafe-eval`, dan `unsafe-inline` selain untuk gaya | Di emulator: `fetch`, gambar, script, iframe jarak jauh dan `eval` diblokir (5 pelanggaran CSP tercatat); halaman dan IPC tetap berjalan |
| 3. Tanpa analitik/crash reporter | `deny.toml` `[bans]` + `check_offline.py source` (API jaringan dan URL di kode kita) | 14 tes skrip; uji negatif `cargo deny` untuk `ureq` dan `reqwest` oleh induk tak dikenal |
| 4. Berulang dihitung saat buka | `mt_core::Schedule::catch_up` + `Db::catch_up_recurring`, tanpa layanan latar | 13 tes core, 9 tes storage |
| 5. Data keluar hanya lewat backup pilihan pengguna | `allowBackup=false` + aturan ekstraksi (Fase 3 perbaikan) | Dicek oleh penjaga manifest |

CSP rilis:

```
default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline';
img-src 'self' data:; font-src 'self'; connect-src ipc: http://ipc.localhost;
base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'
```

`style-src-attr 'unsafe-inline'` hanya untuk atribut `style` (SvelteKit membungkus isi dengan `display: contents`). Skrip inline SvelteKit di-hash oleh Tauri saat build, jadi `script-src` tetap tanpa `unsafe-inline`. `devCsp` terpisah mengizinkan hanya `localhost:1420`.

### Catch-up transaksi berulang

- Kemunculan dihitung **dari indeks** (`start + k × every`), tidak dengan menambah ke tanggal sebelumnya, sehingga tanggal 31 tidak "melorot" ke 28 setelah Februari. Februari 29 di tahun kabisat dan jatuh ke 28 di tahun biasa.
- `next_due` harus tepat berada di jadwal; bila tidak, aturan dilaporkan `rules_invalid` dan dilewati tanpa memblokir aturan lain.
- Satu transaksi `IMMEDIATE`: semua transaksi baru dan `next_due` maju bersama, atau tidak sama sekali. Idempoten (indeks unik aturan+tanggal, `INSERT OR IGNORE`).
- Batas per aturan per pembukaan (1.000 sesuai rencana); sisanya dilanjutkan pada pembukaan berikutnya. Aturan di akun/kategori yang diarsipkan tidak disentuh sampai dipulihkan.
- Aturan melewati `end_on` dinonaktifkan (`active = 0`).
- **Belum terpasang ke alur aplikasi:** fungsi ini baru dipanggil dari pengujian. Pemanggilan saat aplikasi dibuka (setelah buka-kunci) disambungkan di Fase 5/7 bersama command `recurring_catch_up`.

### Ukuran dasar rilis (arm64, tanpa tanda tangan)

| Berkas | Ukuran |
|---|---|
| `app-universal-release-unsigned.apk` (arm64 saja) | 5,9 MB |
| `app-universal-debug.apk` (arm64) | 285 MB (simbol debug) |

Angka rilis hanya mencakup layar "Hello" + SQLCipher/OpenSSL; menjadi dasar Fase 8.

### Catatan

- `src-tauri/gen/android` adalah keluaran generator. Perubahan manifest di sana (`allowBackup`, tanpa INTERNET, `src/debug`) hilang bila `tauri android init` dijalankan ulang; penjaga CI akan menangkapnya.
- Penjaga manifest berjalan di job `android` lewat `./gradlew :app:processUniversalReleaseMainManifest`, bukan membangun APK rilis penuh di CI (hemat menit). APK rilis penuh diverifikasi manual di atas.

---

## 17. Fitur 1: akun (Fase 5)

### Lapisan

| Lapisan | Isi |
|---|---|
| `mt-core` (`account.rs`) | `AccountKind`, `normalize_account_name` (trim, 1..=80 karakter, tanpa karakter kontrol; panjang dihitung dalam karakter seperti `length()` SQLite), `AccountDraft` (hanya bisa dibuat tervalidasi; mata uang dibawa oleh `Money`) |
| `mt-storage` (`accounts.rs`) | `create_account`, `update_account`, `set_account_archived`, `account`, `accounts(include_archived)`, `app_currency`. Saldo = `opening_balance_minor + SUM(transactions.amount_minor)` dihitung SQLite pada setiap baca; tidak ada saldo tersimpan |
| `src-tauri` (`accounts.rs`, `state.rs`, `error.rs`) | Command `accounts_list`, `account_create`, `account_update`, `account_archive`; `AppState { db: Mutex<Option<Db>> }`; `AppError { code, message }` |
| UI (`src/lib/api.ts`, `money.ts`, `routes/+page.svelte`) | Satu modul pemanggil command; BigInt untuk uang; layar daftar, tambah, ubah, arsip |

### Aturan

- Nama rekening aktif unik tanpa memedulikan huruf besar/kecil (`uq_accounts_active_name`, indeks parsial `WHERE archived = 0`). Mengarsipkan membebaskan nama; memulihkan yang bentrok ditolak (`name_in_use`).
- Mata uang tetap (K1): satu mata uang per instalasi (`settings.currency`, default IDR sampai ada pilihan di UI). Akun tidak dapat berpindah mata uang.
- Saldo awal boleh negatif (kartu kredit, utang).
- Rekening tidak dihapus, hanya diarsipkan; saldonya tetap dihitung dan masuk kekayaan bersih.
- Saldo di luar ±(2^53−1) dilaporkan sebagai `amount_out_of_range`, tidak dipotong atau di-wrap.

### Kontrak IPC

Uang: string satuan minor (`"1250000"`, `"-250000"`), 1–16 digit, tanpa tanda `+`/spasi/desimal; rentang diperiksa `Money`. DTO menyertakan `currency` dan `exponent` agar UI memformat.

| Kode galat | Arti |
|---|---|
| `unavailable` | Penyimpanan gagal dibuka (atau terkunci, setelah Fase 7) |
| `invalid_input` | Nama kosong/panjang/berkarakter kontrol, jenis tidak dikenal, jumlah tidak terbaca |
| `name_in_use` | Nama sudah dipakai rekening aktif lain |
| `not_found`, `currency_mismatch`, `amount_out_of_range`, `internal` | sesuai namanya |

Muatan galat tidak pernah memuat nama, jumlah, atau catatan (ada tesnya).

### Pengujian

| Tingkat | Jumlah | Mencakup |
|---|---|---|
| core | 4 | nama, panjang dalam karakter, kode jenis, draft |
| storage | 12 | saldo = pembukaan + transaksi, per akun, negatif, urutan, arsip, ubah, unik huruf-besar-kecil, tak ditemukan, mata uang tetap, batas rentang |
| app | 9 | string uang ketat, DTO camelCase, kode galat, duplikat, ubah/arsip, tanpa data pengguna di galat, penyimpanan tak tersedia |
| UI (`pnpm test`) | 7 | format/parsing BigInt, desimal, rentang, tanpa nol negatif |
| E2E emulator | 10 skenario | membuat, form direset, duplikat berbeda huruf, jumlah tak terbaca, ubah (isi awal terisi), negatif, total aktif, arsip menyembunyikan, pulihkan, persisten setelah restart proses |

### Batasan yang diketahui

- Sampai ada kunci aplikasi (Fase 7), basis data dibuka saat startup. Bila gagal, UI menampilkan "Penyimpanan tidak tersedia".
- Build **desktop** tidak punya Keystore, jadi `pnpm tauri dev` di macOS menampilkan pesan itu. Iterasi UI cepat memakai emulator, atau dapat ditambahkan kunci khusus-debug di desktop bila diperlukan (belum dilakukan).
- Tampilan baru fungsional; sistem desain ada di Fase 6.
- Ikon, urutan manual (`sort_order` ada tetapi belum dapat diubah), dan pilihan mata uang menunggu fase berikutnya.

---

## 18. Fitur 2: transaksi dan kategori (Fase 5)

### Lapisan

| Lapisan | Isi |
|---|---|
| `mt-core` | `FlowKind` (income/expense), `TransactionDraft` (jumlah = besaran positif; tanda diturunkan dari jenis sekali saja), `TransferDraft` (dua akun berbeda, jumlah positif), `CategoryDraft`, normalisasi catatan (≤500 karakter, boleh kosong) |
| `mt-storage` | Migrasi 0002 (indeks unik kategori aktif + 12 kategori bawaan); `create/update/delete_transaction`, `create_transfer`, `transactions(filter, cursor, limit)`, `categories`, `create_category` |
| `src-tauri` | `categories_list`, `category_create`, `transactions_list`, `transaction_create`, `transaction_update`, `transaction_delete`, `transfer_create` |
| UI | `api.ts` diperluas, `dates.ts`, layout dengan navigasi, layar `/transactions` (form, filter, daftar berhalaman) |

### Aturan

- **Tanda**: pengeluaran disimpan negatif, pemasukan positif; UI dan command hanya mengirim besaran positif.
- **Validasi di dalam satu transaksi `IMMEDIATE`**: rekening ada, belum diarsipkan, dan mata uangnya sama; kategori ada, belum diarsipkan, dan jenisnya cocok. Gagal berarti tidak ada yang tertulis.
- **Ubah**: hanya income/expense. Rekening atau kategori yang diarsipkan boleh tetap melekat pada catatan lama, tetapi catatan tidak dapat dipindahkan ke yang diarsipkan. Transfer tidak diubah (hapus lalu buat ulang); tombol Ubah tidak tampil.
- **Transfer**: `id = MAX(id)+1` dialokasikan dalam transaksi yang sama; kaki keluar menyebut dirinya sebagai grup; kedua kaki dihapus dengan satu `DELETE ... WHERE transfer_group_id = ?` dari kaki mana pun. Kekayaan bersih tidak berubah (diuji).
- **Daftar**: urut `occurred_on DESC, id DESC`; paging keyset (`(occurred_on < ? OR (occurred_on = ? AND id < ?))`), satu baris ekstra menentukan ada-tidaknya halaman berikut; `limit` dijepit 1..=200 (bawaan 50).
- **Filter**: rekening, kategori (induk mencakup sub-kategorinya), jenis (`income`/`expense`/`transfer` = kedua kaki), rentang tanggal inklusif, dan pencarian pada catatan atau nama kategori. `%`, `_`, dan `\` diperlakukan literal (`ESCAPE`); LIKE tidak peka huruf besar-kecil hanya untuk ASCII.
- **Kategori**: maksimal satu tingkat, jenis anak = jenis induk, nama aktif unik per jenis dan induk.

### Pengujian

| Tingkat | Baru | Mencakup |
|---|---|---|
| core | 10 | tanda, jumlah > 0, catatan, transfer dua akun, kategori |
| storage | 21 + 1 | tanda dan saldo, kecocokan jenis, arsip/mata uang/tak ditemukan, atomik, ubah, tak berpindah ke arsip, hapus, transfer dan kedua kaki, urutan dan tie-breaker, paging 137 baris tanpa duplikat, jepit limit, semua filter, pencarian literal, filter + cursor; **upgrade DB v1 → v2** |
| app | 10 | DTO bertanda, kode galat stabil, ubah/hapus, transfer, paging via cursor DTO, filter, camelCase, kategori, arsip |
| UI | 3 | tanggal lokal dan format |
| E2E emulator | 14 | upgrade pada data nyata, buat pengeluaran/pemasukan/transfer lewat form, saldo, pencarian, filter, ubah, hapus dua sisi dengan konfirmasi, tolak rekening arsip, paging 120 baris |

### Batasan yang diketahui

- Daftar transfer menampilkan kedua kaki (satu per rekening) tanpa menggabungkannya; sistem desain di Fase 6 dapat menggabungkannya secara visual.
- Kategori: belum ada ubah nama/arsip; nama bawaan berbahasa Indonesia.
- Pencarian `LIKE '%x%'` memindai baris yang lolos filter lain; angka untuk 100 ribu baris diukur di Fase 8 (`EXPLAIN QUERY PLAN`).
- Tidak ada batas jumlah baris yang dimuat UI selain tombol "Muat lebih banyak"; daftar tervirtualisasi menunggu Fase 6.
