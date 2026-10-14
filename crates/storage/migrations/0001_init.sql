-- Schema v1. See docs/ARCHITECTURE.md section 5 for the rationale.

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
) STRICT;

CREATE TABLE accounts (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
  kind TEXT NOT NULL CHECK (kind IN ('cash','bank','ewallet')),
  currency TEXT NOT NULL,
  opening_balance_minor INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1)),
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE categories (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
  kind TEXT NOT NULL CHECK (kind IN ('income','expense')),
  parent_id INTEGER REFERENCES categories(id) ON DELETE RESTRICT,
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1))
) STRICT;

CREATE TABLE recurring_rules (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  category_id INTEGER NOT NULL REFERENCES categories(id),
  kind TEXT NOT NULL CHECK (kind IN ('income','expense')),
  amount_minor INTEGER NOT NULL,
  note TEXT NOT NULL DEFAULT '',
  freq TEXT NOT NULL CHECK (freq IN ('daily','weekly','monthly','yearly')),
  every_n INTEGER NOT NULL DEFAULT 1 CHECK (every_n >= 1),
  anchor_day INTEGER NOT NULL CHECK (anchor_day BETWEEN 1 AND 31),
  start_on TEXT NOT NULL,
  end_on TEXT,
  next_due TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  CHECK ((kind = 'income' AND amount_minor > 0) OR (kind = 'expense' AND amount_minor < 0))
) STRICT;

CREATE TABLE transactions (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  category_id INTEGER REFERENCES categories(id),
  amount_minor INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('income','expense','transfer_out','transfer_in')),
  note TEXT NOT NULL DEFAULT '' CHECK (length(note) <= 500),
  occurred_on TEXT NOT NULL,
  transfer_group_id INTEGER,
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
