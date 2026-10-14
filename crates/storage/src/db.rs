//! Opening and migrating the encrypted database.

use std::path::Path;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::StorageError;

/// Ordered, append-only list of migrations. Migration `n` (1-based) moves
/// `PRAGMA user_version` from `n - 1` to `n`.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

/// Raw 256-bit SQLCipher key. Generated and wrapped by the platform layer.
pub type DbKey = Zeroizing<[u8; 32]>;

/// An open, unlocked, migrated database.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Schema version this build understands.
    pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

    /// Opens (creating if needed) the database at `path` with `key`.
    pub fn open(path: &Path, key: &DbKey) -> Result<Self, StorageError> {
        Self::init(Connection::open(path)?, key)
    }

    /// In-memory database, for tests.
    pub fn open_in_memory(key: &DbKey) -> Result<Self, StorageError> {
        Self::init(Connection::open_in_memory()?, key)
    }

    fn init(conn: Connection, key: &DbKey) -> Result<Self, StorageError> {
        apply_key(&conn, key)?;
        // Fails with NotADatabase when the key is wrong, before any other work.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA secure_delete = ON;",
        )?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let found: u32 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if found > Self::SCHEMA_VERSION {
            return Err(StorageError::SchemaTooNew {
                found,
                supported: Self::SCHEMA_VERSION,
            });
        }
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
            let version = index + 1;
            self.conn.execute_batch(&format!(
                "BEGIN IMMEDIATE;\n{sql}\nPRAGMA user_version = {version};\nCOMMIT;"
            ))?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// `PRAGMA key = "x'<64 hex>'"` selects a raw key and skips SQLCipher's KDF.
fn apply_key(conn: &Connection, key: &DbKey) -> Result<(), StorageError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut literal = Zeroizing::new(String::with_capacity(67));
    literal.push_str("x'");
    for byte in key.iter() {
        literal.push(char::from(HEX[usize::from(byte >> 4)]));
        literal.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    literal.push('\'');
    conn.pragma_update(None, "key", literal.as_str())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU32, Ordering},
    };

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("mt-storage-{}-{n}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("money.db")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn key(byte: u8) -> DbKey {
        Zeroizing::new([byte; 32])
    }

    fn sql(db: &Db, statement: &str) -> Result<usize, StorageError> {
        Ok(db.conn().execute(statement, [])?)
    }

    const SEED: &str =
        "INSERT INTO accounts(name,kind,currency,created_at) VALUES('Cash','cash','IDR',0);
                        INSERT INTO categories(name,kind) VALUES('Food','expense');";

    fn seeded() -> Db {
        let db = Db::open_in_memory(&key(1)).unwrap();
        db.conn().execute_batch(SEED).unwrap();
        db
    }

    #[test]
    fn creates_schema_at_current_version() {
        let db = Db::open_in_memory(&key(1)).unwrap();
        assert_eq!(db.schema_version(), Ok(Db::SCHEMA_VERSION));
        let tables: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 6);
    }

    #[test]
    fn file_is_encrypted_and_needs_the_key() {
        let dir = TempDir::new();
        let path = dir.file();
        {
            let db = Db::open(&path, &key(7)).unwrap();
            db.conn().execute_batch(SEED).unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        assert!(
            !bytes.starts_with(b"SQLite format 3"),
            "header must not be plaintext"
        );
        assert!(
            !bytes.windows(4).any(|w| w == b"Food"),
            "content must not be readable"
        );

        assert_eq!(
            Db::open(&path, &key(8)).err(),
            Some(StorageError::WrongKeyOrCorrupt)
        );
        // Opening with no key at all must also fail to read.
        let plain = Connection::open(&path).unwrap();
        assert!(
            plain
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                    .get::<_, i64>(0))
                .is_err()
        );

        let again = Db::open(&path, &key(7)).unwrap();
        let name: String = again
            .conn()
            .query_row("SELECT name FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Food");
    }

    #[test]
    fn reopening_does_not_rerun_migrations() {
        let dir = TempDir::new();
        let path = dir.file();
        Db::open(&path, &key(3)).unwrap();
        let db = Db::open(&path, &key(3)).unwrap();
        assert_eq!(db.schema_version(), Ok(Db::SCHEMA_VERSION));
    }

    #[test]
    fn refuses_a_newer_schema() {
        let dir = TempDir::new();
        let path = dir.file();
        {
            let db = Db::open(&path, &key(3)).unwrap();
            db.conn()
                .execute_batch("PRAGMA user_version = 99;")
                .unwrap();
        }
        assert_eq!(
            Db::open(&path, &key(3)).err(),
            Some(StorageError::SchemaTooNew {
                found: 99,
                supported: Db::SCHEMA_VERSION
            })
        );
    }

    #[test]
    fn database_rejects_wrong_signs_for_kind() {
        let db = seeded();
        let expense_positive = "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
                                VALUES(1,1,100,'expense','2026-10-14',0)";
        assert_eq!(sql(&db, expense_positive), Err(StorageError::Constraint));
        let expense_ok = "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
                          VALUES(1,1,-100,'expense','2026-10-14',0)";
        assert_eq!(sql(&db, expense_ok), Ok(1));
    }

    #[test]
    fn transfers_need_a_group_and_no_category() {
        let db = seeded();
        let no_group =
            "INSERT INTO transactions(account_id,amount_minor,kind,occurred_on,created_at)
                        VALUES(1,-100,'transfer_out','2026-10-14',0)";
        assert_eq!(sql(&db, no_group), Err(StorageError::Constraint));
        let with_category = "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                             VALUES(1,1,-100,'transfer_out','2026-10-14',1,0)";
        assert_eq!(sql(&db, with_category), Err(StorageError::Constraint));
        let ok = "INSERT INTO transactions(account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                  VALUES(1,-100,'transfer_out','2026-10-14',1,0)";
        assert_eq!(sql(&db, ok), Ok(1));
    }

    #[test]
    fn income_and_expense_need_a_category() {
        let db = seeded();
        let none = "INSERT INTO transactions(account_id,amount_minor,kind,occurred_on,created_at)
                    VALUES(1,-100,'expense','2026-10-14',0)";
        assert_eq!(sql(&db, none), Err(StorageError::Constraint));
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = seeded();
        let bad = "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
                   VALUES(999,1,-100,'expense','2026-10-14',0)";
        assert_eq!(sql(&db, bad), Err(StorageError::Constraint));
        let in_use = "DELETE FROM categories WHERE id = 1";
        sql(&db, "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
                  VALUES(1,1,-100,'expense','2026-10-14',0)").unwrap();
        assert_eq!(sql(&db, in_use), Err(StorageError::Constraint));
    }

    #[test]
    fn recurring_occurrences_are_unique_per_rule_and_day() {
        let db = seeded();
        sql(&db, "INSERT INTO recurring_rules(account_id,category_id,kind,amount_minor,freq,anchor_day,start_on,next_due)
                  VALUES(1,1,'expense',-500,'monthly',1,'2026-01-01','2026-01-01')").unwrap();
        let insert = "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,recurring_rule_id,created_at)
                      VALUES(1,1,-500,'expense','2026-01-01',1,0)";
        assert_eq!(sql(&db, insert), Ok(1));
        assert_eq!(sql(&db, insert), Err(StorageError::Constraint));
    }

    #[test]
    fn budgets_are_unique_per_category_and_month() {
        let db = seeded();
        let insert =
            "INSERT INTO budgets(category_id,month,limit_minor) VALUES(1,'2026-10',1000000)";
        assert_eq!(sql(&db, insert), Ok(1));
        assert_eq!(sql(&db, insert), Err(StorageError::Constraint));
        let bad_month = "INSERT INTO budgets(category_id,month,limit_minor) VALUES(1,'Oct-26',1)";
        assert_eq!(sql(&db, bad_month), Err(StorageError::Constraint));
    }
}
