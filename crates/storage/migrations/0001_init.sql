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
