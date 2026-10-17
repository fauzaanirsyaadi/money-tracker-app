-- Phase 8: covering index for account balances.
-- `SUM(amount_minor) WHERE account_id = ?` is how every balance is computed (no
-- stored balance, so nothing can drift). With only (account_id, occurred_on) it
-- reads the table for each row; with the amount in the index it never touches the
-- table: 33.5 ms -> 3.0 ms for the account list at 100 000 rows (release build,
-- measured by `perf_100k_rows`, docs/ARCHITECTURE.md section 33).

CREATE INDEX idx_tx_account_amount ON transactions(account_id, amount_minor);
