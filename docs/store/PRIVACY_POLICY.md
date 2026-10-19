# Kebijakan Privasi Money Tracker

Terakhir diperbarui: 19 Oktober 2026

Money Tracker adalah aplikasi pencatat keuangan pribadi untuk Android yang **bekerja sepenuhnya di perangkat Anda**. Dokumen ini menjelaskan data apa yang diproses aplikasi dan apa yang tidak.

## Ringkasan

- Kami **tidak mengumpulkan** data Anda. Tidak ada akun, server, atau analitik.
- Aplikasi **tidak memiliki izin internet**, sehingga tidak dapat mengirim data apa pun keluar dari perangkat.
- Data keuangan disimpan **terenkripsi** di penyimpanan aplikasi pada perangkat Anda.
- Tidak ada iklan dan tidak ada pelacak pihak ketiga.

## Data yang disimpan di perangkat Anda

Data berikut Anda masukkan sendiri dan hanya disimpan di perangkat:

- rekening, saldo awal, transaksi (pemasukan, pengeluaran, transfer), kategori, anggaran, dan transaksi berulang;
- pengaturan aplikasi, serta data kunci aplikasi (hash PIN dan status biometrik).

Basis data dienkripsi (SQLCipher, AES-256). Kuncinya acak dan dilindungi oleh Android Keystore pada perangkat. Aplikasi menolak tangkapan layar dan menyembunyikan isinya di pratinjau aplikasi terbaru.

## Izin yang diminta

| Izin | Tujuan |
|---|---|
| Biometrik (`USE_BIOMETRIC`, `USE_FINGERPRINT`) | Membuka kunci aplikasi dengan sidik jari atau wajah, bila Anda mengaktifkannya. Data biometrik diproses oleh sistem Android dan tidak pernah dibaca atau disimpan oleh aplikasi. |

Aplikasi **tidak** meminta izin internet, lokasi, kontak, kamera, mikrofon, atau akses ke berkas dan media.

## Cadangan dan ekspor

Anda dapat membuat cadangan terenkripsi (dilindungi frasa sandi yang Anda pilih) dan menyimpannya ke lokasi yang Anda pilih lewat dialog berkas Android. Cadangan hanya dibuat atas permintaan Anda. Kami tidak menerima salinannya. Jika Anda menyimpan cadangan ke layanan pihak ketiga (misalnya Google Drive), layanan itu tunduk pada kebijakan privasinya sendiri. **Jika Anda lupa PIN atau frasa sandi cadangan, kami tidak dapat memulihkan data Anda.**

## Berbagi data

Kami tidak membagikan data apa pun kepada pihak ketiga, karena kami tidak memiliki akses ke data Anda.

## Penyimpanan dan penghapusan

Data berada di perangkat Anda sampai Anda menghapusnya di aplikasi atau mencopot pemasangan aplikasi. Mencopot aplikasi menghapus semua datanya.

## Anak-anak

Aplikasi ini ditujukan untuk pengguna dewasa dan tidak secara sengaja ditujukan kepada anak-anak.

## Perubahan kebijakan

Jika kebijakan ini berubah, versi baru dipublikasikan di alamat yang sama dengan tanggal pembaruan yang baru.

## Kontak

Pertanyaan tentang privasi: **[ISI EMAIL KONTAK ANDA]**

---

# Privacy Policy for Money Tracker

Last updated: October 19, 2026

Money Tracker is a personal finance tracker for Android that **runs entirely on your device**.

- We **do not collect** your data. There are no accounts, servers or analytics.
- The app has **no internet permission**, so it cannot send any data off your device.
- Your financial data is stored **encrypted** (SQLCipher, AES-256) in the app's private storage, with a random key protected by the Android Keystore. Screenshots are blocked and the content is hidden in the recent-apps preview.
- No ads and no third-party trackers.

**Data stored on your device:** accounts, opening balances, transactions, categories, budgets, recurring transactions, settings, and app-lock data (a PIN hash and the biometric on/off state), all entered by you.

**Permissions:** only biometrics (`USE_BIOMETRIC`, `USE_FINGERPRINT`), to unlock the app with your fingerprint or face if you enable it. Biometric data is handled by Android and never read or stored by the app. The app requests no internet, location, contacts, camera, microphone or file/media access.

**Backups:** you can create an encrypted backup (protected by a passphrase you choose) and save it where you want through Android's file dialog. Backups are created only on your request, and we never receive a copy. If you store a backup with a third-party service, that service's own privacy policy applies. If you forget your PIN or backup passphrase, we cannot recover your data.

**Sharing:** we share nothing with third parties because we have no access to your data.

**Deletion:** your data stays on your device until you delete it in the app or uninstall the app, which removes all of it.

**Children:** the app is intended for adults.

**Contact:** **[ADD YOUR CONTACT EMAIL]**
