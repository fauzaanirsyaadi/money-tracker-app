//! Opening and migrating the encrypted database.

use std::path::Path;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::StorageError;

/// Ordered, append-only list of migrations. Migration `n` (1-based) moves
/// `PRAGMA user_version` from `n - 1` to `n`.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_categories.sql"),
    include_str!("../migrations/0003_balance_index.sql"),
];

/// Raw 256-bit SQLCipher key. Generated and wrapped by the platform layer.
///
/// A newtype rather than `Zeroizing<[u8; 32]>` on its own because the library
/// type prints its bytes with `{:?}`. This one prints `DbKey(<redacted>)`, is
/// wiped on drop, and offers no `Display`.
#[derive(Clone)]
pub struct DbKey(Zeroizing<[u8; 32]>);

impl DbKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

impl std::ops::Deref for DbKey {
    type Target = [u8; 32];

    fn deref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbKey(<redacted>)")
    }
}

impl From<[u8; 32]> for DbKey {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

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
        Self::init_to(conn, key, Self::SCHEMA_VERSION)
    }

    /// Like [`Db::init`] but stops migrating at `target`, so a backup can be
    /// loaded into the schema it was written with and migrated afterwards.
    pub(crate) fn init_to(
        conn: Connection,
        key: &DbKey,
        target: u32,
    ) -> Result<Self, StorageError> {
        // Plain SQLite silently ignores `PRAGMA key` and would write a
        // plaintext file, so prove SQLCipher is linked before touching the key.
        require_cipher(cipher_version(&conn)?)?;
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
        db.migrate_to(target)?;
        Ok(db)
    }

    /// Applies the migrations after the current version, up to the latest.
    pub(crate) fn migrate(&self) -> Result<(), StorageError> {
        self.migrate_to(Self::SCHEMA_VERSION)
    }

    /// Applies the migrations after the stored version, up to `target`.
    ///
    /// Each step reads `user_version` **inside** its own `BEGIN IMMEDIATE`
    /// transaction. Reading it first and locking afterwards would let two
    /// connections (two processes, or a restore running beside the app) both
    /// decide the same migration is due, and the second would apply it twice.
    fn migrate_to(&self, target: u32) -> Result<(), StorageError> {
        let target = target.min(Self::SCHEMA_VERSION);
        // A database from a newer app is refused before anything is touched.
        let found = self.schema_version()?;
        if found > Self::SCHEMA_VERSION {
            return Err(StorageError::SchemaTooNew {
                found,
                supported: Self::SCHEMA_VERSION,
            });
        }
        for (index, sql) in MIGRATIONS.iter().enumerate().take(target as usize) {
            let version = u32::try_from(index + 1).map_err(|_| StorageError::InvalidData)?;
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            let step = self.apply_if_due(version, sql);
            match step {
                Ok(()) => self.conn.execute_batch("COMMIT")?,
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    /// Runs inside the write transaction opened by [`Db::migrate_to`].
    fn apply_if_due(&self, version: u32, sql: &str) -> Result<(), StorageError> {
        let found = self.schema_version()?;
        if found > Self::SCHEMA_VERSION {
            return Err(StorageError::SchemaTooNew {
                found,
                supported: Self::SCHEMA_VERSION,
            });
        }
        if found >= version {
            // Another connection applied it while we waited for the lock.
            return Ok(());
        }
        self.conn.execute_batch(sql)?;
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {version};"))?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

/// `PRAGMA cipher_version` returns a row only on SQLCipher builds.
fn cipher_version(conn: &Connection) -> Result<Option<String>, StorageError> {
    match conn.query_row("PRAGMA cipher_version", [], |r| r.get::<_, String>(0)) {
        Ok(version) => Ok(Some(version)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn require_cipher(version: Option<String>) -> Result<(), StorageError> {
    match version {
        Some(v) if !v.trim().is_empty() => Ok(()),
        _ => Err(StorageError::EncryptionUnavailable),
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
        DbKey::new([byte; 32])
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
        db.conn()
            .execute_batch(
                "INSERT INTO accounts(name,kind,currency,created_at) VALUES('Bank','bank','IDR',0);",
            )
            .unwrap();
        db
    }

    const OUT_LEG: &str = "INSERT INTO transactions(id,account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                           VALUES(10,1,-500,'transfer_out','2026-10-14',10,0)";

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
            .query_row("SELECT name FROM categories WHERE name = 'Food'", [], |r| {
                r.get(0)
            })
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
    fn a_version_1_database_upgrades_and_keeps_its_data() {
        let dir = TempDir::new();
        let path = dir.file();
        {
            // Build exactly what the first release would have left on disk.
            let conn = Connection::open(&path).unwrap();
            apply_key(&conn, &key(5)).unwrap();
            conn.execute_batch(&format!(
                "BEGIN;\n{}\nPRAGMA user_version = 1;\nCOMMIT;
                 INSERT INTO accounts(name,kind,currency,opening_balance_minor,created_at)
                 VALUES('Lama','cash','IDR',42,0);",
                MIGRATIONS[0]
            ))
            .unwrap();
        }
        let db = Db::open(&path, &key(5)).unwrap();
        assert_eq!(db.schema_version(), Ok(Db::SCHEMA_VERSION));
        let categories: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(categories, 12);
        let opening: i64 = db
            .conn()
            .query_row("SELECT opening_balance_minor FROM accounts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(opening, 42);
        // Opening it again must not seed a second time.
        drop(db);
        let again = Db::open(&path, &key(5)).unwrap();
        let categories: i64 = again
            .conn()
            .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(categories, 12);
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
        let ok = "INSERT INTO transactions(id,account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                  VALUES(1,1,-100,'transfer_out','2026-10-14',1,0)";
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
        assert_eq!(sql(&db, insert), Err(StorageError::Duplicate));
    }

    #[test]
    fn budgets_are_unique_per_category_and_month() {
        let db = seeded();
        let insert =
            "INSERT INTO budgets(category_id,month,limit_minor) VALUES(1,'2026-10',1000000)";
        assert_eq!(sql(&db, insert), Ok(1));
        assert_eq!(sql(&db, insert), Err(StorageError::Duplicate));
        let bad_month = "INSERT INTO budgets(category_id,month,limit_minor) VALUES(1,'Oct-26',1)";
        assert_eq!(sql(&db, bad_month), Err(StorageError::Constraint));
    }

    // --- hardening (review findings H1, M1, M2, M3, L11) -------------------

    #[test]
    fn cipher_is_required() {
        assert_eq!(
            require_cipher(None),
            Err(StorageError::EncryptionUnavailable)
        );
        assert_eq!(
            require_cipher(Some("  ".into())),
            Err(StorageError::EncryptionUnavailable)
        );
        assert_eq!(require_cipher(Some("4.6.1 community".into())), Ok(()));
    }

    #[test]
    fn this_build_links_sqlcipher() {
        let conn = Connection::open_in_memory().unwrap();
        let version = cipher_version(&conn).unwrap();
        assert!(version.is_some_and(|v| !v.is_empty()));
    }

    #[test]
    fn money_columns_are_bounded_like_mt_core() {
        let db = seeded();
        let max = mt_core::MAX_MINOR;
        let income = |amount: i64| {
            format!(
                "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
                 VALUES(1,1,{amount},'income','2026-10-14',0)"
            )
        };
        assert_eq!(sql(&db, &income(max)), Ok(1));
        assert_eq!(sql(&db, &income(max + 1)), Err(StorageError::Constraint));
        assert_eq!(sql(&db, &income(i64::MAX)), Err(StorageError::Constraint));

        let expense = format!(
            "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
             VALUES(1,1,{},'expense','2026-10-14',0)",
            -max - 1
        );
        assert_eq!(sql(&db, &expense), Err(StorageError::Constraint));

        let opening = format!(
            "INSERT INTO accounts(name,kind,currency,opening_balance_minor,created_at)
             VALUES('Big','bank','IDR',{},0)",
            max + 1
        );
        assert_eq!(sql(&db, &opening), Err(StorageError::Constraint));

        let limit = format!(
            "INSERT INTO budgets(category_id,month,limit_minor) VALUES(1,'2026-11',{})",
            max + 1
        );
        assert_eq!(sql(&db, &limit), Err(StorageError::Constraint));
    }

    #[test]
    fn dates_must_be_canonical_and_real() {
        let db = seeded();
        let insert = |date: &str| {
            format!(
                "INSERT INTO transactions(account_id,category_id,amount_minor,kind,occurred_on,created_at)
                 VALUES(1,1,-1,'expense','{date}',0)"
            )
        };
        assert_eq!(sql(&db, &insert("2026-02-28")), Ok(1));
        assert_eq!(sql(&db, &insert("2024-02-29")), Ok(1)); // leap day
        for bad in [
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "kemarin",
            "2026-1-1",
            "2026-10-14 10:00",
            "",
        ] {
            assert_eq!(
                sql(&db, &insert(bad)),
                Err(StorageError::Constraint),
                "{bad:?}"
            );
        }

        let rule = |start: &str, end: &str, next: &str| {
            format!(
                "INSERT INTO recurring_rules(account_id,category_id,kind,amount_minor,freq,anchor_day,start_on,end_on,next_due)
                 VALUES(1,1,'expense',-5,'monthly',1,'{start}',{end},'{next}')"
            )
        };
        assert_eq!(sql(&db, &rule("2026-01-01", "NULL", "2026-01-01")), Ok(1));
        assert_eq!(
            sql(&db, &rule("2026-01-01", "'2026-12-31'", "2026-02-01")),
            Ok(1)
        );
        assert_eq!(
            sql(&db, &rule("2026-01-32", "NULL", "2026-01-01")),
            Err(StorageError::Constraint)
        );
        assert_eq!(
            sql(&db, &rule("2026-01-01", "'soon'", "2026-01-01")),
            Err(StorageError::Constraint)
        );
        assert_eq!(
            sql(&db, &rule("2026-01-01", "NULL", "2026-02-30")),
            Err(StorageError::Constraint)
        );

        let month = |m: &str| {
            format!("INSERT INTO budgets(category_id,month,limit_minor) VALUES(1,'{m}',1)")
        };
        assert_eq!(sql(&db, &month("2026-12")), Ok(1));
        assert_eq!(sql(&db, &month("2026-13")), Err(StorageError::Constraint));
        assert_eq!(sql(&db, &month("2026-00")), Err(StorageError::Constraint));
    }

    #[test]
    fn a_complete_transfer_is_accepted() {
        let db = seeded();
        assert_eq!(sql(&db, OUT_LEG), Ok(1));
        let in_leg = "INSERT INTO transactions(account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                      VALUES(2,500,'transfer_in','2026-10-14',10,0)";
        assert_eq!(sql(&db, in_leg), Ok(1));
        // Removing both legs in one statement is allowed.
        assert_eq!(
            sql(&db, "DELETE FROM transactions WHERE transfer_group_id = 10"),
            Ok(2)
        );
    }

    #[test]
    fn an_incoming_leg_must_mirror_an_outgoing_leg() {
        let db = seeded();
        let in_leg = |account: i64, amount: i64, group: i64| {
            format!(
                "INSERT INTO transactions(account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                 VALUES({account},{amount},'transfer_in','2026-10-14',{group},0)"
            )
        };
        // Nothing to mirror yet.
        assert_eq!(sql(&db, &in_leg(2, 500, 10)), Err(StorageError::Constraint));
        sql(&db, OUT_LEG).unwrap();
        // Wrong amount, same account, unknown group.
        assert_eq!(sql(&db, &in_leg(2, 499, 10)), Err(StorageError::Constraint));
        assert_eq!(sql(&db, &in_leg(1, 500, 10)), Err(StorageError::Constraint));
        assert_eq!(sql(&db, &in_leg(2, 500, 99)), Err(StorageError::Constraint));
        // The valid leg works once, and a second one is a duplicate.
        assert_eq!(sql(&db, &in_leg(2, 500, 10)), Ok(1));
        assert_eq!(sql(&db, &in_leg(2, 500, 10)), Err(StorageError::Duplicate));
    }

    #[test]
    fn an_outgoing_leg_must_name_itself_as_the_group() {
        let db = seeded();
        let wrong = "INSERT INTO transactions(id,account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                     VALUES(10,1,-500,'transfer_out','2026-10-14',11,0)";
        assert_eq!(sql(&db, wrong), Err(StorageError::Constraint));
    }

    #[test]
    fn transfer_legs_cannot_be_orphaned_or_rewritten() {
        let db = seeded();
        sql(&db, OUT_LEG).unwrap();
        sql(&db, "INSERT INTO transactions(account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                  VALUES(2,500,'transfer_in','2026-10-14',10,0)").unwrap();
        // Deleting only the outgoing leg would orphan the incoming one.
        assert_eq!(
            sql(&db, "DELETE FROM transactions WHERE id = 10"),
            Err(StorageError::Constraint)
        );
        // Changing a leg's amount or account would unbalance the pair.
        assert_eq!(
            sql(
                &db,
                "UPDATE transactions SET amount_minor = -400 WHERE id = 10"
            ),
            Err(StorageError::Constraint)
        );
        assert_eq!(
            sql(&db, "UPDATE transactions SET account_id = 2 WHERE id = 10"),
            Err(StorageError::Constraint)
        );
        // Notes and dates stay editable.
        assert_eq!(
            sql(
                &db,
                "UPDATE transactions SET note = 'rent' WHERE transfer_group_id = 10"
            ),
            Ok(2)
        );
    }

    #[test]
    fn a_category_cannot_be_its_own_parent() {
        let db = seeded();
        assert_eq!(
            sql(&db, "UPDATE categories SET parent_id = 1 WHERE id = 1"),
            Err(StorageError::Constraint)
        );
    }

    #[test]
    fn two_connections_opening_an_old_database_apply_each_migration_once() {
        use std::sync::{Arc, Barrier};

        for round in 0..12 {
            let dir = TempDir::new();
            let path = dir.file();
            // A version-1 database, as an older app left it.
            {
                let conn = Connection::open(&path).unwrap();
                let db = Db::init_to(conn, &key(7), 1).unwrap();
                assert_eq!(db.schema_version(), Ok(1));
            }
            let barrier = Arc::new(Barrier::new(4));
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                    std::thread::spawn(move || {
                        barrier.wait();
                        Db::open(&path, &key(7)).map(|db| db.schema_version())
                    })
                })
                .collect();
            for handle in handles {
                assert_eq!(
                    handle.join().unwrap(),
                    Ok(Ok(Db::SCHEMA_VERSION)),
                    "round {round}"
                );
            }
            // Migration 0002 seeds the default categories: exactly once.
            let db = Db::open(&path, &key(7)).unwrap();
            let seeded: i64 = db
                .conn()
                .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
                .unwrap();
            assert_eq!(seeded, 12, "round {round}: a migration ran twice");
        }
    }

    #[test]
    fn a_failed_migration_step_rolls_back_and_leaves_the_version_alone() {
        let dir = TempDir::new();
        let path = dir.file();
        {
            let db = Db::init_to(Connection::open(&path).unwrap(), &key(7), 1).unwrap();
            // Make migration 0002 fail: its unique index cannot be created over duplicates.
            db.conn()
                .execute_batch(
                    "INSERT INTO categories(name,kind) VALUES('Makanan','expense'),('Makanan','expense');",
                )
                .unwrap();
        }
        assert!(Db::open(&path, &key(7)).is_err());
        let conn = Connection::open(&path).unwrap();
        apply_key(&conn, &key(7)).unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1, "the failed step must not advance the version");
        let in_txn = !conn.is_autocommit();
        assert!(!in_txn, "no transaction may be left open");
    }

    #[test]
    fn a_key_never_prints_its_bytes() {
        let key = DbKey::new([0x5a; 32]);
        let shown = [
            format!("{key:?}"),
            format!("{key:#?}"),
            format!("{:?}", Some(&key)),
            format!("{:?}", vec![key.clone()]),
            format!("{:?}", Ok::<_, ()>(key.clone())),
        ];
        for text in shown {
            assert!(text.contains("<redacted>"), "{text}");
            for leak in ["90", "5a", "5A", "0x5a", "[90"] {
                assert!(!text.contains(leak), "{text} leaked {leak}");
            }
        }
        // It still works as a key: the bytes are reachable on purpose, not by printing.
        assert_eq!(key.len(), 32);
        assert!(key.iter().all(|b| *b == 0x5a));
        // ... and a copy of it does not share a printable form either.
        assert_eq!(format!("{:?}", key.clone()), "DbKey(<redacted>)");
    }
}
