//! Categories: the default set comes from migration 0002; users can add more.

use mt_core::{CategoryDraft, FlowKind};
use rusqlite::{OptionalExtension, params};

use crate::{db::Db, error::StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub kind: FlowKind,
    pub parent_id: Option<i64>,
    pub archived: bool,
}

impl Db {
    /// Income first, then expense; parents before their children.
    pub fn categories(&self, include_archived: bool) -> Result<Vec<Category>, StorageError> {
        let mut statement = self.conn().prepare(
            "SELECT id, name, kind, parent_id, archived FROM categories
             WHERE (?1 = 1 OR archived = 0)
             ORDER BY kind DESC, COALESCE(parent_id, id), parent_id IS NOT NULL, name, id",
        )?;
        let rows = statement.query_map(params![i64::from(include_archived)], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, kind, parent_id, archived) = row?;
            Ok(Category {
                id,
                name,
                kind: FlowKind::from_code(&kind).ok_or(StorageError::InvalidData)?,
                parent_id,
                archived: archived != 0,
            })
        })
        .collect()
    }

    /// Adds a category (or a sub-category of a top-level one of the same kind).
    pub fn create_category(&mut self, draft: &CategoryDraft) -> Result<i64, StorageError> {
        let tx = self.conn_mut().transaction()?;
        if let Some(parent_id) = draft.parent_id() {
            let parent: Option<(String, Option<i64>, i64)> = tx
                .query_row(
                    "SELECT kind, parent_id, archived FROM categories WHERE id = ?1",
                    params![parent_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let (kind, grandparent, archived) = parent.ok_or(StorageError::NotFound)?;
            if kind != draft.kind().code() {
                return Err(StorageError::KindMismatch);
            }
            if grandparent.is_some() {
                // Only one level of nesting.
                return Err(StorageError::Constraint);
            }
            if archived != 0 {
                return Err(StorageError::Archived);
            }
        }
        tx.execute(
            "INSERT INTO categories(name, kind, parent_id) VALUES (?1, ?2, ?3)",
            params![draft.name(), draft.kind().code(), draft.parent_id()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use zeroize::Zeroizing;

    use super::*;

    fn db() -> Db {
        Db::open_in_memory(&Zeroizing::new([6u8; 32])).unwrap()
    }

    fn draft(name: &str, kind: FlowKind, parent: Option<i64>) -> CategoryDraft {
        CategoryDraft::new(name, kind, parent).unwrap()
    }

    #[test]
    fn the_default_set_is_seeded_once() {
        let db = db();
        let all = db.categories(false).unwrap();
        assert_eq!(
            all.iter().filter(|c| c.kind == FlowKind::Expense).count(),
            8
        );
        assert_eq!(all.iter().filter(|c| c.kind == FlowKind::Income).count(), 4);
        assert!(all.iter().any(|c| c.name == "Makanan"));
        // "Lainnya" exists for both kinds: uniqueness is per kind.
        assert_eq!(all.iter().filter(|c| c.name == "Lainnya").count(), 2);
    }

    #[test]
    fn income_comes_before_expense() {
        let db = db();
        let kinds: Vec<FlowKind> = db
            .categories(false)
            .unwrap()
            .iter()
            .map(|c| c.kind)
            .collect();
        let first_expense = kinds.iter().position(|k| *k == FlowKind::Expense).unwrap();
        assert!(
            kinds[..first_expense]
                .iter()
                .all(|k| *k == FlowKind::Income)
        );
    }

    #[test]
    fn a_custom_category_can_be_added_once_per_kind_and_name() {
        let mut db = db();
        let id = db
            .create_category(&draft("Hobi", FlowKind::Expense, None))
            .unwrap();
        assert!(id > 0);
        assert_eq!(
            db.create_category(&draft("hobi", FlowKind::Expense, None)),
            Err(StorageError::Duplicate)
        );
        // Same name under the other kind is fine.
        assert!(
            db.create_category(&draft("Hobi", FlowKind::Income, None))
                .is_ok()
        );
    }

    #[test]
    fn sub_categories_follow_the_nesting_rules() {
        let mut db = db();
        let food = db
            .categories(false)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Makanan")
            .unwrap()
            .id;
        let sub = db
            .create_category(&draft("Kopi", FlowKind::Expense, Some(food)))
            .unwrap();
        // Two levels at most.
        assert_eq!(
            db.create_category(&draft("Latte", FlowKind::Expense, Some(sub))),
            Err(StorageError::Constraint)
        );
        // The parent's kind must match.
        assert_eq!(
            db.create_category(&draft("Kopi", FlowKind::Income, Some(food))),
            Err(StorageError::KindMismatch)
        );
        assert_eq!(
            db.create_category(&draft("X", FlowKind::Expense, Some(9999))),
            Err(StorageError::NotFound)
        );
        let listed: Vec<String> = db
            .categories(false)
            .unwrap()
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let at = listed.iter().position(|n| n == "Makanan").unwrap();
        assert_eq!(
            listed[at + 1],
            "Kopi",
            "children follow their parent: {listed:?}"
        );
    }
}
