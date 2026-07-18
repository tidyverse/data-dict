use std::fs::File;
use std::path::Path;

use hashbrown::HashSet;
use parquet::file::reader::{FileReader, SerializedFileReader};
use rayon::prelude::*;

use crate::ParquetError;
use crate::column_scan::{
    ByteKeys, ColumnBatch, PlannedColumn, normalize_bytes, plan_column, read_batch,
};

#[derive(Clone)]
pub struct UniquenessCheck {
    pub columns: Vec<String>,
}

#[derive(Default)]
pub struct UniquenessStats {
    pub duplicate_count: usize,
    pub duplicate_rows: Vec<usize>,
}

/// Validate uniqueness exactly by hashing each row's key in a streaming,
/// column-projected pass over the file.
///
/// Memory is proportional to the number of distinct keys: a duplicate is a key
/// already seen. A single integer-like column is hashed as `i64` without
/// allocating; a single byte column is hashed by its value bytes directly
/// (a zero-copy slice into the decoded page); only a composite key pays to
/// build a length-framed key so its columns never collide across splits. Checks
/// are independent and run in parallel, each reading only the columns it needs.
pub fn uniqueness_stats(
    path: &Path,
    checks: &[UniquenessCheck],
    sample_limit: usize,
) -> Result<Vec<UniquenessStats>, ParquetError> {
    checks
        .par_iter()
        .map(|check| check_uniqueness(path, check, sample_limit))
        .collect()
}

fn check_uniqueness(
    path: &Path,
    check: &UniquenessCheck,
    sample_limit: usize,
) -> Result<UniquenessStats, ParquetError> {
    let file =
        File::open(path).map_err(|e| ParquetError::General(format!("Cannot open file: {e}")))?;
    let reader = SerializedFileReader::new(file)?;
    let descr = reader.metadata().file_metadata().schema_descr_ptr();

    let columns = plan_columns(check, &descr)?;
    let rows = reader.metadata().file_metadata().num_rows().max(0) as usize;
    let mut dedup = Dedup::new(check, &columns, rows);
    let mut stat = UniquenessStats::default();

    let mut row_offset = 0usize;
    let mut batches: Vec<ColumnBatch> = Vec::with_capacity(columns.len());
    for group in 0..reader.num_row_groups() {
        let row_group = reader.get_row_group(group)?;
        let mut readers = columns
            .iter()
            .map(|column| row_group.get_column_reader(column.leaf))
            .collect::<Result<Vec<_>, _>>()?;
        loop {
            batches.clear();
            let mut rows = 0;
            for (reader, column) in readers.iter_mut().zip(&columns) {
                let batch = read_batch(reader, column)?;
                rows = batch.len();
                batches.push(batch);
            }
            if rows == 0 {
                break;
            }
            dedup.scan(&columns, &batches, row_offset, sample_limit, &mut stat);
            row_offset += rows;
        }
    }
    Ok(stat)
}

/// Resolve every column named by the check to its leaf position, de-duplicated
/// so a column repeated within a key is read only once.
fn plan_columns(
    check: &UniquenessCheck,
    descr: &parquet::schema::types::SchemaDescPtr,
) -> Result<Vec<PlannedColumn>, ParquetError> {
    let mut columns: Vec<PlannedColumn> = Vec::new();
    for name in &check.columns {
        if columns.iter().any(|c| &c.name == name) {
            continue;
        }
        columns.push(plan_column(descr, name)?);
    }
    Ok(columns)
}

/// Per-check duplicate detector, with a fast path for each single-column shape:
/// a scalar column hashes bare `i64`s, a single byte column hashes the value
/// bytes directly, and only a composite key pays for length-framing (so its
/// columns never collide across different splits).
///
/// Nulls never count as duplicates: a row with a null in any key column is
/// skipped, matching SQL uniqueness (multiple nulls are allowed) and avoiding a
/// spurious D02 alongside the D01 that a null in a `required`/`primary_key`
/// column already draws.
enum Dedup {
    Scalar {
        column: usize,
        seen: HashSet<i64>,
    },
    SingleBytes {
        column: usize,
        seen: ByteKeys,
    },
    Bytes {
        columns: Vec<usize>,
        seen: ByteKeys,
        key: Vec<u8>,
    },
}

impl Dedup {
    fn new(check: &UniquenessCheck, columns: &[PlannedColumn], rows: usize) -> Self {
        let positions = check
            .columns
            .iter()
            .map(|name| {
                columns
                    .iter()
                    .position(|column| &column.name == name)
                    .expect("planned column is missing")
            })
            .collect::<Vec<_>>();
        // A uniqueness check expects mostly-distinct keys, so size for one entry
        // per row up front. This both skips the incremental rehashes and, more
        // importantly, avoids the transient 2x spike of growing a near-full
        // table by doubling.
        if let [only] = positions.as_slice() {
            if columns[*only].is_scalar() {
                return Dedup::Scalar {
                    column: *only,
                    seen: HashSet::with_capacity(rows),
                };
            }
            return Dedup::SingleBytes {
                column: *only,
                seen: ByteKeys::with_capacity(rows),
            };
        }
        Dedup::Bytes {
            columns: positions,
            seen: ByteKeys::with_capacity(rows),
            key: Vec::new(),
        }
    }

    fn scan(
        &mut self,
        planned: &[PlannedColumn],
        batches: &[ColumnBatch],
        row_offset: usize,
        sample_limit: usize,
        stat: &mut UniquenessStats,
    ) {
        match self {
            Dedup::Scalar { column, seen } => {
                let batch = &batches[*column];
                for row in 0..batch.len() {
                    if batch.is_null(row) {
                        continue;
                    }
                    if !seen.insert(batch.scalar(row)) {
                        record(stat, row_offset + row + 1, sample_limit);
                    }
                }
            }
            Dedup::SingleBytes { column, seen } => {
                let batch = &batches[*column];
                let normalize = planned[*column].normalize;
                for row in 0..batch.len() {
                    if batch.is_null(row) {
                        continue;
                    }
                    if !seen.insert(normalize_bytes(batch.bytes(row), normalize)) {
                        record(stat, row_offset + row + 1, sample_limit);
                    }
                }
            }
            Dedup::Bytes { columns, seen, key } => {
                let rows = batches[columns[0]].len();
                'row: for row in 0..rows {
                    key.clear();
                    for &column in columns.iter() {
                        let batch = &batches[column];
                        if batch.is_null(row) {
                            continue 'row;
                        } else if let ColumnBatch::Scalar { .. } = batch {
                            key.push(1);
                            key.extend_from_slice(&batch.scalar(row).to_le_bytes());
                        } else {
                            let value =
                                normalize_bytes(batch.bytes(row), planned[column].normalize);
                            key.push(2);
                            key.extend_from_slice(&(value.len() as u32).to_le_bytes());
                            key.extend_from_slice(value);
                        }
                    }
                    if !seen.insert(key) {
                        record(stat, row_offset + row + 1, sample_limit);
                    }
                }
            }
        }
    }
}

fn record(stat: &mut UniquenessStats, row: usize, sample_limit: usize) {
    stat.duplicate_count += 1;
    if stat.duplicate_rows.len() < sample_limit {
        stat.duplicate_rows.push(row);
    }
}
