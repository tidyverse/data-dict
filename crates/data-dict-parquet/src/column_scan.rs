//! Shared column-reading primitives for the value-scanning data checks.
//!
//! A streaming, column-projected batch reader that decodes one column's values
//! row-aligned (nulls included), plus the value canonicalization and the
//! arena-backed byte-key set the uniqueness (D02) and foreign-key (D05) checks
//! both build on. See the "comparable types" section of `site/validation.md`
//! for why values are normalized the way they are.

use std::fs::File;

use hashbrown::{DefaultHashBuilder, HashTable};
use parquet::basic::Type as PhysicalType;
use parquet::column::reader::ColumnReader;
use parquet::data_type::ByteArray;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::schema::types::SchemaDescPtr;
use std::hash::BuildHasher;

use crate::ParquetError;
use crate::metadata::{Comparability, Normalization, uniqueness_comparability};

/// Rows decoded per `read_records` call. Large enough to amortise per-call
/// overhead, small enough that a batch of every scanned column stays in cache.
pub(crate) const BATCH_ROWS: usize = 8192;

/// A column scanned for a check, identified by its leaf index.
pub(crate) struct PlannedColumn {
    pub(crate) name: String,
    pub(crate) leaf: usize,
    pub(crate) physical: PhysicalType,
    pub(crate) max_def: i16,
    pub(crate) normalize: Normalization,
}

impl PlannedColumn {
    /// Whether the column's values are hashed as a single `i64` scalar (as
    /// opposed to a variable-length byte string).
    pub(crate) fn is_scalar(&self) -> bool {
        !matches!(
            self.physical,
            PhysicalType::BYTE_ARRAY | PhysicalType::FIXED_LEN_BYTE_ARRAY | PhysicalType::INT96
        )
    }
}

/// Resolve a column name to its leaf position and reading plan.
pub(crate) fn plan_column(
    descr: &SchemaDescPtr,
    name: &str,
) -> Result<PlannedColumn, ParquetError> {
    let leaf = (0..descr.num_columns())
        .find(|&i| descr.column(i).name() == name)
        .ok_or_else(|| ParquetError::General(format!("Column not found: {name}")))?;
    let column = descr.column(leaf);
    let normalize = match uniqueness_comparability(column.self_type()) {
        Comparability::Comparable(normalize) => normalize,
        Comparability::Incomparable(_) => Normalization::None,
    };
    Ok(PlannedColumn {
        name: name.to_string(),
        leaf,
        physical: column.physical_type(),
        max_def: column.max_def_level(),
        normalize,
    })
}

/// The barrier reason if the column's type can't be compared (see
/// [`uniqueness_comparability`]), else `None`.
pub(crate) fn column_barrier(descr: &SchemaDescPtr, leaf: usize) -> Option<&'static str> {
    match uniqueness_comparability(descr.column(leaf).self_type()) {
        Comparability::Incomparable(reason) => Some(reason),
        Comparability::Comparable(_) => None,
    }
}

/// One column's values for a batch of rows, decoded row-aligned (nulls included)
/// into either scalar `i64`s or byte strings. Byte values keep the [`ByteArray`]
/// handles the reader produced, so [`ColumnBatch::bytes`] is a zero-copy slice
/// into Parquet's decoded page (or dictionary) buffer rather than a re-copy.
///
/// `null` is empty when the column is `REQUIRED` (it can't contain nulls), which
/// skips a per-batch mask allocation on the common path.
pub(crate) enum ColumnBatch {
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
    pub(crate) fn len(&self) -> usize {
        match self {
            ColumnBatch::Scalar { values, .. } => values.len(),
            ColumnBatch::Bytes { values, .. } => values.len(),
        }
    }

    pub(crate) fn is_null(&self, row: usize) -> bool {
        match self {
            ColumnBatch::Scalar { null, .. } | ColumnBatch::Bytes { null, .. } => {
                !null.is_empty() && null[row]
            }
        }
    }

    pub(crate) fn scalar(&self, row: usize) -> i64 {
        match self {
            ColumnBatch::Scalar { values, .. } => values[row],
            ColumnBatch::Bytes { .. } => unreachable!("scalar read of a byte column"),
        }
    }

    pub(crate) fn bytes(&self, row: usize) -> &[u8] {
        match self {
            ColumnBatch::Bytes { values, .. } => values[row].data(),
            ColumnBatch::Scalar { .. } => unreachable!("byte read of a scalar column"),
        }
    }
}

/// Stream a single column across every row group, invoking `visit` once per
/// non-null value with `(batch, row within batch, 0-based row number)`.
pub(crate) fn scan_column<F>(
    reader: &SerializedFileReader<File>,
    column: &PlannedColumn,
    mut visit: F,
) -> Result<(), ParquetError>
where
    F: FnMut(&ColumnBatch, usize, usize),
{
    let mut row_offset = 0usize;
    for group in 0..reader.num_row_groups() {
        let row_group = reader.get_row_group(group)?;
        let mut column_reader = row_group.get_column_reader(column.leaf)?;
        loop {
            let batch = read_batch(&mut column_reader, column)?;
            let rows = batch.len();
            if rows == 0 {
                break;
            }
            for row in 0..rows {
                if !batch.is_null(row) {
                    visit(&batch, row, row_offset + row);
                }
            }
            row_offset += rows;
        }
    }
    Ok(())
}

/// Read up to [`BATCH_ROWS`] rows from one column, expanding nulls back into
/// their row positions so every column in a batch shares the same row indices.
pub(crate) fn read_batch(
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

/// Hash a float by value, not by bits: `-0.0`/`+0.0` collapse to one key and
/// every NaN bit pattern collapses to one key, so logically-equal floats compare
/// equal (see the "comparable types" section of `site/validation.md`).
fn float_bits(value: &f32) -> i64 {
    let bits = if *value == 0.0 {
        0
    } else if value.is_nan() {
        f32::NAN.to_bits()
    } else {
        value.to_bits()
    };
    bits as i64
}

fn double_bits(value: &f64) -> i64 {
    let bits = if *value == 0.0 {
        0
    } else if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    };
    bits as i64
}

/// Apply a byte column's [`Normalization`] before hashing. Only decimals need it
/// (trimming redundant leading sign bytes); everything else is returned as-is.
pub(crate) fn normalize_bytes(bytes: &[u8], normalize: Normalization) -> &[u8] {
    match normalize {
        Normalization::DecimalBytes => normalize_decimal(bytes),
        _ => bytes,
    }
}

/// Canonical minimal two's-complement form: drop redundant leading sign-extension
/// bytes so equal decimal values encoded at different byte lengths compare equal.
/// Keeps at least one byte.
fn normalize_decimal(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start + 1 < bytes.len() {
        let sign = bytes[start];
        let next_high = bytes[start + 1] & 0x80;
        // A leading byte is redundant only when the next byte carries the same
        // sign bit: 0x00 before a positive byte, or 0xFF before a negative one.
        let redundant = (sign == 0x00 && next_high == 0) || (sign == 0xFF && next_high == 0x80);
        if !redundant {
            break;
        }
        start += 1;
    }
    &bytes[start..]
}

/// A human-readable form of one value, for sampling in a diagnostic.
pub(crate) fn display_value(batch: &ColumnBatch, row: usize, column: &PlannedColumn) -> String {
    match batch {
        ColumnBatch::Scalar { .. } => {
            let bits = batch.scalar(row);
            match column.physical {
                PhysicalType::BOOLEAN => (bits != 0).to_string(),
                PhysicalType::FLOAT => f32::from_bits(bits as u32).to_string(),
                PhysicalType::DOUBLE => f64::from_bits(bits as u64).to_string(),
                _ => bits.to_string(),
            }
        }
        ColumnBatch::Bytes { .. } => {
            let bytes = batch.bytes(row);
            match std::str::from_utf8(bytes) {
                Ok(text) => text.to_string(),
                Err(_) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
            }
        }
    }
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

/// A set of byte-string keys packed into one arena so distinct keys cost an
/// amortised append, not an allocation each. Entries reference the arena by
/// `(chunk, offset, length)`, and a single hash probe both tests membership and,
/// when absent, positions the insertion.
#[derive(Default)]
pub(crate) struct ByteKeys {
    arena: Arena,
    table: HashTable<KeyRef>,
    hasher: DefaultHashBuilder,
}

/// A key's location in the arena: `(chunk, offset within chunk, length)`.
type KeyRef = (u32, u32, u32);

impl ByteKeys {
    pub(crate) fn with_capacity(rows: usize) -> Self {
        ByteKeys {
            arena: Arena::default(),
            table: HashTable::with_capacity(rows),
            hasher: DefaultHashBuilder::default(),
        }
    }

    /// Insert `key`, returning `true` if it was new (i.e. not already present).
    pub(crate) fn insert(&mut self, key: &[u8]) -> bool {
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

    /// Whether `key` is present, without inserting it.
    pub(crate) fn contains(&self, key: &[u8]) -> bool {
        let hash = self.hasher.hash_one(key);
        self.table
            .find(hash, |&r| self.arena.get(r) == key)
            .is_some()
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
