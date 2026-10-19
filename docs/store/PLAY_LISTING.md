# Isian Google Play Console untuk Money Tracker

Draf untuk disalin ke Play Console. Sesuaikan jika fitur berubah.

## Detail aplikasi
| Field | Isi |
|---|---|
| Nama aplikasi (maks. 30) | `Money Tracker` |
| Kategori | Keuangan |
| Tag | Pelacak keuangan, anggaran |
| Email kontak | **[ISI]** |
| Situs web | (opsional) `https://github.com/fauzaanirsyaadi/money-tracker-app` |
| Kebijakan privasi | URL halaman `PRIVACY_POLICY.md` (lihat catatan di bawah) |

## Deskripsi singkat (maks. 80 karakter)
**Indonesia (id):** `Catat uang, anggaran, dan laporan. Offline, terenkripsi, tanpa akun.`

**English (en-US):** `Track money, budgets and reports. Offline, encrypted, no account.`

## Deskripsi lengkap (maks. 4000 karakter), Indonesia

```
Money Tracker adalah pencatat keuangan pribadi yang bekerja sepenuhnya di ponsel Anda. Tanpa akun, tanpa server, tanpa iklan, dan tanpa izin internet: data Anda tidak pernah meninggalkan perangkat.

FITUR
• Rekening: dompet, bank, atau tabungan, masing-masing dengan saldo awal.
• Transaksi: catat pemasukan, pengeluaran, dan transfer antar rekening dengan cepat.
• Kategori dan sub-kategori untuk merapikan pengeluaran.
• Anggaran bulanan per kategori, dengan peringatan saat hampir habis atau terlampaui.
• Transaksi berulang untuk tagihan dan pemasukan rutin.
• Laporan: pengeluaran per kategori, tren 6 bulan, dan kekayaan bersih.

PRIVASI DAN KEAMANAN
• Sepenuhnya offline: aplikasi tidak meminta izin internet.
• Basis data terenkripsi (AES-256) dengan kunci yang dilindungi Android Keystore.
• Kunci aplikasi dengan PIN atau sidik jari/wajah, dengan jeda otomatis jika PIN salah berulang kali.
• Tangkapan layar diblokir dan isi disembunyikan di pratinjau aplikasi terbaru.
• Tanpa pelacak dan tanpa analitik.

CADANGAN
Buat cadangan terenkripsi dengan frasa sandi Anda sendiri, simpan ke lokasi pilihan Anda, dan pulihkan di perangkat baru. Karena tidak ada server, cadangan adalah satu-satunya cara memindahkan atau memulihkan data. Jika Anda lupa PIN atau frasa sandi, data tidak dapat dipulihkan.

Money Tracker dibuat untuk Anda yang ingin mencatat keuangan dengan tenang, tanpa menyerahkan data ke siapa pun.
```

## Deskripsi lengkap, English (en-US)

```
Money Tracker is a personal finance tracker that works entirely on your phone. No account, no server, no ads and no internet permission: your data never leaves your device.

FEATURES
• Accounts: wallet, bank or savings, each with an opening balance.
• Transactions: quickly record income, expenses and transfers between accounts.
• Categories and sub-categories to organise spending.
• Monthly budgets per category, with warnings when you are close to or over the limit.
• Recurring transactions for regular bills and income.
• Reports: spending by category, a 6-month trend and net worth.

PRIVACY AND SECURITY
• Fully offline: the app requests no internet permission.
• Encrypted database (AES-256) with a key protected by the Android Keystore.
• App lock with a PIN or fingerprint/face, with an automatic delay after repeated wrong PINs.
• Screenshots are blocked and content is hidden in the recent-apps preview.
• No trackers and no analytics.

BACKUP
Create an encrypted backup with your own passphrase, save it wherever you choose, and restore it on a new device. Since there is no server, a backup is the only way to move or recover your data. If you forget your PIN or passphrase, your data cannot be recovered.
```

## Aset yang masih harus Anda siapkan
- Ikon 512×512 PNG, feature graphic 1024×500, minimal 2 screenshot ponsel (**perhatikan:** aplikasi memblokir tangkapan layar lewat `FLAG_SECURE`. Ambil screenshot dari build debug, bukan rilis).

## Konten aplikasi (isian deklarasi)
| Bagian | Jawaban |
|---|---|
| Akses aplikasi | Semua fungsi tersedia tanpa akun atau login |
| Iklan | Tidak berisi iklan |
| Peringkat konten | Kuesioner IARC: tanpa kekerasan, seksual, judi, dll. Hasilnya biasanya "semua usia" |
| Target audiens | 18 tahun ke atas |
| Aplikasi keuangan | Pelacak/pencatat pengeluaran saja: tidak ada pinjaman, tidak ada transaksi uang nyata, tidak ada dompet atau pembayaran |
| Aplikasi berita, COVID, pemerintah | Tidak |

## Keamanan data (Data safety)
| Pertanyaan | Jawaban |
|---|---|
| Apakah aplikasi mengumpulkan atau membagikan data pengguna yang diperlukan? | **Tidak.** Semua data diproses dan disimpan di perangkat, tidak ada yang dikirim ke pengembang atau pihak ketiga |
| Apakah semua data pengguna dienkripsi saat transit? | Tidak berlaku (tidak ada transmisi data; aplikasi tanpa izin internet) |
| Apakah pengguna dapat meminta penghapusan data? | Data ada di perangkat: dihapus lewat aplikasi atau dengan mencopot pemasangan |
| Kebijakan privasi | URL kebijakan privasi |

Karena tidak ada data yang meninggalkan perangkat, semua kategori data diisi "tidak dikumpulkan".

## Catatan hosting kebijakan privasi
Play memerlukan URL publik. Dua pilihan:
1. **GitHub Pages**: aktifkan di Settings → Pages (branch `main`, folder `/docs`) agar `docs/store/PRIVACY_POLICY.md` terbit sebagai halaman. Ini mempublikasikan konten repo, jadi pastikan Anda setuju.
2. **Tautan berkas di GitHub** (repo harus publik): `https://github.com/fauzaanirsyaadi/money-tracker-app/blob/main/docs/store/PRIVACY_POLICY.md`. Lebih sederhana tetapi kurang rapi.
