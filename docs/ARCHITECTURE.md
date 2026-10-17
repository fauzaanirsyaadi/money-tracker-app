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
- Biometrik lewat plugin sendiri `mt-platform` (BiometricPrompt AndroidX, dengan fallback PIN aplikasi). *Direvisi di Fase 7A: bukan `tauri-plugin-biometric`, agar hasil prompt tidak melewati JS; lihat §23 dan SECURITY.md.*
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
| ~~`tauri-plugin-biometric` 2.4~~ → `androidx.biometric` 1.1.0 | Prompt biometrik (lewat plugin `mt-platform`) | Apache-2.0 | Google, stabil | Hanya mobile. Diganti di Fase 7A (hasil tidak boleh lewat JS) |
| `tauri-plugin-dialog` 2.8 | Dialog simpan/buka (SAF) | MIT/Apache-2.0 | Resmi Tauri | Ke R5 |
| `tauri-plugin-fs` 2.7 | Baca/tulis URI `content://` | MIT/Apache-2.0 | Resmi Tauri | Hanya bila R5 mengharuskan |
| `proptest` (dev) | Property test `Money` | MIT/Apache-2.0 | Aktif | Hanya dev. **Disetujui pemilik di Fase 8** (fitur bawaan dimatikan, hanya `std`; menambah `rand`, `rand_core`, `rand_xorshift`, `unarray` ke pohon dev) |
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
| `setUnlockedDeviceRequired(true)` | **Sengaja tidak dipakai** di spike: risiko tak terbaca pada perangkat tanpa layar kunci belum dipahami. **Diputuskan di Fase 7A: dipakai** (§23) |
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
7. Perintah uji mandiri `storage_selftest` hanya ada di build debug (`cfg(debug_assertions)`), tanpa kunci atau data keuangan di keluaran; **dihapus di Fase 7A** bersama pemasangan alur buka-kunci.

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

Diperbarui 2026-10-14 sebelum Fase 7. Tidak ada yang diperbaiki di sini; ini daftar kerja. Nomor baris mengacu pada commit `debde43`.

| ID | Tingkat | Temuan | Lokasi | Rencana |
|---|---|---|---|---|
| M5 | Sedang | **Selesai di Fase 7C (§25).** Kunci ada tetapi `money.db` hilang/kosong: `Db::open` membuat DB baru yang kosong tanpa kabar (kebalikannya, DB ada tanpa kunci, sudah ditolak lewat `KeyMissing`). Pengguna melihat data hilang tanpa penjelasan. | `src-tauri/src/vault.rs:85-91` (cabang kunci ada), `crates/storage/src/db.rs:30` (`Db::open`) | Fase 7 langkah C: tolak dan laporkan "database hilang" bila `db.key` ada; tawarkan restore dari backup |
| L1 | Rendah | **Selesai di 7C.** `#[derive(Debug)]` pada `Money` mencetak jumlah, bisa bocor ke log lewat `{:?}` | `crates/core/src/money.rs:12` | Langkah C: `Debug` manual yang menyamarkan nilai; tes bahwa `format!("{:?}")` tidak memuat angka |
| L2 | Rendah | **Selesai di 7C.** `Currency::new` menerima eksponen yang salah untuk kode yang dikenal (mis. `IDR` dengan eksponen 2) | `crates/core/src/currency.rs:49` | Langkah C: validasi terhadap tabel kode dikenal |
| L3 | Rendah | **Selesai di 7C.** `getKey` mengembalikan null bila alias hilang lalu `key()` **membuat kunci baru**; `unwrap` lalu gagal dengan kesalahan GCM yang menyesatkan, dan kunci baru menimpa kemungkinan pemulihan | `src-tauri/plugins/mt-keystore/.../KeyVault.kt:20,46` | Langkah C: `unwrap` tidak pernah membuat kunci; hanya `wrap` yang boleh |
| L4 | Rendah | Salinan kunci sebagai hex/stack tidak di-zeroize sepenuhnya | `crates/storage/src/db.rs:111` (literal sudah `Zeroizing`; salinan pada tumpukan SQLCipher/Kotlin tidak terjangkau) | Batas yang diketahui; dicatat di SECURITY.md |
| L5 | Rendah | **Selesai di 7C.** FileProvider mengekspos `external-path` dan `cache-path` seluruhnya | `src-tauri/gen/android/app/src/main/res/xml/file_paths.xml:3-4` | Langkah B: sempitkan ke direktori ekspor khusus atau hapus FileProvider bila SAF cukup |
| L6 | Rendah | CI: aksi tidak di-pin ke SHA, tanpa `timeout-minutes`, komentar job `android` | `.github/workflows/ci.yml:23-106` | Setelah CI pertama berjalan |
| L7 | Info | `mt-core` belum dipakai `mt-storage` | — | **Selesai** di Fase 5 (`mt-storage` memakai `mt_core` di `categories.rs`, `transactions.rs`) |
| L8 | Rendah | **Selesai di 7C** (tidak ada lagi `allow(dead_code)` tingkat modul). `allow(dead_code)` tingkat modul | `src-tauri/src/vault.rs:2` | Langkah C: persempit ke item, atau hapus setelah kunci dibuka dari alur kunci aplikasi |
| L9 | Rendah | **Selesai di 7C.** Migrasi membaca `user_version` sebelum `BEGIN IMMEDIATE`; dua proses bisa sama-sama menjalankan migrasi yang sama | `crates/storage/src/db.rs:61` vs `:71` | Langkah C: baca versi di dalam transaksi |
| L10 | — | **Tidak ada**: daftar review asli berhenti di L9 (L7–L9 digabung), tidak ada temuan L10 | — | Bila review lain memakai L10, mohon sebut isinya |

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
- **Terpasang (fitur 3, §19):** UI memanggil `recurring_catch_up` sekali saat aplikasi dibuka. Setelah ada kunci aplikasi (Fase 7), pemanggilan terjadi setelah buka-kunci.

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

---

## 19. Fitur 3: anggaran dan transaksi berulang (Fase 5)

### Anggaran

- Satu batas per (kategori pengeluaran, bulan) di tabel `budgets`; menyimpan lagi untuk pasangan yang sama mengganti batas (upsert, satu baris).
- **Pengeluaran dihitung dari transaksi `expense` pada bulan itu** (`YYYY-MM-01` s.d. akhir bulan, inklusif). Pemasukan dan transfer tidak pernah dihitung. Anggaran pada kategori induk juga menghitung sub-kategorinya; anggaran sub-kategori tidak menghitung induknya.
- Progres (`mt_core::progress`): `used_bp` (basis point, dibulatkan setengah-menjauhi-nol), `remaining = limit − spent` (negatif bila terlampaui), dan status:
  - `warning` bila pemakaian **≥ 80 %**, `over` bila **> 100 %** (tepat 100 % masih `warning`).
  - Status memakai aritmetika bilangan bulat eksak, bukan `used_bp` yang dibulatkan: 79.999 dari 100.000 menampilkan 80,00 % tetapi tetap `ok` (ditemukan oleh tes).
- Hanya kategori pengeluaran yang aktif yang dapat dianggarkan; mata uang harus sama dengan instalasi.
- UI: penelusuran bulan, bilah progres, spanduk "hampir habis"/"terlampaui", ubah batas, hapus dengan konfirmasi dua langkah.

### Transaksi berulang

| Hal | Aturan |
|---|---|
| Pembuatan | Jadwal dari tanggal mulai; `anchor_day` = hari pada tanggal mulai, jadi tanggal mulai selalu salah satu kemunculan. `end_on ≥ start_on`. Rekening/kategori divalidasi seperti transaksi biasa |
| Yang dapat diubah | Rekening, kategori (jenis sama), jumlah, catatan, tanggal berakhir. **Jadwal dan jenis tidak dapat diubah**: mengubahnya akan menulis ulang arti "jatuh tempo" bagi transaksi yang sudah dibuat; caranya hapus lalu buat ulang |
| Jeda | Menghentikan pembuatan. **Lanjutkan melewati celah**: `next_due` menjadi kemunculan pertama pada/setelah hari ini (atau tetap bila belum jatuh tempo), jadi jeda panjang tidak mengisi mundur transaksi yang sengaja tidak diinginkan |
| Aturan berakhir | Setelah melewati `end_on` aturan dinonaktifkan dan tidak dapat dilanjutkan |
| Hapus | Aturan dihapus; transaksi yang sudah dibuat tetap ada dan tidak lagi tertaut |
| Catch-up | UI memanggil `recurring_catch_up(today)` sekali saat aplikasi dibuka; banner menyebut jumlah transaksi yang dicatat. Idempoten dan dibatasi 1.000 per aturan per pembukaan |

**Tanggal "hari ini"** hanya diketahui UI (kalender lokal perangkat). Backend menerimanya hanya bila berjarak ≤ 2 hari dari tanggal UTC (zona waktu UTC−12..UTC+14), agar nilai salah atau berbahaya tidak menghasilkan transaksi bertahun-tahun.

### Perintah baru

`budgets_list(month)`, `budget_set`, `budget_delete`, `recurring_list`, `recurring_create`, `recurring_update`, `recurring_set_active(id, active, today)`, `recurring_delete`, `recurring_catch_up(today)`.

### Pengujian

| Tingkat | Baru | Mencakup |
|---|---|---|
| core | 14 | bulan (parse ketat, awal/akhir bulan, tahun kabisat), progres, ambang eksak, `first_on_or_after` (termasuk kesesuaian dengan catch-up selama 2 tahun), draft/edit aturan |
| storage | 20 | anggaran: bulan, induk/anak, pemasukan dan transfer tidak dihitung, ambang, upsert, kategori tak valid, hapus; aturan: buat, validasi, ubah, jeda/lanjut tanpa mengisi mundur, aturan selesai, hapus menjaga transaksi, urutan |
| app | 11 | DTO, kode galat, kewajaran tanggal, catch-up sekali, jeda/lanjut/hapus, ubah |
| UI | 2 | geser bulan, format bulan |
| E2E emulator | 22 | anggaran: layar, buat, hitung transaksi lama, 85 % peringatan, 105 % terlampaui dan sisa negatif, bulan terpisah, ubah batas satu baris, hapus berkonfirmasi; berulang: buat, tanpa transaksi dulu, catch-up 4 lalu 0, jeda dan lanjut, kolom jadwal terkunci, ubah jumlah; **catch-up saat aplikasi dibuka ulang**: banner "3 transaksi berulang dicatat", tanggal tepat, tanpa duplikat, pembukaan berikutnya senyap |

### Batasan yang diketahui

- Satu putaran E2E menunjukkan ekspektasi tes saya salah (17 % padahal 21 %); aplikasi benar. Diperbaiki di tes, bukan di kode.
- Anggaran tidak dibawa otomatis ke bulan berikutnya (harus diatur per bulan); salin-dari-bulan-lalu dapat ditambahkan di Fase 6.
- Aturan berulang tidak punya "lewati satu kemunculan"; hapus transaksi yang dicatat bila tidak diinginkan.
- Notifikasi anggaran hanya berupa spanduk di layar Anggaran (tanpa notifikasi sistem, sesuai mode lokal).

---

## 20. Fitur 4: laporan (Fase 5)

Semua agregasi dilakukan di SQLite; Rust hanya melakukan pembulatan bilangan bulat dan mengisi bulan kosong; UI hanya memformat dan menggambar (aturan lapisan §2 butir 5).

### Tiga laporan

| Laporan | Command | Definisi |
|---|---|---|
| Pengeluaran per kategori | `report_spending(from, to)` | Jumlah `-amount_minor` untuk `kind = 'expense'`, periode inklusif, dikelompokkan per kategori lalu digulung ke induknya. Tiap induk menyertakan daftar anak, jumlah transaksi, dan `share_bp`. Urut dari terbesar, lalu nama |
| Tren bulanan | `report_trend(from, to)` | Per bulan: pemasukan, pengeluaran, `net = pemasukan − pengeluaran`, dan **kekayaan bersih akhir bulan**. Bulan tanpa aktivitas tetap muncul dengan arus nol. Maksimal 120 bulan |
| Kekayaan bersih | `report_net_worth()` | `Σ saldo awal + Σ semua transaksi`, per rekening dan total. Rekening yang diarsipkan tetap dihitung karena masih menyimpan uang |

### Aturan bersama

- **Transfer tidak pernah menjadi pemasukan atau pengeluaran** (hanya memindahkan uang antar rekening sendiri) dan membatalkan dirinya pada kekayaan bersih.
- **Saldo awal dianggap sudah ada sebelum bulan pertama** pada tren; saldo awal tidak punya tanggal.
- `share_bp` dibulatkan per baris (setengah-menjauhi-nol), sehingga kolom bisa berjumlah 99,99–100,01 %. Bila diperlukan total tepat 100 %, gunakan `Money::allocate` untuk tampilan.
- Penjumlahan memakai `i128` sebelum dikembalikan ke `Money`, jadi kelebihan rentang dilaporkan (`InvalidData`), tidak di-wrap.
- Titik terakhir tren sama dengan `report_net_worth().total` bila tidak ada transaksi bertanggal masa depan (diuji).

### UI

Layar **Laporan**: kekayaan bersih dan rincian rekening; pengeluaran per kategori dengan penelusuran bulan, bilah proporsional, dan rincian anak yang dapat dibuka; tren 6 bulan dengan bilah pemasukan/pengeluaran, net, dan kekayaan. Bilah memakai `percentOf` berbasis BigInt dan CSS biasa (tanpa pustaka grafik), jadi bundel tetap kecil.

### Pengujian

| Tingkat | Baru | Mencakup |
|---|---|---|
| core | 1 | aritmetika bulan (`plus_months`, `index`) |
| storage | 14 | pengelompokan, urutan, pembagian, gulungan anak, anak tunggal, periode inklusif, tanpa pemasukan dan transfer, periode kosong, bulan kosong dan kekayaan terbawa, transaksi sebelum jendela, transfer netral, kecocokan dengan laporan kekayaan, validasi rentang, negatif dan terarsip |
| app | 5 | DTO string, periode salah, tren bertanda, rentang tak valid, kekayaan |
| UI | 4 | skala batang eksak, maks, format persen |
| E2E emulator | 8 | **oracle independen**: kekayaan, pengeluaran Oktober (induk dan anak), tren 6 bulan termasuk kekayaan akhir bulan, dihitung ulang dari seluruh transaksi mentah dan dicocokkan; layar; rincian; bulan lain |

### Batasan yang diketahui

- Performa pada 100 ribu baris diukur di Fase 8 (`EXPLAIN QUERY PLAN`); kueri memakai `idx_tx_date`, `idx_tx_category`, dan `idx_tx_account`.
- Satu mata uang (K1); laporan tidak mengonversi.
- Belum ada ekspor laporan atau filter per rekening.

---

## 21. Status Fase 5

| Fitur | Status |
|---|---|
| 1 Akun | Selesai (§17) |
| 2 Transaksi, kategori, transfer, daftar berhalaman | Selesai (§18) |
| 3 Anggaran dan transaksi berulang | Selesai (§19) |
| 4 Laporan | Selesai (§20) |

Ruang lingkup v1 yang belum ada: kunci aplikasi (PIN/biometrik), `FLAG_SECURE`, backup dan restore terenkripsi (semuanya Fase 7), serta sistem desain dan input 2 ketukan (Fase 6).

---

## 22. Fase 6: UI dan sistem desain (2026-10-14)

Gaya: netral dan bersih, huruf sistem (tanpa aset jarak jauh), tanpa efek yang mahal bagi GPU. Seluruh teks antarmuka berbahasa Indonesia.

### Token dan komponen

| Hal | Isi |
|---|---|
| Token (`src/app.css`) | Warna terang dan gelap sebagai variabel CSS; gelap mengikuti OS atau pilihan pengguna (`data-theme`); skala tipografi 12–36 px; skala jarak 4 px; radius; target sentuh 48 px; kontras teks ≥ 4,5:1 |
| Tema | Otomatis → Terang → Gelap (tombol di bilah atas), disimpan di `localStorage` dengan `try/catch` |
| Komponen (`src/lib/components`) | `Button`, `Field`, `Segmented`, `Sheet`, `ProgressBar`, `Amount`, `Card`, `EmptyState`, `PageHeader`, `Icon` (SVG sebaris), `VirtualList`, `QuickAdd` |
| Navigasi | Tab bawah di ponsel; rel samping ≥ 720 px; tombol **+** global di semua layar |
| Layar | Rekening, Transaksi, Anggaran, Berulang, Laporan ditulis ulang; semua formulir berada di dalam lembar (bottom sheet) |

### Catat transaksi: dua ketukan

Definisi yang diukur: **ketukan 1** = tombol **+** (lembar terbuka, kolom jumlah sudah fokus, papan angka tampil), mengetik digit = masukan keyboard, **ketukan 2** = **Simpan**. Rekening dan kategori terisi dari pemakaian terakhir (`localStorage`), tanggal hari ini.

Diukur dengan ketukan `adb` sungguhan di emulator: `+` → `25000` → `Simpan` menyimpan pengeluaran `-25.000` kategori Makanan, rekening Dompet, lembar menutup, dan banner "Transaksi tersimpan." muncul. Mengganti kategori atau rekening menambah satu ketukan.

### Daftar tervirtualisasi

`VirtualList` merender hanya baris yang terlihat (tinggi baris tetap dalam `rem`, jadi mengikuti ukuran huruf pengguna). Pada 133 transaksi, DOM berisi **16 baris** sementara tinggi daftar 9.576 px; memuat halaman berikut dipicu mendekati ujung. Logika jendela (`visibleRange`, `nearEnd`) murni dan diuji; pengujian menemukan bug rentang terbalik saat daftar menyusut di bawah posisi gulir (diperbaiki dengan menjepit posisi gulir).

### Tata letak adaptif

Ponsel: tab bawah, satu kolom. ≥ 720 px: rel samping, lembar menjadi dialog tengah. ≥ 960 px: laporan dua kolom, lebar konten 62 rem. Diverifikasi lewat emulasi ukuran 1100×800.

### Ukuran bundel web (dilaporkan, dengan anggaran di CI)

| | Mentah | gzip |
|---|---|---|
| JS (21 berkas) | 138,5 KB | **55,9 KB** |
| CSS (11 berkas) | 18,5 KB | 6,3 KB |
| Total | 160,3 KB | 64,4 KB |

`scripts/bundle_size.py --max-js-gzip-kb=80` berjalan di CI (job `desktop`).

### Temuan selama verifikasi

| Temuan | Tindakan |
|---|---|
| Konten tergambar di bawah status bar dan bilah gestur (edge-to-edge) | `MainActivity` memasang listener inset native (bilah sistem, cutout, keyboard). Tidak bergantung pada dukungan CSS `env()` |
| Dengan keyboard terbuka, tombol **Simpan** berada di luar layar (viewport 530 px, tombol di 534–582 px) | Lembar punya footer yang disematkan di luar area gulir; semua formulir lembar memakainya |
| `backdrop-filter: blur()` pada bilah atas | Dibuang demi warna solid (mahal pada GPU rendah) |
| `adb input text` menghasilkan ANR berulang di emulator | Akar masalah: AVD dikonfigurasi `hw.gpu.enabled=no` (render perangkat lunak). Diubah ke `hw.gpu.mode=host`; ANR hilang. Jejak ANR sebelumnya menunjukkan thread utama menunggu `ThreadedRenderer.syncAndDrawFrame`, dan ANR di `system_server` sendiri. **Bukan loop di kode aplikasi**, tetapi penghapusan efek mahal tetap berguna untuk HP kelas bawah |
| Back menutup aplikasi saat lembar dibuka dengan klik JS | Artefak uji: entri riwayat tanpa gestur pengguna dilewati Chromium. Dengan ketukan nyata: Back pertama menyembunyikan keyboard (perilaku Android), Back kedua menutup lembar dan tetap di halaman |
| Pemformat kolom jumlah menelan digit | `.` diperlakukan sebagai pengelompokan dan `,` sebagai desimal; mata uang tanpa desimal mengabaikan semua pemisah. Diuji termasuk "umpan balik" keluarannya sendiri |

### Yang belum dilakukan / batasan

- Verifikasi hanya di emulator; belum di HP kelas bawah (Fase 8 untuk angka).
- Warna ikon status bar mengikuti tema sistem, bukan pilihan tema aplikasi.
- Skrip E2E sebelumnya (DOM lama) tidak lagi relevan; alur diverifikasi dengan ketukan nyata dan pemeriksaan DOM baru, belum ada suite E2E otomatis yang disimpan di repo.
- Tidak ada animasi transisi antar-layar; lembar hanya menaik halus (dihormati `prefers-reduced-motion`).
- Salin anggaran dari bulan lalu dan urutan manual rekening belum ada.

---

## 23. Fase 7, langkah A: kunci aplikasi (2026-10-14)

Urutan Fase 7 yang disepakati: **A** kunci aplikasi + `FLAG_SECURE`, **B** backup/restore terenkripsi (SAF), **C** temuan tertunda, lalu `/security-review`, `cargo audit`, `cargo deny check`. Bagian ini mencakup A dan langkah pra-Fase 7. Kompromi PIN/biometrik dijelaskan di [SECURITY.md](SECURITY.md).

### Pra-Fase 7

| Item | Hasil |
|---|---|
| Smoke test adb di repo | `scripts/android_smoke.mjs`: memasang APK debug, mengetuk **+**, mengetik 25000, mengetuk **Simpan**, memastikan baris tersimpan (`-25000`, pengeluaran, rekening, kategori), tidak ada ANR, `FLAG_SECURE` aktif. `--lock` dan `--biometric` menambah pemeriksaan kunci. Dijalankan di emulator: **43 pemeriksaan lulus** |
| Tes regresi | Dua bug Fase 6 sudah punya tes (`virtual.test.ts` "a list that shrank…", `money.test.ts` umpan balik `formatAmountInput`) |
| Ikon status bar mengikuti tema aplikasi | Plugin `mt-platform` (API Rust saja) + perintah aplikasi `system_bars_set`. Diverifikasi: sistem terang + tema gelap → `LIGHT_STATUS_BARS` hilang |
| Anggaran bundel JS | 120 → **80 KB gzip** (saat ini 61,7 KB) |
| Daftar temuan tertunda | Dengan tingkat, berkas, dan baris di §15 |

### Keputusan dan implementasi

| Keputusan | Pilihan |
|---|---|
| Hubungan PIN dengan kunci DB | **Gerbang UI, bukan kriptografis** (K2). Kunci DB tetap dibungkus Keystore, tanpa ikatan biometrik per-pemakaian |
| `setUnlockedDeviceRequired` | **Dipakai** (Android 9+). Aplikasi tak punya kerja latar, jadi tidak ada yang rusak. Bukan syarat autentikasi per-pemakaian |
| Penyimpanan verifier | `lock.json` di samping DB (DB tidak dibuka saat terkunci), ditulis atomik (tmp + fsync + rename), berisi PHC Argon2id (19 MiB, t=2, p=1), hitungan salah, biometrik, tenggang |
| Aturan PIN | 6–16 angka; ditolak bila semua sama atau berurutan (`mt-core::applock`) |
| Penundaan | 0 untuk 4 salah pertama; lalu 30 dtk, 1, 5, 15, 60 mnt. Hitungan **ditulis sebelum PIN diperiksa**; jeda memakai `Instant` (monotonik), mulai penuh lagi setelah proses dimulai ulang |
| Status saat start | Tanpa `lock.json`: DB dibuka seperti sebelumnya. Dengan `lock.json`: DB **tidak** dibuka. `lock.json` tak terbaca atau versi tak dikenal: **gagal tertutup** |
| Mengunci | Native: `WindowEvent::Suspended` (onPause) menutup DB (tenggang 0) atau mencatat waktu; `Resumed` memeriksa tenggang dengan dua jam dan mengunci bila jam dinding mundur. Dilakukan di Rust, bukan di WebView yang mungkin beku |
| Biometrik | `BiometricPrompt` AndroidX (`BIOMETRIC_STRONG`; sejak §30 dengan `CryptoObject` pada kunci biometrik tersendiri) di plugin sendiri, **bukan** `tauri-plugin-biometric`: hasilnya harus dari native langsung ke Rust, tidak dilewatkan lewat JS (R8). Mengaktifkan mensyaratkan satu sentuhan yang berhasil |
| `FLAG_SECURE` | Seluruh aplikasi (K6). Build debug dapat membukanya dengan `am start --ez mt_insecure true` agar emulator bisa difoto; build rilis mengabaikan ekstra itu |
| Perintah | `lock_status`, `lock_unlock_pin`, `lock_unlock_biometric`, `lock_setup`, `lock_change_pin`, `lock_remove`, `lock_biometric_set`, `lock_auto_set`, `lock_now`. Perintah data gagal dengan `locked` bila DB tertutup |
| UI | `LockScreen` (menggantikan seluruh shell, sehingga komponen dan datanya dibuang), `/settings`, tombol kunci + gerigi di bilah atas, penutup konten saat `visibilitychange` hidden, catch-up berulang dijalankan setelah tiap buka-kunci |
| Capabilities | Tetap `core:default` saja. Plugin `mt-platform` dan `mt-keystore` tanpa perintah JS |

### Temuan selama verifikasi

| Temuan | Tindakan |
|---|---|
| **Deadlock/ANR:** perintah Tauri sinkron berjalan di thread utama, lalu menunggu Kotlin yang memakai `runOnUiThread` (thread yang sama) | Semua perintah platform `async`; komentar di `platform.rs`; terverifikasi tidak ada ANR |
| **Celah penjaga CI:** `processUniversalReleaseMainManifest` tidak memuat manifes pustaka (izin `USE_BIOMETRIC` tidak muncul), jadi dependensi yang menambah `INTERNET` akan lolos | CI memakai `processUniversalReleaseManifest` (manifes gabungan penuh). `USE_BIOMETRIC` dan `USE_FINGERPRINT` masuk daftar putih dengan tes |
| Unlock lewat `invoke` mentah tidak mengubah layar | Hanya artefak uji (UI memperbarui status dari panggilannya sendiri); `visibilitychange` menyinkronkan |
| `uiautomator dump` kadang gagal saat layar beranimasi | Skrip mencoba ulang |
| Jendela utama memuat `KEEP_SCREEN_ON` pada build debug | Belum dilacak sumbernya (tidak ada di kode kita, wry, atau tauri); periksa pada build rilis di langkah C |

### Hasil uji

- Rust: 228 tes lulus (+ `mt-core::applock` 6, `applock` app 16). `clippy -D warnings`, `cargo fmt`, `cargo deny check`, `cargo audit` bersih.
- UI: 36 tes (+5 `lock.test.ts`). Penjaga offline: 15 tes.
- Emulator (`android_smoke.mjs --lock --biometric`): 43 pemeriksaan, termasuk PIN salah/benar dengan ketukan nyata, kunci saat ke latar belakang, penundaan setelah 5 salah dan bertahan setelah mulai ulang dingin, prompt biometrik otomatis, jari terdaftar membuka, Back membatalkan tanpa galat, jari tak terdaftar tidak membuka.
- Biometrik diuji dengan sidik jari emulator. Perilaku di perangkat nyata (terutama vendor dengan wajah Kelas 2) belum diverifikasi.

### Belum dilakukan

- Lupa PIN: tak ada pemulihan dalam aplikasi sampai langkah B. Pengaturan memperingatkan.
- M5, L1–L9 (langkah C). L7 sudah selesai sebelumnya.
- Tidak ada dukungan "layar kunci" sistem sebagai cadangan PIN (`DEVICE_CREDENTIAL`) — sengaja, agar tidak ada jalur kedua yang tak terhitung.

### Pemeriksaan sebelum langkah B (2026-10-14)

| Butir | Hasil |
|---|---|
| Penjaga izin pada artefak jadi | `check_offline.py apk <apk> [aapt2]` membaca `aapt2 dump permissions` dan `dump xmltree` dari **APK rilis jadi**, menolak `INTERNET`, `allowBackup`≠false, cleartext, debuggable, tanpa `dataExtractionRules`, dan pustaka native yang memuat `setWebContentsDebuggingEnabled`. CI membangun APK rilis dan menjalankannya; tes: APK debug gagal (INTERNET, cleartext, debuggable, devtools), APK rilis lolos |
| `forbid(unsafe_code)` | Masih berlaku di crate aplikasi dan seluruh workspace; tidak ada `unsafe` di kode kita. Titik sentuh Kotlin/JNI didaftar di SECURITY.md |
| `mt_insecure` pada rilis | `scripts/android_release_check.mjs`: APK rilis ditandatangani-uji dengan kunci debug, dijalankan dengan `--ez mt_insecure true`; `FLAG_SECURE` tetap aktif, tidak `KEEP_SCREEN_ON`, tidak debuggable |
| `KEEP_SCREEN_ON` | **Hanya build debug**; tidak ada pada jendela rilis. Syarat rilis kini dijaga skrip di atas |
| Soket DevTools pada rilis | Terlihat di emulator, tetapi image `userdebug` (`ro.debuggable=1`) yang membukanya; pustaka rilis tidak memuat panggilan pengaktifnya. Perlu dicek ulang di perangkat build `user` |
| Jeda PIN setelah reboot | Hitungan bertahan; jeda dimulai penuh lagi tiap proses baru (SECURITY.md) |
| Aturan async | Ditambahkan ke CLAUDE.md |
| Parameter Argon2id | Dicatat di SECURITY.md (19 MiB, t=2, p=1); pengukuran perangkat kelas bawah menunggu Fase 8 |

---

## 24. Fase 7, langkah B: backup dan restore terenkripsi (2026-10-15)

Rancangan keamanan ada di [SECURITY.md](SECURITY.md) (bagian "Backup terenkripsi"). Ringkas teknis:

| Lapisan | Isi |
|---|---|
| `mt-backup` (crate baru, tanpa I/O data) | Wadah terenkripsi: Argon2id + XChaCha20-Poly1305 STREAM-BE32, `BackupWriter`/`BackupReader` di atas `Write`/`Read`, memori konstan (potongan 64 KiB). Kebijakan frasa sandi |
| `mt-storage::backup` | `export_logical` (JSON per baris, satu snapshot baca, `ORDER BY` deterministik) dan `restore_logical` (basis data baru, skema milik cadangan lalu dimigrasikan, validasi lengkap). Tes penjaga: tabel baru wajib masuk `TABLE_ORDER` |
| `src-tauri::backup` | `write_backup`, `stage_restore`, `swap_in`/`finish_swap`/`undo_swap`, `recover_interrupted` (dipanggil sebelum membuka DB saat start) |
| `src-tauri::backup_cmd` | `backup_export`, `backup_restore_pick`, `backup_restore_prepare` (dapat diulang bila frasa sandi salah), `backup_restore_commit`, `backup_restore_cancel`. Sesi restore dihapus bersama berkasnya saat aplikasi terkunci |
| `mt-platform` | `saveFile` / `pickFile` (SAF, `ACTION_CREATE_DOCUMENT` / `ACTION_OPEN_DOCUMENT`); Kotlin hanya menyalin ciphertext antara URI dan cache privat |
| `applock` | `begin_external_ui` / `end_external_ui`: jendela sempit (maks. 60 detik, ditutup saat picker kembali, hanya di memori) agar picker tidak mengunci aplikasi |

### Hasil uji

| Uji | Hasil |
|---|---|
| `mt-backup` (14) | round-trip kosong/kecil/tepat satu potongan/banyak potongan; frasa sandi salah; membalik satu byte di 14 posisi; pemotongan di mana saja; pelepasan potongan terakhir pada batas; penyusunan ulang/duplikasi; data tambahan; versi tak dikenal; parameter rekayasa; kebijakan frasa sandi |
| `mt-storage::backup` (8) | restore identik (ekspor ulang menghasilkan **byte yang sama**); kategori bawaan terganti; basis data kosong; skema versi lama dimigrasikan; 11 jenis berkas rusak ditolak **tanpa meninggalkan berkas**; skema terlalu baru; tidak menimpa berkas yang ada |
| `src-tauri::backup` (11 + 1 besar) | ekspor→restore: jumlah dan saldo per rekening identik; tanpa plaintext; frasa sandi lemah ditolak sebelum menulis; klasifikasi kegagalan; wadah sah dengan isi sampah ditolak; swap, undo, kegagalan di tiap langkah menyisakan DB lama, pemulihan di dua titik crash |
| `applock` (+6) | picker tidak mengunci; dibatasi; menutup setelah picker kembali; jendela kedaluwarsa tidak melindungi; jam mundur; tidak bertahan setelah proses baru |
| UI | 4 tes `backup.test.ts` (aturan frasa sandi, nama berkas, pesan galat) |

### Uji emulator (`scripts/android_backup_e2e.mjs`, 29 pemeriksaan, picker sistem sungguhan)

Data diisi (2 rekening, 40 transaksi, 1 transfer) → PIN dipasang (kunci otomatis: segera) → frasa sandi pendek ditolak tanpa membuka picker → ekspor lewat picker "Simpan" **tanpa mengunci aplikasi** → berkas ditarik: ada magic `MTBK`, tanpa plaintext → membatalkan picker memberi `false` → uninstall + pasang ulang (kunci Keystore baru) → pilih berkas → frasa sandi salah dilaporkan dan tidak mengubah apa pun → pratinjau → konfirmasi → **rekening, saldo, 42 transaksi, dan kategori identik**; PIN tidak ikut → tambah satu transaksi lalu restore lagi: data diganti, identik dengan semula → proses dimulai ulang: data tetap ada.

`android_smoke.mjs --lock` setelah langkah B: 34 pemeriksaan lulus.

### Skala: 100 000 transaksi (rilis, macOS arm64, parameter produksi m = 64 MiB, t = 3)

| Langkah | Waktu |
|---|---|
| Impor dump teks ke DB baru (termasuk membangun indeks dan semua pemeriksaan) | 3,9 dtk |
| Ekspor + enkripsi | **0,75 dtk** |
| Ukuran berkas | **8,0 MB** |
| Dekripsi + restore + verifikasi | **1,9 dtk** |
| Puncak memori proses | **102 MB** RSS (footprint 92 MB), didominasi 64 MiB Argon2 + dump teks uji di memori; **tidak tumbuh dengan ukuran DB** |

Angka perangkat kelas bawah menunggu Fase 8.

### Temuan selama verifikasi

| Temuan | Tindakan |
|---|---|
| Kerusakan di tengah berkas awalnya dilaporkan sebagai "frasa sandi salah" bila seluruh isi muat satu potongan | Hanya potongan pertama yang membedakan; kerusakan sesudahnya dilaporkan sebagai **cadangan rusak**. Tes memakai banyak potongan |
| Hitungan baris setelah migrasi naik karena migrasi menyemai data baru | Jumlah baris dibandingkan **sebelum** migrasi naik; saldo sesudahnya |
| Jam dinding host melompat berjam-jam di antara beberapa pemanggilan (host/sandbox tertahan), tampak seperti `adb install` macet 15 menit | Bukan galat aplikasi; batas waktu skrip E2E dilonggarkan (240 dtk per picker) |
| Satu ANR `com.moneytracker.app` saat mengetik di smoke test, ketika load host 8–12 | Jejak: thread utama menunggu `ThreadedRenderer.syncAndDrawFrame` (render thread/GPU emulator), bukan kode aplikasi, sama dengan temuan Fase 6. Lulus pada ulangan dengan host lebih tenang. Perlu dipantau di perangkat nyata (Fase 8) |
| Emulator menjadi lamban setelah uji reboot (ANR Pixel Launcher/System UI) | Emulator dimulai ulang dingin; tidak terkait aplikasi |

---

## 25. Fase 7, langkah C: temuan tertunda (2026-10-16)

### Pra-C

| Butir | Hasil |
|---|---|
| Frasa sandi NFKC | `mt-backup` menormalkan frasa sandi dengan Unicode NFKC sebelum Argon2id. **Format versi 2** mencatatnya; versi 1 (tak pernah dirilis) ditolak. Tes: aksen tergabung vs terurai, huruf lebar penuh dan ligatur membuka cadangan yang sama; frasa berbeda tetap gagal. Kebijakan panjang dihitung pada bentuk ternormalisasi (UI memakai `normalize("NFKC")` yang sama) |
| Pesan galat potongan pertama | "wrong passphrase or **corrupted** backup" untuk potongan pertama; "the backup is corrupted" untuk kerusakan berikutnya (Rust dan UI), dengan tes |
| Berkas sementara setelah crash | `remove_leftovers` (cache: `export-*.mtbak`, `restore-input.mtbak`; data: `restore-staged.db*`) dipanggil saat start; tes memeriksa yang dihapus dan yang **tidak** boleh dihapus (`money.db`, `db.key`, `lock.json`) |
| StrictMode | `ThreadPolicy` (disk read/write, network, `penaltyLog`) pada build debug. Hasil di bawah |

#### Pelanggaran StrictMode (smoke `--lock` + sebagian E2E backup, emulator, 94 pelanggaran)

| Jumlah | Maks | Jenis | Sumber |
|---|---|---|---|
| 50 | **3 413 ms** | DiskRead | `System.loadLibrary` di `Rust.<clinit>`: memuat pustaka native **debug 160 MB** di thread utama saat `onCreate` (kode hasil generator Tauri). Pustaka rilis 10 MB; angka rilis perlu diukur di HP |
| 22 | 198 ms | DiskRead/Write | `app.tauri.PathPlugin.getCacheDir/getDataDir` (Tauri) |
| ~24 | 198 ms | DiskRead/Write | **Plugin kita**: `KeyVault.key/wrap/unwrap/generate` (operasi Keystore; plugin Tauri berjalan di **thread utama**), `PlatformPlugin.insideCache` (`canonicalFile`) |
| 0 | — | Network | — |

StrictMode hanya melihat I/O Java/Kotlin, **bukan** I/O native Rust (pembukaan DB, `lock.json`, Argon2 di `setup` berjalan di thread utama dan tidak terlihat). Tidak ada yang diubah (di luar ruang lingkup); dicatat untuk Fase 8: operasi Keystore dan `insideCache` dapat dipindah dari thread utama, dan waktu mulai dingin di perangkat nyata perlu diukur.

### Butir 1: M5 + L3, tidak ada yang dibuat diam-diam

**Aturan:** database dan kunci hanya dibuat pada *first run sebenarnya*: tidak ada `money.db` (berisi), tidak ada `db.key`, tidak ada `lock.json`. Selain itu, bila data hilang atau tidak terbaca, aplikasi tidak membuat apa pun dan menampilkan layar pemulihan.

Matriks keadaan (diuji: `classify` pada 8 kombinasi, dan pembukaan nyata dengan DB SQLCipher; setiap penolakan dibuktikan **tidak mengubah satu berkas pun**):

| `money.db` | `db.key` | `lock.json` | Hasil |
|---|---|---|---|
| tidak | tidak | tidak | **First run**: kunci dan DB dibuat |
| tidak | tidak | ada | Pemulihan `database_missing` |
| tidak | ada | tidak | Pemulihan `database_missing` (M5) |
| tidak | ada | ada | Pemulihan `database_missing` |
| ada | tidak | tidak | Pemulihan `key_missing` |
| ada | tidak | ada | Pemulihan `key_missing` |
| ada | ada | tidak | Siap |
| ada | ada | ada | Siap setelah buka kunci |

Aturan tambahan: `money.db` berukuran 0 dianggap hilang (SQLite akan membuatnya jadi DB kosong); hanya `money.db.old` (restore terputus) dianggap ada dan dipulihkan lebih dulu; satu berkas kosong saja tanpa kunci/lock tetap first run (tidak ada data yang hilang).

**Keystore (L3):** `unwrap` **tidak pernah membuat kunci**; hanya `wrap` boleh. Kotlin membedakan kehilangan permanen (alias hilang, `KeyPermanentlyInvalidatedException`, blob tidak lolos autentikasi, blob terlalu pendek) dari kegagalan sementara. Yang permanen memberi `key_unusable` (layar pemulihan); **kegagalan sementara hanya memberi "Coba lagi"**, tanpa tombol destruktif, tanpa membuat apa pun. DB yang dibuat aplikasi *lebih baru* tidak pernah ditawarkan untuk diganti.

| Kegagalan | Keadaan |
|---|---|
| Alias Keystore hilang / dibatalkan / blob rusak | `key_unusable` |
| Kunci membuka tetapi DB tidak (kunci salah, berkas rusak) | `database_unreadable` |
| Keystore sibuk, perangkat terkunci, I/O sementara | `retry` (tidak ada yang diubah) |
| Skema DB lebih baru dari aplikasi | `retry` (bukan pemulihan) |

**Jalan keluar, keduanya eksplisit:**
- *Pulihkan dari cadangan*: alur restore yang sama (pilih berkas, frasa sandi, pratinjau, konfirmasi), tetapi membangun kunci **baru**: kunci Keystore lama dihapus, kunci baru dibungkus, `db.key` ditulis dulu, berkas lama dibuang, DB disusun, lalu `lock.json` dihapus terakhir (PIN lama milik data lama; dipasang ulang). Urutan ini memastikan crash di titik mana pun tetap terbaca sebagai pemulihan, bukan first run.
- *Mulai dari awal*: butuh `confirm = true` yang menyatakan data lama akan hilang; tanpa itu perintah ditolak.

Layar pemulihan ditampilkan **sebelum** layar PIN (data yang hilang tidak dilindungi PIN; orang yang tahu frasa sandi cadangan sudah memiliki datanya).

**Uji emulator (`android_backup_e2e.mjs`, kini 61 pemeriksaan):** DB dihapus → `database_missing`, tidak ada DB yang dibuat, perintah data ditolak, "mulai baru" tanpa konfirmasi ditolak, restore lewat picker identik; DB 0 byte → `database_missing`; kunci diganti 60 byte acak → `key_unusable`, DB tidak tersentuh, restore identik; `db.key` dihapus → `key_missing`, tidak ada kunci dibuat, "mulai baru" butuh konfirmasi lalu menghasilkan DB kosong yang dapat dipakai; `lock.json` ada tanpa data → `database_missing` meski terkunci, restore mengganti data **dan** menghapus PIN lama.

### Butir 2: L1, `Debug` tidak mencetak jumlah

- `Money` punya `Debug` manual: `Money(<redacted> IDR)` (mata uang boleh, jumlah tidak).
- Makro `mt_core::redacted_debug!` menggantikan `derive(Debug)` pada **39 tipe** (ditambah `Money` yang manual) yang memegang jumlah, nama, catatan, atau hash PIN: draft dan rekaman domain, rekaman dan laporan penyimpanan, seluruh DTO dan input perintah, dan `Record` kunci aplikasi. Tipe galat tidak membawa nilai dan tetap derive.
- **Penjaga CI:** `scripts/check_debug_redaction.py` (8 tes) menggagalkan `derive(Debug)` baru pada tipe yang bidangnya bernama `amount*`, `*_minor`, `balance*`, `limit*`, `opening*`, `note`, `name`, `total*`, `pin_hash`, `Money`, dll. kecuali ditandai `// debug-safe: <alasan>`.
- Tes: `Money` di dalam `Vec`/`Option`/`Result` dan format `{:#?}`; semua tipe nilai inti; rekaman yang dibaca dari DB; semua input dan output perintah aplikasi.
- Batas: `DbKey = Zeroizing<[u8; 32]>` masih mencetak byte kunci bila diformat dengan `{:?}`; tidak ada kode yang melakukannya dan tipenya milik pustaka. Dicatat, bukan diubah.

### Butir 3: L2, eksponen mata uang

`Currency::new(kode, eksponen)` menolak eksponen yang salah untuk kode yang dikenal (`IDR` dengan 2 desimal akan membaca `1000` sebagai `10,00`: semua jumlah ×100). Kode yang tidak ada di tabel (mis. `BHD`) diterima dengan eksponen yang disebutkan (0–4). Tes: setiap mata uang tabel dengan setiap eksponen salah; IDR 0 / USD 2 / KWD 3 (3 desimal); `BHD` 3; parse dan cetak per eksponen (`1500000`, `12.34`, `1.234`, penolakan desimal berlebih); pembulatan rasio tetap sama.

### Butir 4: L5, FileProvider dihapus

Satu-satunya pemakai adalah kamera di pemilih berkas WebView hasil generator, yang tidak pernah dipanggil (tidak ada `<input type=file>`; backup memakai picker SAF lewat plugin). `file_paths.xml` membuka `external-path` dan `cache-path` seluruhnya. Provider dan berkas itu dihapus. `check_offline.py` (manifest dan APK jadi) kini gagal bila `FileProvider` muncul lagi, misalnya setelah `tauri android init` menimpa manifes.

### Butir 5: L8 dan L9

- **L8:** tidak ada lagi `allow(dead_code)` tingkat modul (dihapus di langkah A); yang tersisa per-item, berdasar alasan.
- **L9:** migrasi membaca `user_version` **di dalam** `BEGIN IMMEDIATE`. Tes: empat koneksi membuka DB versi 1 bersamaan, 12 putaran, setiap migrasi berjalan **sekali** (12 kategori bawaan, tidak ganda); langkah migrasi yang gagal di-rollback tanpa menaikkan versi dan tanpa transaksi menggantung.

---

## 26. Pengerasan pemulihan: temuan F1–F4 dan F13 (2026-10-16)

Dari `/security-review` dan tinjauan adversarial manual. Tiga aturan baru ada di CLAUDE.md: *galat yang tidak dikenal dianggap sementara*, *hancurkan paling akhir*, *tindakan destruktif butuh PIN (bila `lock.json` valid) dan konfirmasi ketik*. Satu commit per temuan, gerbang validasi di tiap commit.

| Temuan | Perbaikan | Bukti |
|---|---|---|
| **F1** `UnrecoverableKeyException` dianggap permanen | `KeyFailures.kt`: hanya **alias hilang, kunci tidak valid permanen (`KeyPermanentlyInvalidatedException`), blob terlalu pendek, atau tag GCM gagal dua kali** yang `key_gone`. `UnrecoverableKeyException`, `UserNotAuthenticatedException`, `KeyStoreException`, `ProviderException`, `IllegalBlockSizeException`, dan semua yang tidak dikenal adalah sementara. `unwrap` mencoba ulang sekali bila tag gagal. Di Rust hanya kode `key_gone` yang menjadi `KeyGone` | 10 tes JVM Kotlin (satu per jenis galat, dijalankan CI) + 2 tes Rust |
| **F4** `Path::exists()` menelan galat | `fs::metadata`; hanya `NotFound` berarti tidak ada. Galat lain (izin, bukan direktori, I/O) = `Unavailable`, yaitu "Coba lagi": tidak ada layar pemulihan, tidak ada first run, tidak ada berkas berubah | tes bukan-direktori dan izin ditolak |
| **F2** Pemulihan destruktif tanpa PIN | Dengan `lock.json` valid dan terbaca: **PIN lebih dulu** (`recovery_authenticate`, memakai batas percobaan yang sama, berlaku 5 menit di memori) sebelum "restore menimpa" atau "mulai dari awal" ditawarkan; **kata `HAPUS` diketik** (dicek backend). **Satu pemeriksaan ulang** (`vault::recheck`) tepat sebelum menghancurkan apa pun: bila data ternyata terbuka, atau keadaannya tak bisa dipastikan, perintah dibatalkan tanpa perubahan. `lock.json` rusak tidak dapat diperiksa dan tidak memblokir | tes kewenangan, kedaluwarsa, PIN salah dihitung, recheck untuk tiap keadaan, kata konfirmasi (Rust + UI) |
| **F3** Kunci Keystore dihapus lebih dulu | Restore-menimpa dan "mulai dari awal" kini **satu jalur** (`commit_recovery`): (1) DB staged sudah ada dan terverifikasi (atau DB kosong baru); (2) kunci DB dibungkus lalu **dibuka lagi** untuk membuktikannya; (3) `money.db` dan `db.key` ditukar **bersama** (lama disimpan sebagai `*.old`, kunci baru lewat `db.key.new`), tiap kegagalan mengembalikan semuanya; (4) berkas baru dibuka dan dibandingkan dengan isi staged, **baru sesudah itu** berkas lama dihapus dan `lock.json` dihapus terakhir. Pengecualian untuk `key_unusable` **dihapus** (tindak lanjut, §27): alias Keystore bernomor versi, jadi kunci baru selalu dibungkus dengan alias **baru** dan alias lama tidak disentuh sampai data baru terverifikasi | injeksi kegagalan di setiap dari 5 langkah (data lama utuh dan terbaca); crash sebelum tiap langkah dipulihkan menjadi lama **atau** baru, tidak pernah campuran; undo setelah verifikasi gagal, termasuk menghapus berkas baru bila tak ada yang lama |
| **F13** `.old` dipulihkan karena galat sementara | `recover_interrupted` memakai probe tiga nilai. **Pulihkan yang lama hanya pada jawaban pasti**: `db.key.new` masih ada, DB aktif hilang, atau probe `Corrupt` (bukan database, blob kunci tak terpakai permanen). Probe `Opens` menghapus yang lama (sisa `.old` tak pernah menghidupkan data basi). **Probe `Unknown`** (Keystore sibuk, galat I/O, berkas terkunci, galat pemeriksaan) **tidak memindahkan atau menghapus apa pun** dan pembukaan melapor "coba lagi". Pasangan dinilai sebelum berkas diklasifikasi | tes untuk tiap hasil, `.old` basi + galat sementara, dan pembukaan yang tak diputuskan tidak mengubah satu berkas pun |

### Matriks keadaan setelah perubahan

Berkas yang diperiksa: `money.db` (tidak kosong, atau `money.db.old` ada), `db.key`, `lock.json`. Pertukaran yang terputus dinilai **lebih dulu** (marker `money.db.old`, `db.key.old`, `db.key.new`).

| `money.db` | `db.key` | `lock.json` | Keadaan | Boleh membuat sesuatu? | Tindakan destruktif |
|---|---|---|---|---|---|
| tidak | tidak | tidak | First run | **ya** (satu-satunya) | tidak ada |
| tidak | tidak | ada | `database_missing` | tidak | PIN + (restore atau `HAPUS`) |
| tidak | ada | tidak | `database_missing` | tidak | restore atau `HAPUS` |
| tidak | ada | ada | `database_missing` | tidak | PIN + (restore atau `HAPUS`) |
| ada | tidak | tidak | `key_missing` | tidak | restore atau `HAPUS` |
| ada | tidak | ada | `key_missing` | tidak | PIN + (restore atau `HAPUS`) |
| ada | ada | tidak | Siap | tidak | tidak ada |
| ada | ada | ada | Siap setelah buka kunci | tidak | tidak ada |
| *berkas tak bisa diperiksa (galat selain NotFound)* | | | **`retry`** | tidak | **tidak ada** |
| *Keystore sibuk / galat sementara / tak dikenal* | | | **`retry`** | tidak | **tidak ada** |
| *kunci tak terpakai permanen* | | | `key_unusable` | tidak | PIN (bila lock) + restore atau `HAPUS` (reset kunci Keystore hanya di sini) |
| *kunci membuka, DB tidak (bukan database)* | | | `database_unreadable` | tidak | PIN (bila lock) + restore atau `HAPUS` |
| *pertukaran terputus, probe tak bisa memutuskan* | | | `retry` | tidak | **tidak ada** (kedua salinan utuh) |

Sebelum tindakan destruktif mana pun, keadaan diperiksa sekali lagi. Hasil `Readable` atau `Transient` membatalkan tanpa perubahan; hanya `StillUnreadable` (termasuk "tidak ada apa-apa lagi") yang dilanjutkan.

**Batas yang diketahui:** `UnrecoverableKeyException` yang sebenarnya permanen (entri Keystore rusak) kini selalu tampil sebagai "Coba lagi" dan pengguna harus menghapus data aplikasi lewat pengaturan Android. Itu pilihan sadar: lebih baik buntu daripada menawarkan "mulai dari awal" karena galat yang mungkin sementara.

---

## 27. Tindak lanjut kelompok 1 (2026-10-16)

### Alias Keystore bernomor versi: "hancurkan paling akhir" tanpa pengecualian

Sebelumnya `key_unusable` me-reset satu-satunya kunci lebih dulu. Kini alias bernama `money_tracker_db_key_wrap_v<N>` dan blob kunci terbungkus diawali satu byte yang menyebut alias pembungkusnya (`version(1) || iv(12) || ciphertext+tag`).

| Hal | Aturan |
|---|---|
| Kunci pertama | first run membuat `v1` |
| Mengganti kunci | `next_alias_version` = di atas semua alias yang ada (tidak pernah dipakai ulang); `wrap_new_alias` membuat alias **baru** dan membungkus dengannya; alias lama tidak disentuh |
| `unwrap` | memakai alias yang disebut blob; tidak pernah membuat kunci |
| Menghapus alias lama | hanya **setelah** data baru terverifikasi (`prune_aliases_below`); kegagalan di sini meninggalkan alias basi yang tidak berbahaya |
| Pembersihan | setelah pembukaan yang berhasil dan pasangan berkas tenang, alias yang lebih lama dari alias `db.key` dihapus; alias yang lebih baru (penggantian belum selesai) tidak disentuh |
| `reset` | dihapus dari plugin |

`vault::replace_data` (diuji dengan Keystore palsu): buat alias baru → bungkus → buka lagi dan bandingkan → tukar berkas bersama → verifikasi → hapus berkas dan alias lama. **Injeksi kegagalan di tiap langkah** (buat alias, round-trip, lima langkah pertukaran, verifikasi): alias lama tetap ada, `db.key` dan `money.db` tidak berubah, data lama masih terbaca, alias baru dihapus lagi, tidak ada marker tersisa, dan DB staged selamat (kini dikembalikan ke tempatnya saat undo, bukan dihapus). Juga: kegagalan menghapus alias lama setelah sukses tidak kehilangan data dan dibersihkan di start berikutnya; start tidak pernah menghapus alias yang masih dibutuhkan; penggantian tanpa pasangan lama juga bersih.

Tes Kotlin JVM (18 total): penamaan alias, alias asing diabaikan, versi tidak dipakai ulang dan tidak berputar, hanya versi lebih lama yang basi, blob cacat ditolak.

### Panduan saat "Coba lagi" terus gagal

Setelah tiga kegagalan beruntun layar menampilkan penjelasan, tanpa tindakan destruktif: data tidak dapat dibuka sementara ini, cadangan adalah satu-satunya jalan pemulihan, dan langkahnya (mulai ulang ponsel; hapus data aplikasi lewat pengaturan Android; lalu Pulihkan dari cadangan; tanpa cadangan data tidak dapat dipulihkan).

### Dicatat untuk kelompok 3

Masa berlaku konfirmasi PIN pemulihan (5 menit) terlalu lama untuk tindakan yang menghapus data. Disamakan dengan mekanisme PIN ulang F11 (sekitar 60 detik).

---

## 28. Kelompok 2 (2026-10-17)

| Temuan | Perbaikan | Bukti |
|---|---|---|
| **F9** Kebijakan frasa sandi diakali NFKC | Panjang minimum dan cek pengulangan diterapkan pada teks **asli dan** bentuk NFKC; yang lebih ketat menang (Rust dan cermin UI). NFKC bisa memperbesar (U+FDFA menjadi 18 karakter, ligatur ffi menjadi 3, U+337F menjadi 4) maupun memperkecil teks. *Koreksi:* U+FDFD **tidak** mengembang (tetap 1 karakter); karakter yang mengembang menjadi 18 adalah U+FDFA | tes U+FDFA, U+FDFD, ffi ×4, U+337F ×3 dan ×12, A lebar penuh, batas maksimum setelah ekspansi |
| **F20** `DbKey` mencetak byte kunci | Newtype dengan `Debug` tersamar; penjaga CI diperluas (kata `key/pin/hash/blob/salt/token/secret/passphrase/password/nonce`, tipe `DbKey/Zeroizing`, `cfg_attr(derive(Debug))`, `impl Debug` manual yang tak menyamarkan) | tes `DbKey` di `Option/Vec/Result/{:#?}`; 5 tes baru penjaga |
| **F15** `securityLevel()` bisa membuat kunci | Memakai kunci yang ada (`keyOrGone`), tidak pernah membuat | dikerjakan bersama alias bernomor versi |
| **F19** Zeroization | `argon2` dengan fitur `zeroize`; `chacha20poly1305` memang sudah menghapus kunci dan state saat di-drop; PIN dan frasa sandi diterima perintah langsung sebagai `Zeroizing<String>` (fitur `serde` pada `zeroize`) | tes deserialisasi |
| **F7** URI SAF tanpa pemeriksaan skema | `SafUri.isAcceptable`: hanya `content://` milik aplikasi lain; `file://`, skema lain, authority kosong, dan provider milik sendiri (termasuk dengan user-info/port) ditolak, di kedua callback | 6 tes JVM Kotlin (dijalankan CI) |


---

## 29. Kelompok 3, bagian A: PIN ulang dan jendela picker (2026-10-17)

| Temuan | Perbaikan | Bukti |
|---|---|---|
| **F11** Tindakan sensitif tanpa PIN ulang | `lock_step_up(pin)` memverifikasi lewat `LockService::verify_pin` (penghitung tersimpan dan jeda yang sama dengan membuka kunci) lalu mencatat `(Instant, lock_generation)` di `AppState`. Berlaku `STEP_UP_WINDOW` = 60 detik dan gugur bila kunci menyala (`generation` naik) atau pemulihan selesai. Dijaga `lock::require_step_up`: `backup_export`, `backup_restore_commit` (sebelum `staged.take()`, dan sekali lagi di jalur pemulihan; sukses mengosongkan step-up), `lock_biometric_set` (dua arah), `lock_auto_set` **hanya bila tenggang bertambah**, dan `recovery_start_fresh`. `recovery_authenticate` dan jendela 5 menit dihapus; layar pemulihan memakai `lock_step_up` | `lock::tests`: ditolak tanpa step-up, diterima dengan step-up, gugur setelah 60 dtk dan tetap gugur, PIN salah tidak memberi apa pun dan menambah penghitung bersama (termasuk jeda: PIN benar ditolak saat jeda), salah di layar kunci dan di step-up dijumlahkan, tidak selamat dari kunci yang menyala, tanpa kunci valid tidak diminta, tabel "melemahkan"; tes penjaga memeriksa di sumber bahwa tiap perintah terjaga memanggil `require_step_up` sebelum pekerjaan lain; skrip perangkat memeriksa perintah nyata |
| **F21** Jendela picker 120 detik | `EXTERNAL_UI_MAX` = 60 detik; `on_resumed` menghabiskan jendela begitu aplikasi kembali dari picker yang kita luncurkan (tanpa menunggu perintahnya melapor); pembatalan menghapusnya | tes batas 59/61 detik, jendela habis saat kembali, picker dibatalkan |

UI: `guarded(aksi)` mencoba aksi; hanya bila backend menjawab `pin_required` lembar PIN (`StepUpSheet`) muncul, lalu aksi diulang sekali. Menutup lembar menggagalkan aksi dengan `cancelled`. Yang memutuskan tetap backend.


## 30. Kelompok 3, bagian B: kunci biometrik tersendiri (F8, 2026-10-17)

| Aspek | Keputusan |
|---|---|
| Kunci | AES-256-GCM, alias `money_tracker_biometric_unlock_v1` (bukan `money_tracker_db_key_wrap_v<N>`), `setUserAuthenticationRequired(true)`, per-pemakaian (`setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG)` di API 30+, `validityDuration=-1` di bawahnya), `setInvalidatedByBiometricEnrollment(true)` |
| Pendaftaran (`biometricEnroll`) | hapus kunci biometrik lama, buat yang baru, `Cipher` mode ENCRYPT, `BiometricPrompt` dengan `CryptoObject`; setelah sentuhan: token acak 32 byte disegel (`iv‖ct+tag`). Native mengembalikan token dan blob (hex) ke Rust. Rust menyimpan blob dan SHA-256 token di `lock.json` (`bio_blob`, `bio_check`, field baru dengan `serde(default)`); token dibuang (`Zeroizing`) |
| Buka-kunci (`biometricUnlock`) | `Cipher` DECRYPT dengan IV dari blob, `CryptoObject`, setelah biometrik sungguhan `doFinal` melepas token, Rust membandingkan hash dengan `subtle` |
| Pembatalan kunci | `KeyPermanentlyInvalidatedException` saat `cipher.init`, alias hilang, atau blob tidak lagi terautentikasi -> `invalidated` -> `LockService::disable_biometric` (blob dan hash dihapus, PIN tetap), kunci biometrik dihapus (hanya alias itu), UI diberi `biometric_invalidated`. Tidak ada jalur ke `StorageProblem`, `key_gone`, pemulihan, `db.key`, atau `money.db` |
| Sinyal lain | sementara (`Failed`): biometrik tetap aktif, PIN tersedia |
| Saklar tanpa bahan | `biometric: true` tanpa `bio_blob`/`bio_check` dibaca sebagai mati |
| Mematikan, `lock_remove` | biometrik dimatikan di `lock.json` dahulu, baru kunci dihapus (best effort) |
| Dependensi | `sha2` 0.10 dan `subtle` 2 disebut langsung di `src-tauri` (RustCrypto, MIT/Apache-2.0 dan BSD-3, sudah ada di pohon lewat tauri/argon2) |
| Tes | Rust: token terdaftar membuka, token lain mematikan, `Invalidated` mematikan biometrik dan **tidak mengubah byte `money.db`/`db.key`**, `problem()` tetap `None`, PIN tetap bekerja, bisa diaktifkan lagi, batal/gagal tidak mengubah apa pun; Kotlin JVM: klasifikasi kunci biometrik (tiga sinyal permanen, sisanya sementara), alias biometrik bukan alias DB, tidak ada "GONE" dalam kosakatanya, blob dan hex; di `mt-keystore` alias biometrik tidak dikenali sebagai alias DB (tidak dihitung, ditahan, atau dipangkas) |

## 31. Kelompok 3, bagian C, banner backup, vektor passphrase (2026-10-17)

| Temuan | Perbaikan | Bukti |
|---|---|---|
| **Item 1** Pemangkasan alias harus aman | `prune_stale_aliases` hanya berjalan bila `db.key` terbaca, byte pertamanya nama alias, blob benar-benar membuka (alias yang disebut terbukti ada) dan tidak ada penanda tukar (`*.old`, `db.key.new`). Byte rusak seperti 200 tidak lagi berarti "semua alias lebih tua" | tes byte 0, kosong, berkas hilang, alias tidak ada, badan blob rusak, penanda tukar |
| **F10** `Suspended` hilang saat unlock | `on_suspended` mencatat kepergian walau terkunci; `mark_unlocked` mengunci lagi seketika bila aplikasi pergi saat unlock dan tenggangnya nol, selain itu waktu pergi diukur dari kepergian yang sebenarnya; `open_and_unlock` menutup DB dan menjawab `locked` | 5 tes `applock` (tanpa tenggang, tenggang di dalam dan di luar, tidak ada yang pergi, sudah kembali sebelum unlock selesai, jendela picker) |
| **F12** `with_db` hanya memeriksa koneksi | `with_db` gagal dengan `locked` selama flag kunci menyala, dicek sebelum dan sekali lagi di bawah kunci DB, sehingga tidak ada perintah yang lolos di antara flag menyala dan DB ditutup | 2 tes `state` (koneksi hidup tetapi flag menyala: closure tidak jalan) |
| **F14** Balapan commit/initialize/start_fresh | `AppState::storage_mutex` (`tauri::async_runtime::Mutex`) diambil di awal perintah **async** `recovery_retry`, `recovery_start_fresh`, `backup_restore_commit`, dan kedua perintah unlock, sebelum pekerjaan ke `spawn_blocking`. Tidak pernah diambil oleh hook siklus hidup atau helper sinkron di thread utama (thread utama harus bebas melayani panggilan Kotlin yang ditunggu pekerjaan yang diblokir; pelajaran ANR); fungsi yang dilindungi tidak mengambilnya sendiri; prasyarat diperiksa ulang setelah mutex diambil | 24 operasi bersamaan tidak pernah tumpang tindih dan tidak buntu dengan thread utama tiruan yang melayani permintaan "Kotlin" (diuji mutasi: tanpa kunci pernyataan tumpang tindih gagal); tes sumber memastikan kelima perintah mengambil mutex lebih dulu dan `lib.rs`/`applock.rs` tidak pernah |

### Banner backup

`backup_meta.json` di samping DB menyimpan dua cap waktu: backup terakhir yang disimpan pengguna dan kapan aplikasi pertama melihat (agar "belum pernah" punya umur). Tidak ada jaringan dan tidak ada data keuangan. `backup_status` (hanya saat terbuka). Banner di atas tiap layar setelah 7 hari (atau 7 hari dipakai tanpa backup) dan satu baris di kartu cadangan. Cap di masa depan dihitung "hari ini".

### Vektor passphrase bersama

`testdata/passphrase_vectors.json` (ditulis tangan, bukan dihasilkan oleh salah satu implementasi) dibaca oleh tes Rust `mt-backup` (otoritas) dan tes UI. Tes UI mengungkap satu ketidakcocokan presedensi (Rust: "terlalu pendek" menang bila salah satu bentuk terlalu pendek, selain itu bentuk yang diketik); UI kini meniru Rust persis lewat `passphraseVerdict`. Satu vektor yang saya tulis salah (ekspektasi tangan) ikut ketahuan dan dikoreksi sesuai perilaku Rust.

### Bukti perangkat (emulator, Keystore asli)

| Skrip | Hasil |
|---|---|
| `android_backup_e2e.mjs` setelah bagian A | 93 pemeriksaan lulus (PIN ulang, lapse 60 dtk, restore dengan kunci) |
| `android_smoke.mjs --lock --biometric` | 46 pemeriksaan (alur CryptoObject) |
| `android_smoke.mjs --only-invalidate` | 13 pemeriksaan: sidik jari baru mematikan biometrik dengan penjelasan, penyimpanan tetap `ok`, tidak ada layar pemulihan, PIN membuka, data utuh, bisa diaktifkan lagi |
| `android_backup_e2e.mjs` setelah bagian C | 93 pemeriksaan lulus |

Catatan: dua percobaan awal E2E gagal karena aplikasi Files di emulator macet (ANR) dan menahan picker lebih dari 60 detik, sehingga kunci menyala; itu perilaku F21 yang benar. Skrip kini membuka Files sekali dan menunggu idle sebelum picker (jendela 60 detik tidak diubah). Banner dan vektor belum dijalankan di perangkat; keduanya diuji unit.

## 32. Kelompok 4: pagar CI (2026-10-17)

| Butir | Perbaikan | Bukti |
|---|---|---|
| Daftar izin capability | Dari daftar-larang awalan (`http:`, `shell:`...) ke **daftar-izin** (`core:default` saja, jendela `main` saja). Dibaca dari `.json`, `.json5` (komentar dan koma akhir saja), `.toml`, dan objek inline di `tauri.conf.json`; berkas yang tidak dapat diurai atau jenis yang tidak dikenal **gagal** (tidak bisa diverifikasi). Scope (`allow`/`deny`), `remote`/`urls`, jendela wildcard, dan flag `dangerousDisableAssetCspModification` / `dangerousRemoteDomainIpcAccess` ditolak | tes: plugin yang tak terpikirkan (`fs:`, `dialog:`, `my-plugin:`) ditolak, tiga format, inline, scope, jendela, flag |
| CSP direktif pertama | Browser menaati direktif **pertama** bila diulang; penjaga lama menyimpan yang terakhir sehingga `script-src https://evil; script-src 'self'` lolos. Kini direktif berulang ditolak, nama direktif dan kata kunci tidak peka huruf besar, dan bentuk peta (`{direktif: sumber}`) dinilai sama | tes khusus kasus itu |
| Pin SHA | Semua `uses:` di `ci.yml` dipin ke SHA 40 heksa (komentar tag di belakangnya); `dtolnay/rust-toolchain` dengan `toolchain: 1.98.1` eksplisit (versi yang dipakai gerbang lokal); `cargo-deny@0.20.2` dan `cargo-audit@0.22.2` dipin. `scripts/check_workflows.py` menolak tag/cabang/SHA pendek, image Docker tanpa digest, `pull_request_target`/`workflow_run`, workflow tanpa `permissions:` di atas, dan `curl \| sh` | 15 tes; dijalankan di CI |
| Daftar pengecualian terpusat (temuan 4) | `scripts/guard_exceptions.json` (tiap entri wajib beralasan) menggantikan komentar `// debug-safe:` dan `offline-check: allow`; komentar inline kini **ditolak dan dilaporkan**; entri yang tidak cocok apa pun gagal (daftar tidak menumpuk entri mati). Tiga pengecualian nyata tersisa (`LockError`, `BackupFailure`, `DirState`); lima komentar lain ternyata tidak diperlukan dan dihapus | `test_guard_common.py` dan tes di kedua penjaga |
| CODEOWNERS | `.github/CODEOWNERS` mencakup workflow, `scripts/`, daftar pengecualian, `deny.toml`, capability, `tauri.conf.json`, manifest. Penjaga memastikan berkasnya ada dan mencakup jalur itu. **Pemilik masih placeholder** dan harus diganti dengan nama pengguna GitHub sebelum push; aktifkan juga "Require review from Code Owners" di proteksi cabang `main` |  |

Catatan jujur: SHA di atas diambil dari `git ls-remote` ke repositori masing-masing hari ini (tag rilis terbaru per versi mayor); saya tidak dapat memverifikasi isi tiap rilis, jadi pembaruan pin sebaiknya lewat Dependabot dengan peninjauan. CI GitHub belum pernah berjalan, jadi sintaks workflow dan nama input (`toolchain`, `tool`) belum teruji di runner.

## 33. Fase 8: performa, uji kegagalan, dan APK rilis (2026-10-17)

### F16 dan F17: putus daya saat penukaran data

| Temuan | Perbaikan | Bukti |
|---|---|---|
| **F17** `money.db`, `-wal`, `-shm` dipindah dengan rename terpisah; bila terputus di antaranya, undo/pemulihan menghapus berkas di nama live, termasuk log WAL lama yang berisi commit yang belum di-checkpoint (**jendela kehilangan data nyata**, ditemukan oleh tes baru, bukan teori) | Pindah menjauh: pendamping dahulu, berkas utama terakhir (kembali: utama dahulu), jadi satu berkas utama yang sudah tiba berarti seluruh set tiba; gerakan yang terputus dikenali (pendamping lama di samping tanpa berkas utama lama) dan dibatalkan **tanpa menghapus apa pun** di nama live; `finish_swap` menghapus berkas utama terakhir; penanda mencakup seluruh set | tes menghentikan "proses" sebelum **setiap** rename (anggaran rename per thread) pada penukaran dengan dan tanpa kunci, undo, rollback setelah rename gagal, dan memotong `finish_swap` setelah tiap penghapusan, selalu dengan `-wal` panas berisi satu rekening yang tidak ada di tempat lain. Uji mutasi: dengan urutan lama, empat tes gagal |
| **F16** rename tidak menyinkronkan direktori | modul `durable`: `write_atomic` (tmp + fsync + rename + fsync direktori) untuk `db.key`, `lock.json`, `backup_meta.json`; `sync_dir` setelah kelompok rename penukaran, undo, finish | tes `write_atomic` (tanpa sisa tmp, tmp basi ditimpa). **Batas:** fsync direktori tidak dapat diuji dengan benar-benar mencabut daya; bukti hanya kode dan urutan |

### Rencana kueri dan pengukuran (100 000 transaksi, build rilis, host Apple silicon, SQLCipher dalam memori)

`crates/storage/src/perf.rs` memeriksa `EXPLAIN QUERY PLAN` pada SQL yang benar-benar dijalankan (`page_query` dan konstanta laporan dipakai bersama) di setiap `cargo test`; pengukuran waktu ada di tes `ignored` (`cargo test -p mt-storage --release -- --ignored --nocapture perf_100k`).

| Kueri | Sebelum | Sesudah | Catatan |
|---|---|---|---|
| Daftar halaman 50, terbaru | 0,07 ms | sama | `idx_tx_date`, tanpa sort |
| Halaman berikutnya (kursor) | 2,7 ms | **0,06 ms** | kondisi OR diganti nilai baris `(occurred_on, id) < (?, ?)` |
| Satu rekening / satu bulan / hanya pengeluaran | 0,05–0,07 ms | sama | indeks, tanpa sort |
| Satu kategori (dengan anak) | 12 ms | sama | dua pencarian indeks digabung lalu disortir (diterima) |
| Pencarian teks `LIKE '%x%'` | 21 ms | sama | pemindaian (diterima, tidak ada indeks yang membantu) |
| Daftar rekening (saldo per rekening) | 33,5 ms | **3,0 ms** | migrasi 0003: indeks penutup `(account_id, amount_minor)` |
| Kekayaan bersih | 33 ms | ~3 ms (sama dengan daftar rekening, tidak diukur terpisah) | memakai saldo yang sama |
| Laporan pengeluaran, sebulan / setahun | 1,1 / 13 ms | sama | `idx_tx_date` |
| Tren 12 bulan | 26 ms | sama | `idx_tx_date` |

**Saldo tersimpan: tidak dibutuhkan.** Dengan indeks penutup, saldo pada 100 ribu baris 3 ms di host; pengguna nyata punya jauh lebih sedikit baris. Saldo tidak dapat melenceng karena tidak disimpan. Biayanya: satu indeks tambahan (tulis sedikit lebih berat, sekitar 2 MB pada 100 ribu baris). Pada perangkat kelas bawah angkanya perlu dikalikan sekian kali (belum diukur).

### Argon2id dan waktu mulai

| Ukuran | Hasil | Catatan |
|---|---|---|
| Satu pemeriksaan PIN (19 MiB, t=2, p=1) | **17 ms** per percobaan (host, rilis) | `perf_pin_hash_with_the_production_cost` |
| Ekspor + enkripsi 100 ribu baris (64 MiB, t=3) | 206 ms; berkas 8,0 MB | |
| Dekripsi + restore + verifikasi 100 ribu baris | 1,7 s | |
| Mulai dingin, APK rilis, emulator arm64 Android 15 (`am start -W`, 5 kali) | 2,1 / 2,6 / 3,0 / 3,2 / 3,7 s | emulator di bawah beban host; sebagian besar inisialisasi WebView |

**Belum terbukti pada perangkat kelas bawah.** Argon2 terikat memori dan CPU; waktu di ponsel murah dapat 5-20 kali angka host. Ukur di perangkat nyata: waktu buka kunci (harus tetap di bawah sekitar 1 detik), ekspor (KDF 64 MiB), dan mulai dingin. Perintah mulai dingin: `adb shell am force-stop com.moneytracker.app; adb shell am start -W -n com.moneytracker.app/.MainActivity`.

### APK rilis untuk HP nyata

* `pnpm tauri android build --apk --target aarch64` menghasilkan APK **tanpa tanda tangan**, **12,3 MB** (arm64; ponsel 32-bit tidak didukung). Penjaga APK (`check_offline.py apk`) lulus: tanpa INTERNET, `allowBackup=false`, tidak dapat di-debug, tanpa WebView debugging. `android_release_check.mjs` lulus di emulator: `FLAG_SECURE` tetap aktif walau `mt_insecure` diberikan.
* Salinan **bertanda tangan kunci debug** untuk uji coba di ponsel Anda: `target/release-apk/money-tracker-0.1.0-arm64-TEST-SIGNED.apk` (diabaikan git). Pasang: `adb install -r <berkas>` atau pindahkan ke ponsel dan izinkan "pasang dari sumber ini". **Bukan untuk distribusi.** Kunci debug dikenal semua orang.
* Menggantinya nanti dengan APK bertanda tangan kunci rilis sendiri memaksa uninstall (tanda tangan berbeda), yang menghapus data: **buat backup terenkripsi dulu** dan pulihkan setelahnya. Kunci rilis (keystore pribadi) adalah keputusan dan rahasia Anda; belum dibuat.

### Property test `Money` (proptest, disetujui pemilik)

`crates/core/tests/money_properties.rs`, 13 properti x 512 kasus, dengan nilai ekstrem (+-(2^53-1), 0) diberi bobot lebih: `from_minor` menerima tepat rentang simetris; `a + b - b == a` dan komutatif (hanya `Overflow` yang boleh menolak); `a - a == 0`, negasi involusi; mata uang berbeda tidak pernah bercampur; **`allocate` tidak pernah kehilangan atau menambah satu satuan minor**, bagian bertanda sama dengan keseluruhan, rasio nol mendapat nol, tiap bagian dalam satu satuan dari proporsi eksak, dan cermin untuk nilai negatif; `checked_mul_ratio` membulatkan ke terdekat dengan seri menjauhi nol (dibandingkan dengan aritmetika i128 independen); seri dibangun khusus karena acak hampir tak pernah jatuh tepat di tengah; pembagian nol dan `checked_mul` tidak pernah panik; `parse(format(x)) == x` untuk enam mata uang di seluruh rentang; `parse_decimal` pada teks acak (termasuk tumpukan digit raksasa) tidak pernah panik dan yang berhasil di-parse kanonik.

Uji mutasi: menghilangkan satu pembagian sisa di `allocate` ditangkap `allocate_never_loses_or_invents_a_minor_unit`; mengganti `>=` menjadi `>` pada aturan seri hanya ditangkap setelah properti seri khusus ditambahkan (properti acak saja tidak menangkapnya, jadi saya menambahkannya). Tidak ada bug ditemukan pada kode `Money` itu sendiri.
