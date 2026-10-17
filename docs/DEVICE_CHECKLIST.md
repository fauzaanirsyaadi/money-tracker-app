# Device checklist (one page, real low-end phone)

Install the test APK first: `scripts/build_app.sh --install` (debug-signed, for your own
phone only). Use a **low-end** phone (2-3 GB RAM, an older Android) and the release build.
Write the numbers in the right-hand column and bring them back; nothing gets optimised
before they exist.

Phone: ______________  Android: ____  RAM: ____  Build: `scripts/build_app.sh` commit ______

## A. Numbers

| # | Measure | How | Result |
|---|---|---|---|
| 1 | **Cold start** (first frame), 5 runs | `adb shell am force-stop com.moneytracker.app; adb shell am start -W -n com.moneytracker.app/.MainActivity` (TotalTime) | ____ ____ ____ ____ ____ ms |
| 2 | Time until the lock screen or list is usable | stopwatch from tap to a responsive screen, 3 runs | ____ ____ ____ s |
| 3 | **Unlock with PIN** (PIN check is Argon2id 19 MiB) | stopwatch from tapping Buka to the list, 5 runs | ____ s |
| 4 | **Backup export** (Argon2id 64 MiB), small data | Settings, make a backup, stopwatch from Buat cadangan to the picker | ____ s |
| 5 | Backup restore, small data | stopwatch from confirm to done | ____ s |
| 6 | **List scroll with lots of rows**: restore a 100 000-row backup (ask for the test file), open Transaksi, fling | smoothness: smooth / hitches / stalls; any ANR dialog? | ____ |
| 7 | Search in the list with 100 000 rows | stopwatch from typing to results | ____ s |
| 8 | **Memory peak** | `adb shell dumpsys meminfo com.moneytracker.app` (TOTAL PSS) after start, after export, after restore | ____ / ____ / ____ MB |
| 9 | APK size on disk | `ls -l` of the APK | ____ MB |
| 10 | JS bundle (gzip) | CI prints it (`scripts/bundle_size.py`) | ____ KB |

Argon2 is memory-bound: if #3 is over about 1 s or #4 over about 5 s, say so; the
parameters can be lowered, at a cost the security notes spell out.

## B. Behaviour that only a real phone can prove

| # | Check | Pass |
|---|---|---|
| 1 | Biometric unlock with a fingerprint **and** with face (class 2 devices); adding a fingerprint switches biometrics off with an explanation and the PIN still opens the app | [ ] |
| 2 | Keystore level: `adb logcat` or the app's note says tee/strongbox; app restarts and device reboots keep the data readable | [ ] |
| 3 | A **user** (non-debuggable) build exposes no DevTools socket: `adb shell cat /proc/net/unix \| grep devtools` finds nothing | [ ] |
| 4 | Backup to **Google Drive** and to an **SD card** through the system picker, then restore both | [ ] |
| 5 | **Back** button inside sheets and screens; screen **rotation** keeps the typed input or loses nothing important | [ ] |
| 6 | **Process killed by the system** (Developer options, "Don't keep activities", or `adb shell am kill com.moneytracker.app`): reopening asks for the PIN and shows the same data | [ ] |
| 7 | Backgrounding with auto-lock "Segera": returning always shows the lock screen; the task switcher shows no content | [ ] |
| 8 | Start-up while the phone is busy or just booted: if the "belum selesai memulai" screen ever appears, note when | [ ] |

## C. Do before any of it

* `.github/CODEOWNERS`: put your GitHub username in place of the placeholder.
* Run `gitleaks detect --source . --log-opts="--all"`, push to a **private** repo, watch the
  **first CI run** (many guarantees depend on CI having run once).
* Decide F18 (minSdk 24 or 28) and make your release keystore (outside the repo, backed up).
