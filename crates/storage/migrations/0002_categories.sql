-- Categories: unique active names per kind and parent, plus the default set.
-- Names are Indonesian; the UI can offer a language choice later.

CREATE UNIQUE INDEX uq_categories_active_name
  ON categories(kind, lower(name), COALESCE(parent_id, 0)) WHERE archived = 0;

INSERT INTO categories(name, kind) VALUES
  ('Makanan', 'expense'),
  ('Transportasi', 'expense'),
  ('Belanja', 'expense'),
  ('Tagihan', 'expense'),
  ('Hiburan', 'expense'),
  ('Kesehatan', 'expense'),
  ('Pendidikan', 'expense'),
  ('Lainnya', 'expense'),
  ('Gaji', 'income'),
  ('Bonus', 'income'),
  ('Hadiah', 'income'),
  ('Lainnya', 'income');
