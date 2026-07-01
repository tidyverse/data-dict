use std::fs::File;
use std::hash::BuildHasher;
use std::path::Path;

use hashbrown::{DefaultHashBuilder, HashSet, HashTable};
use parquet::basic::Type as PhysicalType;
use parquet::column::reader::ColumnReader;
use parquet::data_type::ByteArray;
use parquet::file::reader::{FileReader, SerializedFileReader};
use rayon::prelude::*;

use crate::ParquetError;

/// Rows decoded per `read_records` call. Large enough to amortise per-call
/// overhead, small enough that a batch of every scanned column stays in cache.
const BATCH_ROWS: usize = 8192;

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
            dedup.scan(&batches, row_offset, sample_limit, &mut stat);
            row_offset += rows;
        }
    }
    Ok(stat)
}

/// A column scanned for one or more checks, identified by its leaf index.
struct PlannedColumn {
    name: String,
    leaf: usize,
    physical: PhysicalType,
    max_def: i16,
}

impl PlannedColumn {
    /// Whether the column's values are hashed as a single `i64` scalar (as
    /// opposed to a variable-length byte string).
    fn is_scalar(&self) -> bool {
        !matches!(
            self.physical,
            PhysicalType::BYTE_ARRAY | PhysicalType::FIXED_LEN_BYTE_ARRAY | PhysicalType::INT96
        )
    }
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
        let leaf = (0..descr.num_columns())
            .find(|&i| descr.column(i).name() == name)
            .ok_or_else(|| ParquetError::General(format!("Column not found: {name}")))?;
        let column = descr.column(leaf);
        columns.push(PlannedColumn {
            name: name.clone(),
            leaf,
            physical: column.physical_type(),
            max_def: column.max_def_level(),
        });
    }
    Ok(columns)
}

/// One column's values for a batch of rows, decoded row-aligned (nulls included)
/// into either scalar `i64`s or byte strings. Byte values keep the [`ByteArray`]
/// handles the reader produced, so [`ColumnBatch::bytes`] is a zero-copy slice
/// into Parquet's decoded page (or dictionary) buffer rather than a re-copy.
///
/// `null` is empty when the column is `REQUIRED` (it can't contain nulls), which
/// skips a per-batch mask allocation on the common path.
enum ColumnBatch {
    Scalar {
        values: Vec<i64>,
        null: Vec<bool>,
    },
    Bytes {
        values: Vec<ByteArray>,
        null: Vec<bool>,
    },
}

impl ColumnBatch {
    fn len(&self) -> usize {
        match self {
            ColumnBatch::Scalar { values, .. } => values.len(),
            ColumnBatch::Bytes { values, .. } => values.len(),
        }
    }

    fn is_null(&self, row: usize) -> bool {
        match self {
            ColumnBatch::Scalar { null, .. } | ColumnBatch::Bytes { null, .. } => {
                !null.is_empty() && null[row]
            }
        }
    }

    fn scalar(&self, row: usize) -> i64 {
        match self {
            ColumnBatch::Scalar { values, .. } => values[row],
            ColumnBatch::Bytes { .. } => unreachable!("scalar read of a byte column"),
        }
    }

    fn bytes(&self, row: usize) -> &[u8] {
        match self {
            ColumnBatch::Bytes { values, .. } => values[row].data(),
            ColumnBatch::Scalar { .. } => unreachable!("byte read of a scalar column"),
        }
    }
}

/// Read up to [`BATCH_ROWS`] rows from one column, expanding nulls back into
/// their row positions so every column in a batch shares the same row indices.
fn read_batch(
    reader: &mut ColumnReader,
    column: &PlannedColumn,
) -> Result<ColumnBatch, ParquetError> {
    let max_def = column.max_def;
    macro_rules! scalar {
        ($variant:path, $map:expr) => {{
            let $variant(ref mut typed) = *reader else {
                return Err(physical_mismatch(column));
            };
            let mut def = Vec::new();
            let mut raw = Vec::new();
            let (records, _, _) = typed.read_records(BATCH_ROWS, Some(&mut def), None, &mut raw)?;
            let mut values = vec![0i64; records];
            let mut null = Vec::new();
            if max_def == 0 {
                for (out, value) in values.iter_mut().zip(&raw) {
                    *out = $map(value);
                }
            } else {
                null = vec![false; records];
                let mut cursor = 0;
                for row in 0..records {
                    if def[row] == max_def {
                        values[row] = $map(&raw[cursor]);
                        cursor += 1;
                    } else {
                        null[row] = true;
                    }
                }
            }
            Ok(ColumnBatch::Scalar { values, null })
        }};
    }
    // The rare `FixedLenByteArray`/`Int96` cases must convert each value into an
    // owned `ByteArray`; `BYTE_ARRAY` (below) skips even that, handing the decoded
    // vector straight through.
    macro_rules! bytes_converted {
        ($variant:path, $conv:expr) => {{
            let $variant(ref mut typed) = *reader else {
                return Err(physical_mismatch(column));
            };
            let mut def = Vec::new();
            let mut raw = Vec::new();
            let (records, _, _) = typed.read_records(BATCH_ROWS, Some(&mut def), None, &mut raw)?;
            let values = raw.into_iter().map($conv).collect();
            Ok(expand_bytes(values, &def, max_def, records))
        }};
    }

    match column.physical {
        PhysicalType::BOOLEAN => scalar!(ColumnReader::BoolColumnReader, |v: &bool| *v as i64),
        PhysicalType::INT32 => scalar!(ColumnReader::Int32ColumnReader, |v: &i32| *v as i64),
        PhysicalType::INT64 => scalar!(ColumnReader::Int64ColumnReader, |v: &i64| *v),
        PhysicalType::FLOAT => scalar!(ColumnReader::FloatColumnReader, float_bits),
        PhysicalType::DOUBLE => scalar!(ColumnReader::DoubleColumnReader, double_bits),
        PhysicalType::BYTE_ARRAY => {
            let ColumnReader::ByteArrayColumnReader(ref mut typed) = *reader else {
                return Err(physical_mismatch(column));
            };
            let mut def = Vec::new();
            let mut raw = Vec::new();
            let (records, _, _) = typed.read_records(BATCH_ROWS, Some(&mut def), None, &mut raw)?;
            Ok(expand_bytes(raw, &def, max_def, records))
        }
        PhysicalType::FIXED_LEN_BYTE_ARRAY => {
            bytes_converted!(ColumnReader::FixedLenByteArrayColumnReader, fixed_len_owned)
        }
        PhysicalType::INT96 => bytes_converted!(ColumnReader::Int96ColumnReader, int96_owned),
    }
}

/// Place non-null byte values back at their row positions, moving (never cloning)
/// each value. With no nulls the decoded vector is already row-aligned, so it is
/// used as-is.
fn expand_bytes(nonnull: Vec<ByteArray>, def: &[i16], max_def: i16, records: usize) -> ColumnBatch {
    if max_def == 0 {
        return ColumnBatch::Bytes {
            values: nonnull,
            null: Vec::new(),
        };
    }
    let mut values = Vec::with_capacity(records);
    let mut null = vec![false; records];
    let mut nonnull = nonnull.into_iter();
    for row in 0..records {
        if def[row] == max_def {
            values.push(nonnull.next().expect("a value for each defined row"));
        } else {
            null[row] = true;
            values.push(ByteArray::default());
        }
    }
    ColumnBatch::Bytes { values, null }
}

fn float_bits(value: &f32) -> i64 {
    (if *value == 0.0 { 0 } else { value.to_bits() }) as i64
}

fn double_bits(value: &f64) -> i64 {
    (if *value == 0.0 { 0 } else { value.to_bits() }) as i64
}

fn fixed_len_owned(value: parquet::data_type::FixedLenByteArray) -> ByteArray {
    ByteArray::from(value)
}

fn int96_owned(value: parquet::data_type::Int96) -> ByteArray {
    // Identify the value by the raw bytes of its three words. Byte order only
    // has to be consistent within a run, which it is.
    let bytes: Vec<u8> = value.data().iter().flat_map(|w| w.to_le_bytes()).collect();
    ByteArray::from(bytes)
}

fn physical_mismatch(column: &PlannedColumn) -> ParquetError {
    ParquetError::General(format!(
        "Column reader type does not match physical type {:?}",
        column.physical
    ))
}

/// Per-check duplicate detector, with a fast path for each single-column shape:
/// a scalar column hashes bare `i64`s, a single byte column hashes the value
/// bytes directly, and only a composite key pays for length-framing (so its
/// columns never collide across different splits).
enum Dedup {
    Scalar {
        column: usize,
        seen: HashSet<i64>,
        null_seen: bool,
    },
    SingleBytes {
        column: usize,
        seen: ByteKeys,
        null_seen: bool,
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
                    null_seen: false,
                };
            }
            return Dedup::SingleBytes {
                column: *only,
                seen: ByteKeys::with_capacity(rows),
                null_seen: false,
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
        batches: &[ColumnBatch],
        row_offset: usize,
        sample_limit: usize,
        stat: &mut UniquenessStats,
    ) {
        match self {
            Dedup::Scalar {
                column,
                seen,
                null_seen,
            } => {
                let batch = &batches[*column];
                for row in 0..batch.len() {
                    let duplicate = if batch.is_null(row) {
                        std::mem::replace(null_seen, true)
                    } else {
                        !seen.insert(batch.scalar(row))
                    };
                    if duplicate {
                        record(stat, row_offset + row + 1, sample_limit);
                    }
                }
            }
            Dedup::SingleBytes {
                column,
                seen,
                null_seen,
            } => {
                let batch = &batches[*column];
                for row in 0..batch.len() {
                    let duplicate = if batch.is_null(row) {
                        std::mem::replace(null_seen, true)
                    } else {
                        !seen.insert(batch.bytes(row))
                    };
                    if duplicate {
                        record(stat, row_offset + row + 1, sample_limit);
                    }
                }
            }
            Dedup::Bytes { columns, seen, key } => {
                let rows = batches[columns[0]].len();
                for row in 0..rows {
                    key.clear();
                    for &column in columns.iter() {
                        let batch = &batches[column];
                        if batch.is_null(row) {
                            key.push(0);
                        } else if let ColumnBatch::Scalar { .. } = batch {
                            key.push(1);
                            key.extend_from_slice(&batch.scalar(row).to_le_bytes());
                        } else {
                            let value = batch.bytes(row);
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

/// A set of byte-string keys packed into one arena so distinct keys cost an
/// amortised append, not an allocation each. Entries reference the arena by
/// `(offset, length)`, and a single hash probe both tests membership and, when
/// absent, positions the insertion.
#[derive(Default)]
struct ByteKeys {
    arena: Arena,
    table: HashTable<KeyRef>,
    hasher: DefaultHashBuilder,
}

/// A key's location in the arena: `(chunk, offset within chunk, length)`.
type KeyRef = (u32, u32, u32);

impl ByteKeys {
    fn with_capacity(rows: usize) -> Self {
        ByteKeys {
            arena: Arena::default(),
            table: HashTable::with_capacity(rows),
            hasher: DefaultHashBuilder::default(),
        }
    }

    /// Insert `key`, returning `true` if it was new (i.e. not a duplicate).
    fn insert(&mut self, key: &[u8]) -> bool {
        let Self {
            arena,
            table,
            hasher,
        } = self;
        let hash = hasher.hash_one(key);
        if table.find(hash, |&r| arena.get(r) == key).is_some() {
            return false;
        }
        let entry = arena.push(key);
        table.insert_unique(hash, entry, |&r| hasher.hash_one(arena.get(r)));
        true
    }
}

/// Append-only byte store backed by fixed-size chunks. Existing chunks are never
/// reallocated, so growth is incremental — there is no transient doubling spike
/// as with one growing `Vec`, and no need to estimate the total size up front.
#[derive(Default)]
struct Arena {
    chunks: Vec<Vec<u8>>,
}

impl Arena {
    const CHUNK: usize = 4 * 1024 * 1024;

    fn push(&mut self, key: &[u8]) -> KeyRef {
        let fits = self
            .chunks
            .last()
            .is_some_and(|chunk| chunk.capacity() - chunk.len() >= key.len());
        if !fits {
            self.chunks
                .push(Vec::with_capacity(key.len().max(Self::CHUNK)));
        }
        let chunk = self.chunks.len() as u32 - 1;
        let buffer = self.chunks.last_mut().unwrap();
        let offset = buffer.len() as u32;
        buffer.extend_from_slice(key);
        (chunk, offset, key.len() as u32)
    }

    fn get(&self, (chunk, offset, len): KeyRef) -> &[u8] {
        &self.chunks[chunk as usize][offset as usize..offset as usize + len as usize]
    }
}

fn record(stat: &mut UniquenessStats, row: usize, sample_limit: usize) {
    stat.duplicate_count += 1;
    if stat.duplicate_rows.len() < sample_limit {
        stat.duplicate_rows.push(row);
    }
}
