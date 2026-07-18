//! Foreign-key referential integrity (D05/D06): every non-null value in a
//! child column must appear in the parent's primary-key column.
//!
//! Each check builds a set of the parent column's values in one streaming pass,
//! then streams the child column probing membership. Nulls are exempt on both
//! sides (a null foreign key references nothing). Values are compared by the same
//! normalized form as the uniqueness check (D02), so both columns must use a
//! [comparable type](../metadata/fn.uniqueness_comparability.html); if either
//! can't be compared the check is skipped and reported as D06.

use std::fs::File;
use std::path::{Path, PathBuf};

use hashbrown::HashSet;
use parquet::file::reader::{FileReader, SerializedFileReader};
use rayon::prelude::*;

use crate::ParquetError;
use crate::column_scan::{
    ByteKeys, ColumnBatch, PlannedColumn, column_barrier, display_value, normalize_bytes,
    plan_column, scan_column,
};

/// One foreign-key check: the child column's values must all appear in the
/// parent column. The two may live in the same file (a self-join) or different
/// files.
#[derive(Clone)]
pub struct ForeignKeyCheck {
    pub child_path: PathBuf,
    pub child_column: String,
    pub parent_path: PathBuf,
    pub parent_column: String,
}

/// The outcome of a [`ForeignKeyCheck`].
pub enum ForeignKeyResult {
    /// The child or parent column uses a type whose values can't be compared, so
    /// the reference was not checked (D06). `reason` is a barrier slug (e.g.
    /// `json`).
    NotVerified { reason: &'static str },
    /// The reference was checked; `orphan_count == 0` means it holds.
    Checked(ForeignKeyStats),
}

/// Values found in the child column that are absent from the parent column.
/// `orphan_rows`/`orphan_values` sample the first few (1-based rows, distinct
/// values); `orphan_count` is the total.
#[derive(Default)]
pub struct ForeignKeyStats {
    pub orphan_count: usize,
    pub orphan_rows: Vec<usize>,
    pub orphan_values: Vec<String>,
}

/// Run every foreign-key check in parallel, each reading only its two columns.
pub fn foreign_key_stats(
    checks: &[ForeignKeyCheck],
    sample_limit: usize,
) -> Result<Vec<ForeignKeyResult>, ParquetError> {
    checks
        .par_iter()
        .map(|check| check_foreign_key(check, sample_limit))
        .collect()
}

fn open(path: &Path) -> Result<SerializedFileReader<File>, ParquetError> {
    let file =
        File::open(path).map_err(|e| ParquetError::General(format!("Cannot open file: {e}")))?;
    SerializedFileReader::new(file)
}

fn check_foreign_key(
    check: &ForeignKeyCheck,
    sample_limit: usize,
) -> Result<ForeignKeyResult, ParquetError> {
    let parent_reader = open(&check.parent_path)?;
    let parent_descr = parent_reader.metadata().file_metadata().schema_descr_ptr();
    let parent_col = plan_column(&parent_descr, &check.parent_column)?;
    if let Some(reason) = column_barrier(&parent_descr, parent_col.leaf) {
        return Ok(ForeignKeyResult::NotVerified { reason });
    }

    let child_reader = open(&check.child_path)?;
    let child_descr = child_reader.metadata().file_metadata().schema_descr_ptr();
    let child_col = plan_column(&child_descr, &check.child_column)?;
    if let Some(reason) = column_barrier(&child_descr, child_col.leaf) {
        return Ok(ForeignKeyResult::NotVerified { reason });
    }

    let parent_rows = parent_reader.metadata().file_metadata().num_rows().max(0) as usize;
    let mut seen = KeySet::with_capacity(&parent_col, parent_rows);
    scan_column(&parent_reader, &parent_col, |batch, row, _| {
        seen.insert(batch, row, &parent_col);
    })?;

    let mut stats = ForeignKeyStats::default();
    let mut distinct: HashSet<String> = HashSet::new();
    scan_column(&child_reader, &child_col, |batch, row, absolute| {
        if seen.contains(batch, row, &child_col) {
            return;
        }
        stats.orphan_count += 1;
        if stats.orphan_rows.len() < sample_limit {
            stats.orphan_rows.push(absolute + 1);
        }
        if stats.orphan_values.len() < sample_limit {
            let value = display_value(batch, row, &child_col);
            if distinct.insert(value.clone()) {
                stats.orphan_values.push(value);
            }
        }
    })?;
    Ok(ForeignKeyResult::Checked(stats))
}

/// The parent column's values, with the same fast paths as the uniqueness check:
/// a scalar column hashes bare `i64`s, a byte column hashes its (normalized)
/// value bytes directly. A child value of the other shape is a type mismatch and
/// can't be found, so it's treated as absent.
enum KeySet {
    Scalar(HashSet<i64>),
    Bytes(ByteKeys),
}

impl KeySet {
    fn with_capacity(column: &PlannedColumn, rows: usize) -> Self {
        if column.is_scalar() {
            KeySet::Scalar(HashSet::with_capacity(rows))
        } else {
            KeySet::Bytes(ByteKeys::with_capacity(rows))
        }
    }

    fn insert(&mut self, batch: &ColumnBatch, row: usize, column: &PlannedColumn) {
        match self {
            KeySet::Scalar(set) => {
                set.insert(batch.scalar(row));
            }
            KeySet::Bytes(set) => {
                set.insert(normalize_bytes(batch.bytes(row), column.normalize));
            }
        }
    }

    fn contains(&self, batch: &ColumnBatch, row: usize, column: &PlannedColumn) -> bool {
        match self {
            KeySet::Scalar(set) => {
                matches!(batch, ColumnBatch::Scalar { .. }) && set.contains(&batch.scalar(row))
            }
            KeySet::Bytes(set) => {
                matches!(batch, ColumnBatch::Bytes { .. })
                    && set.contains(normalize_bytes(batch.bytes(row), column.normalize))
            }
        }
    }
}
