//! Logical export and import of the whole database.
//!
//! The export is a stream of JSON lines (no SQL, no file pages), which is what
//! makes it portable across schema versions and independent of the device key:
//!
//! ```text
//! {"format":"money-tracker-logical","version":1,"schema":2}
//! {"table":"accounts","columns":["id","name",...],"rows":3}
//! [1,"Dompet","cash",...]            (one line per row)
//! ...
//! {"end":true,"balances":[[1,"1500000"],...]}
//! ```
//!
//! Import never touches a live database: it builds a *new* encrypted file,
//! checks it, and only then hands it back for the caller to swap in.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, Read, Write},
    path::Path,
};

use rusqlite::{Connection, ToSql, types::ValueRef};
use serde_json::{Value, json};

use crate::{
    db::{Db, DbKey},
    error::StorageError,
};

const FORMAT: &str = "money-tracker-logical";
const VERSION: u64 = 1;
/// Longest accepted line. A transaction row is a few hundred bytes.
const MAX_LINE: usize = 1 << 20;

/// Tables in dependency order (parents first). A guard test fails when a
/// migration adds a table that is missing here, so a backup can never silently
/// omit data.
pub const TABLE_ORDER: &[&str] = &[
    "settings",
    "accounts",
    "categories",
    "recurring_rules",
    "transactions",
    "budgets",
];

/// What a backup contains, for the confirmation screen and verification.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogicalSummary {
    pub schema_version: u32,
    /// Row count per table.
    pub rows: BTreeMap<String, u64>,
    /// `(account id, balance in minor units)`, computed the way the app does.
    pub balances: Vec<(i64, i64)>,
}

impl LogicalSummary {
    pub fn count(&self, table: &str) -> u64 {
        self.rows.get(table).copied().unwrap_or(0)
    }
}

fn io_err(_: io::Error) -> StorageError {
    StorageError::Io
}

fn bad<T>(_: T) -> StorageError {
    StorageError::BackupInvalid
}

fn columns_of(conn: &Connection, table: &str) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
    ))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn value_of(value: ValueRef<'_>) -> Result<Value, StorageError> {
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(n) => Ok(json!(n)),
        ValueRef::Text(bytes) => Ok(Value::String(
            std::str::from_utf8(bytes)
                .map_err(|_| StorageError::InvalidData)?
                .to_owned(),
        )),
        // STRICT tables hold only INTEGER and TEXT in this schema.
        ValueRef::Real(_) | ValueRef::Blob(_) => Err(StorageError::InvalidData),
    }
}

fn write_line(out: &mut (impl Write + ?Sized), value: &Value) -> Result<(), StorageError> {
    serde_json::to_writer(&mut *out, value).map_err(|_| StorageError::Io)?;
    out.write_all(b"\n").map_err(io_err)
}

fn balances(db: &Db) -> Result<Vec<(i64, i64)>, StorageError> {
    Ok(db
        .accounts(true)?
        .into_iter()
        .map(|a| (a.id, a.balance.minor()))
        .collect())
}

impl Db {
    /// Writes every table, in a single read snapshot, and returns what it wrote.
    ///
    /// Memory use is one row at a time: callers can stream this into the
    /// encrypting writer.
    pub fn export_logical(
        &self,
        out: &mut (impl Write + ?Sized),
    ) -> Result<LogicalSummary, StorageError> {
        let conn = self.conn();
        let tx = conn.unchecked_transaction()?;
        let schema = self.schema_version()?;
        let mut summary = LogicalSummary {
            schema_version: schema,
            ..LogicalSummary::default()
        };

        write_line(
            out,
            &json!({"format": FORMAT, "version": VERSION, "schema": schema}),
        )?;
        for table in TABLE_ORDER {
            let columns = columns_of(conn, table)?;
            let count: i64 =
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
            write_line(
                out,
                &json!({"table": table, "columns": columns, "rows": count}),
            )?;
            let order = if *table == "settings" { "key" } else { "id" };
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))?;
            let mut rows = stmt.query([])?;
            let mut written = 0u64;
            while let Some(row) = rows.next()? {
                let mut values = Vec::with_capacity(columns.len());
                for index in 0..columns.len() {
                    values.push(value_of(row.get_ref(index)?)?);
                }
                write_line(out, &Value::Array(values))?;
                written += 1;
            }
            if i64::try_from(written) != Ok(count) {
                return Err(StorageError::InvalidData);
            }
            summary.rows.insert((*table).to_owned(), written);
        }
        summary.balances = balances(self)?;
        let balances: Vec<Value> = summary
            .balances
            .iter()
            .map(|(id, minor)| json!([id, minor.to_string()]))
            .collect();
        write_line(out, &json!({"end": true, "balances": balances}))?;
        tx.finish()?;
        Ok(summary)
    }

    /// Builds a **new** encrypted database at `path` from a logical export.
    ///
    /// `path` must not exist. On any failure the partial file is removed. The
    /// returned database is migrated to the current schema and has passed
    /// `integrity_check`, `foreign_key_check`, row counts and balance checks.
    pub fn restore_logical(
        path: &Path,
        key: &DbKey,
        input: impl BufRead,
    ) -> Result<(Db, LogicalSummary), StorageError> {
        if path.exists() {
            return Err(StorageError::Io);
        }
        let result = restore_into(path, key, input);
        if result.is_err() {
            remove_database_files(path);
        }
        result
    }
}

/// Removes a database file and its WAL/SHM companions, ignoring absence.
pub(crate) fn remove_database_files(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = fs::remove_file(Path::new(&name));
    }
}

fn read_line(input: &mut impl BufRead, buf: &mut String) -> Result<bool, StorageError> {
    buf.clear();
    let read = input
        .by_ref()
        .take(MAX_LINE as u64 + 1)
        .read_line(buf)
        .map_err(|_| StorageError::BackupInvalid)?;
    if read == 0 {
        return Ok(false);
    }
    if buf.len() > MAX_LINE {
        return Err(StorageError::BackupInvalid);
    }
    Ok(true)
}

fn next_json(input: &mut impl BufRead, buf: &mut String) -> Result<Value, StorageError> {
    if !read_line(input, buf)? {
        return Err(StorageError::BackupInvalid);
    }
    serde_json::from_str(buf.trim_end()).map_err(bad)
}

fn restore_into(
    path: &Path,
    key: &DbKey,
    mut input: impl BufRead,
) -> Result<(Db, LogicalSummary), StorageError> {
    let mut line = String::new();

    let head = next_json(&mut input, &mut line)?;
    if head.get("format").and_then(Value::as_str) != Some(FORMAT)
        || head.get("version").and_then(Value::as_u64) != Some(VERSION)
    {
        return Err(StorageError::BackupInvalid);
    }
    let schema = head
        .get("schema")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n >= 1)
        .ok_or(StorageError::BackupInvalid)?;
    if schema > Db::SCHEMA_VERSION {
        return Err(StorageError::SchemaTooNew {
            found: schema,
            supported: Db::SCHEMA_VERSION,
        });
    }

    // The same key as the live database: a swap then needs no re-keying.
    let mut db = Db::init_to(Connection::open(path)?, key, schema)?;
    let mut summary = LogicalSummary {
        schema_version: schema,
        ..LogicalSummary::default()
    };

    {
        let conn = db.conn_mut();
        // Rows arrive parents-first, but self-references (categories, transfer
        // legs) and cleaning seeded rows are simpler with checks deferred to the end.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = conn.transaction()?;
        for table in TABLE_ORDER.iter().rev() {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }

        for table in TABLE_ORDER {
            let header = next_json(&mut input, &mut line)?;
            if header.get("table").and_then(Value::as_str) != Some(*table) {
                return Err(StorageError::BackupInvalid);
            }
            let declared: Vec<String> = header
                .get("columns")
                .and_then(Value::as_array)
                .ok_or(StorageError::BackupInvalid)?
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or(StorageError::BackupInvalid)?;
            // Column names are checked against the real schema, so nothing from
            // the file is ever spliced into SQL unverified.
            if declared != columns_of(&tx, table)? {
                return Err(StorageError::BackupInvalid);
            }
            let expected = header
                .get("rows")
                .and_then(Value::as_u64)
                .ok_or(StorageError::BackupInvalid)?;

            let placeholders = vec!["?"; declared.len()].join(",");
            let sql = format!(
                "INSERT INTO {table} ({}) VALUES ({placeholders})",
                declared.join(",")
            );
            let mut insert = tx.prepare(&sql)?;
            for _ in 0..expected {
                let row = next_json(&mut input, &mut line)?;
                let cells = row.as_array().ok_or(StorageError::BackupInvalid)?;
                if cells.len() != declared.len() {
                    return Err(StorageError::BackupInvalid);
                }
                let mut params: Vec<Box<dyn ToSql>> = Vec::with_capacity(cells.len());
                for cell in cells {
                    params.push(match cell {
                        Value::Null => Box::new(Option::<i64>::None),
                        Value::Number(n) => {
                            Box::new(n.as_i64().ok_or(StorageError::BackupInvalid)?)
                        }
                        Value::String(s) => Box::new(s.clone()),
                        _ => return Err(StorageError::BackupInvalid),
                    });
                }
                let refs: Vec<&dyn ToSql> = params.iter().map(AsRef::as_ref).collect();
                insert.execute(refs.as_slice())?;
            }
            drop(insert);
            summary.rows.insert((*table).to_owned(), expected);
        }

        let footer = next_json(&mut input, &mut line)?;
        if footer.get("end").and_then(Value::as_bool) != Some(true) {
            return Err(StorageError::BackupInvalid);
        }
        // Nothing may follow the footer.
        if read_line(&mut input, &mut line)? {
            return Err(StorageError::BackupInvalid);
        }
        tx.commit()?;

        // Checks run on the finished data, with enforcement back on.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let violations: i64 =
            conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        if violations != 0 {
            return Err(StorageError::BackupInvalid);
        }
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(StorageError::BackupInvalid);
        }

        // Counts are compared as loaded: migrating up may seed new rows.
        for (table, expected) in &summary.rows {
            let found: i64 =
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
            if u64::try_from(found) != Ok(*expected) {
                return Err(StorageError::BackupInvalid);
            }
        }

        let recorded = recorded_balances(&footer)?;
        db.migrate()?;
        summary.schema_version = db.schema_version()?;
        summary.balances = balances(&db)?;
        if summary.balances != recorded {
            return Err(StorageError::BackupInvalid);
        }
    }
    Ok((db, summary))
}

fn recorded_balances(footer: &Value) -> Result<Vec<(i64, i64)>, StorageError> {
    footer
        .get("balances")
        .and_then(Value::as_array)
        .ok_or(StorageError::BackupInvalid)?
        .iter()
        .map(|entry| {
            let pair = entry.as_array().filter(|p| p.len() == 2)?;
            let id = pair[0].as_i64()?;
            let minor = pair[1].as_str()?.parse::<i64>().ok()?;
            Some((id, minor))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(StorageError::BackupInvalid)
}

/// Reads and discards a reader; used by tests that only need the byte count.
#[cfg(test)]
fn drain(mut reader: impl Read) -> usize {
    let mut sink = Vec::new();
    reader.read_to_end(&mut sink).unwrap();
    sink.len()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use zeroize::Zeroizing;

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "mt-logical-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
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

    /// A database with every kind of row: custom categories (one a child of
    /// another), a transfer, budgets, a recurring rule and a recurring occurrence.
    fn populated() -> Db {
        let db = Db::open_in_memory(&key(1)).unwrap();
        db.conn()
            .execute_batch(
                "DELETE FROM categories;
                 INSERT INTO settings(key,value) VALUES('currency','IDR');
                 INSERT INTO accounts(id,name,kind,currency,opening_balance_minor,created_at)
                   VALUES(5,'Dompet','cash','IDR',1500000,100),
                         (7,'Tabungan','bank','IDR',2500000,101),
                         (9,'Lama','bank','IDR',0,102);
                 UPDATE accounts SET archived=1 WHERE id=9;
                 INSERT INTO categories(id,name,kind,parent_id) VALUES
                   (40,'Makan','expense',NULL),(41,'Warung','expense',40),(42,'Gaji','income',NULL);
                 INSERT INTO recurring_rules(id,account_id,category_id,kind,amount_minor,note,freq,
                     every_n,anchor_day,start_on,next_due)
                   VALUES(3,5,40,'expense',-50000,'langganan','monthly',1,1,'2026-01-01','2026-11-01');
                 INSERT INTO transactions(id,account_id,category_id,amount_minor,kind,note,occurred_on,created_at)
                   VALUES(1,5,41,-25000,'expense','nasi, \"goreng\" \u{1F35B}','2026-10-01',1),
                         (2,7,42,5000000,'income','gaji','2026-10-02',2);
                 INSERT INTO transactions(id,account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                   VALUES(10,7,-1000000,'transfer_out','2026-10-03',10,3);
                 INSERT INTO transactions(id,account_id,amount_minor,kind,occurred_on,transfer_group_id,created_at)
                   VALUES(11,5,1000000,'transfer_in','2026-10-03',10,3);
                 INSERT INTO transactions(id,account_id,category_id,amount_minor,kind,occurred_on,recurring_rule_id,created_at)
                   VALUES(12,5,40,-50000,'expense','2026-10-01',3,4);
                 INSERT INTO budgets(category_id,month,limit_minor) VALUES(40,'2026-10',300000);",
            )
            .unwrap();
        db
    }

    fn export_bytes(db: &Db) -> (Vec<u8>, LogicalSummary) {
        let mut out = Vec::new();
        let summary = db.export_logical(&mut out).unwrap();
        (out, summary)
    }

    #[test]
    fn restore_reproduces_everything_exactly() {
        let source = populated();
        let (bytes, summary) = export_bytes(&source);

        let dir = TempDir::new();
        let path = dir.0.join("restored.db");
        let (restored, restored_summary) = Db::restore_logical(&path, &key(9), &bytes[..]).unwrap();

        assert_eq!(restored_summary, summary);
        assert_eq!(summary.count("transactions"), 5);
        assert_eq!(summary.count("accounts"), 3);
        // Balances per account, as the app computes them.
        let balances = |db: &Db| {
            db.accounts(true)
                .unwrap()
                .into_iter()
                .map(|a| (a.id, a.name, a.balance.minor(), a.archived))
                .collect::<Vec<_>>()
        };
        assert_eq!(balances(&restored), balances(&source));
        assert_eq!(
            balances(&restored)[0],
            (
                5,
                "Dompet".to_owned(),
                1_500_000 - 25_000 + 1_000_000 - 50_000,
                false
            )
        );
        // The strongest check: exporting the restored database gives the same bytes.
        let (again, _) = export_bytes(&restored);
        assert_eq!(again, bytes);
    }

    #[test]
    fn restore_replaces_seeded_default_categories() {
        let dir = TempDir::new();
        let path = dir.0.join("r.db");
        let (bytes, _) = export_bytes(&populated());
        let (db, _) = Db::restore_logical(&path, &key(2), &bytes[..]).unwrap();
        let names: Vec<String> = db
            .conn()
            .prepare("SELECT name FROM categories ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(names, ["Makan", "Warung", "Gaji"]);
    }

    #[test]
    fn an_empty_database_round_trips() {
        let source = Db::open_in_memory(&key(1)).unwrap();
        let (bytes, summary) = export_bytes(&source);
        let dir = TempDir::new();
        let (db, restored) = Db::restore_logical(&dir.0.join("e.db"), &key(1), &bytes[..]).unwrap();
        assert_eq!(restored, summary);
        assert_eq!(
            summary.count("categories"),
            12,
            "the seeded defaults travel too"
        );
        assert_eq!(db.schema_version(), Ok(Db::SCHEMA_VERSION));
    }

    #[test]
    fn every_table_is_covered_by_the_backup() {
        let db = Db::open_in_memory(&key(1)).unwrap();
        let tables: Vec<String> = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut covered: Vec<&str> = TABLE_ORDER.to_vec();
        covered.sort_unstable();
        assert_eq!(
            tables, covered,
            "a migration added a table: add it to TABLE_ORDER"
        );
    }

    fn restore(bytes: &[u8]) -> (Result<(Db, LogicalSummary), StorageError>, PathBuf, TempDir) {
        let dir = TempDir::new();
        let path = dir.0.join("r.db");
        let result = Db::restore_logical(&path, &key(3), bytes);
        (result, path, dir)
    }

    fn lines(bytes: &[u8]) -> Vec<String> {
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn join(lines: &[String]) -> Vec<u8> {
        let mut text = lines.join("\n");
        text.push('\n');
        text.into_bytes()
    }

    #[test]
    fn invalid_backups_are_refused_and_leave_no_file() {
        let (good, _) = export_bytes(&populated());
        let all = lines(&good);
        let tx_header = all
            .iter()
            .position(|l| l.contains("\"table\":\"transactions\""))
            .unwrap();

        let mut cases: Vec<(&str, Vec<u8>)> = vec![
            ("not json", b"hello\n".to_vec()),
            ("empty", Vec::new()),
            (
                "wrong format",
                join(&[r#"{"format":"x","version":1,"schema":2}"#.into()]),
            ),
        ];
        // Truncated: footer missing.
        cases.push(("no footer", join(&all[..all.len() - 1])));
        // Extra content after the footer.
        let mut extra = all.clone();
        extra.push("[1]".into());
        cases.push(("trailing", join(&extra)));
        // A row with the wrong arity.
        let mut arity = all.clone();
        arity[tx_header + 1] = "[1,2]".into();
        cases.push(("arity", join(&arity)));
        // A transaction pointing at an account that does not exist.
        let mut orphan = all.clone();
        orphan[tx_header + 1] = orphan[tx_header + 1].replacen("[1,5,", "[1,999,", 1);
        cases.push(("orphan", join(&orphan)));
        // Footer balances that disagree with the data.
        let mut balance = all.clone();
        let last = balance.len() - 1;
        balance[last] = balance[last].replacen("\"2425000\"", "\"1\"", 1);
        cases.push(("balance", join(&balance)));
        // Column list altered (SQL-looking name).
        let mut column = all.clone();
        column[tx_header] = column[tx_header].replacen("\"note\"", "\"note) VALUES(1);--\"", 1);
        cases.push(("column", join(&column)));
        // Declared row count larger than the rows present.
        let mut count = all.clone();
        count[tx_header] = count[tx_header].replacen("\"rows\":5", "\"rows\":6", 1);
        cases.push(("count", join(&count)));
        // A float where an integer is required.
        let mut float = all.clone();
        float[tx_header + 1] = float[tx_header + 1].replacen("-25000", "-25000.5", 1);
        cases.push(("float", join(&float)));
        // Schema newer than this build.
        cases.push((
            "too new",
            join(&[r#"{"format":"money-tracker-logical","version":1,"schema":99}"#.into()]),
        ));

        for (name, bytes) in cases {
            let (result, path, _dir) = restore(&bytes);
            assert!(result.is_err(), "{name} was accepted");
            assert!(!path.exists(), "{name} left a file behind");
            for suffix in ["-wal", "-shm"] {
                let mut companion = path.clone().into_os_string();
                companion.push(suffix);
                assert!(!Path::new(&companion).exists(), "{name} left {suffix}");
            }
        }
    }

    #[test]
    fn a_newer_schema_is_reported_as_such() {
        let bytes = join(&[r#"{"format":"money-tracker-logical","version":1,"schema":99}"#.into()]);
        let (result, _, _dir) = restore(&bytes);
        assert_eq!(
            result.err(),
            Some(StorageError::SchemaTooNew {
                found: 99,
                supported: Db::SCHEMA_VERSION
            })
        );
    }

    #[test]
    fn restore_refuses_to_overwrite_an_existing_file() {
        let dir = TempDir::new();
        let path = dir.0.join("exists.db");
        fs::write(&path, b"keep me").unwrap();
        let (bytes, _) = export_bytes(&populated());
        assert_eq!(
            Db::restore_logical(&path, &key(1), &bytes[..]).err(),
            Some(StorageError::Io)
        );
        assert_eq!(fs::read(&path).unwrap(), b"keep me");
    }

    #[test]
    fn an_old_schema_backup_is_loaded_then_migrated() {
        // A v1 database has no default categories and no unique-name index.
        let source = {
            let conn = Connection::open_in_memory().unwrap();
            Db::init_to(conn, &key(1), 1).unwrap()
        };
        source
            .conn()
            .execute_batch(
                "INSERT INTO accounts(id,name,kind,currency,created_at) VALUES(1,'A','cash','IDR',0);
                 INSERT INTO categories(id,name,kind) VALUES(1,'Food','expense');",
            )
            .unwrap();
        let (bytes, summary) = export_bytes(&source);
        assert_eq!(summary.schema_version, 1);
        let dir = TempDir::new();
        let (db, restored) =
            Db::restore_logical(&dir.0.join("v1.db"), &key(1), &bytes[..]).unwrap();
        assert_eq!(db.schema_version(), Ok(Db::SCHEMA_VERSION));
        assert_eq!(restored.schema_version, Db::SCHEMA_VERSION);
        assert_eq!(restored.count("accounts"), 1);
        // Migration 0002 seeds default categories; they sit beside the restored one.
        let count: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert!(count > 1);
        let _ = drain(&b""[..]);
    }
}
