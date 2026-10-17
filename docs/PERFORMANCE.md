# Performance numbers

**Host numbers are relative only.** Everything below was measured on a developer Mac
(Apple silicon, release build, SQLCipher in memory) or on an arm64 emulator under host
load. Use them to compare *before* and *after* a change, never as promises about a phone.
Real-device numbers go in `docs/DEVICE_CHECKLIST.md` and replace these.

## Queries at 100 000 transactions (host, release)

Run: `cargo test -p mt-storage --release -- --ignored --nocapture perf_100k`.
`EXPLAIN QUERY PLAN` is also asserted on every normal `cargo test` (see
`crates/storage/src/perf.rs`), on exactly the SQL the app runs.

| Query | Before | After | Plan |
|---|---|---|---|
| List page of 50, newest first | 0.07 ms | same | `idx_tx_date`, no sort |
| Next page (cursor) | 2.7 ms | 0.06 ms | row value `(occurred_on, id) < (?, ?)` is one index range (the OR form was not) |
| One account / one month / expenses only | 0.05-0.07 ms | same | `idx_tx_account` / `idx_tx_date`, no sort |
| One category with its children | 12 ms | same | two index searches combined, then sorted (accepted) |
| Text search | 21 ms | same | scan (nothing can index `LIKE '%x%'`; accepted) |
| Account list (a balance each) | 33.5 ms | 3.0 ms | migration 0003, covering index `(account_id, amount_minor)` |
| Net worth | 33 ms | about 3 ms | same balances (not measured separately) |
| Spending report, month / year | 1.1 / 13 ms | same | `idx_tx_date` |
| 12-month trend | 26 ms | same | `idx_tx_date` |

Stored balances were considered and rejected: with the covering index a balance costs
3 ms at 100 000 rows, and a computed balance cannot drift.

## Crypto and backup (host, release)

| What | Time | Test |
|---|---|---|
| One PIN check, Argon2id 19 MiB t=2 p=1 | 17 ms | `perf_pin_hash_with_the_production_cost` |
| Export and encrypt 100 000 rows (Argon2id 64 MiB t=3) | 206 ms, 8.0 MB file | `big_backup_round_trip_with_timing` |
| Decrypt, restore and verify 100 000 rows | 1.7 s | same |

## Start-up (arm64 emulator, release build, under host load)

Measured with identical temporary timing markers on both builds, six cold starts each
(one outlier per side). Emulator numbers swing a lot with host load.

| | Before | After (commit 5c02750) |
|---|---|---|
| Time until the first `lock_status` answer | 6.1-7.4 s | 4.0-5.3 s |
| First frame (`am start -W`) | 2.4-3.1 s | 1.6-2.1 s |

Whether the start-up change earns its complexity is **undecided until real-device numbers
exist**: if a low-end phone shows no gain, it is a candidate to revert (see
docs/ARCHITECTURE.md section 33).

## Size

Release APK, arm64 only: about 12 MB (`scripts/build_app.sh`). JS bundle budget is
enforced in CI by `scripts/bundle_size.py`.

## Not measured yet (needs a real low-end phone)

Everything in `docs/DEVICE_CHECKLIST.md`. No optimisation beyond what is above until
those numbers exist.
