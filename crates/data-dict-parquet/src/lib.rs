//! Parquet reader for data-dict.yaml validation.

use parquet::basic::{LogicalType, TimeUnit, Type as PhysicalType};
use parquet::file::metadata::ParquetMetaData;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use parquet::schema::types::Type;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

/// Re-export of the underlying parquet error type, so crates that consume
/// [`column_types`] can name the error without depending on `parquet` directly.
pub use parquet::errors::ParquetError;

/// Type information for a single parquet column.
pub struct ColumnTypeInfo {
    pub name: String,
    pub dict_type: String,
    pub logical_type: Option<String>,
    pub physical_type: String,
}

/// Returns a list of `(column_name, data_dict_type)` pairs for all columns in a parquet file.
///
/// The data-dict types returned are: `"boolean"`, `"string"`, `"enum"`, `"date"`,
/// `"datetime"`, and `"number"`.
pub fn column_types(
    path: &Path,
) -> Result<Vec<(String, String)>, parquet::errors::ParquetError> {
    let file = File::open(path).map_err(|e| {
        parquet::errors::ParquetError::General(format!("Cannot open file: {e}"))
    })?;
    let reader = SerializedFileReader::new(file)?;
    let schema = reader.metadata().file_metadata().schema();
    let fields = schema.get_fields();
    Ok(fields
        .iter()
        .map(|f| (f.name().to_string(), parquet_type_to_dict_type(f)))
        .collect())
}

/// Returns type information for all columns in a parquet file, including dict type,
/// parquet logical type, and parquet physical type.
pub fn column_type_info(
    path: &Path,
) -> Result<Vec<ColumnTypeInfo>, parquet::errors::ParquetError> {
    let file = File::open(path).map_err(|e| {
        parquet::errors::ParquetError::General(format!("Cannot open file: {e}"))
    })?;
    let reader = SerializedFileReader::new(file)?;
    let schema = reader.metadata().file_metadata().schema();
    let fields = schema.get_fields();
    Ok(fields
        .iter()
        .map(|f| {
            let info = f.get_basic_info();
            ColumnTypeInfo {
                name: f.name().to_string(),
                dict_type: parquet_type_to_dict_type(f),
                logical_type: info.logical_type().map(format_logical_type),
                physical_type: format!("{:?}", f.get_physical_type()),
            }
        })
        .collect())
}

/// What a column's data must be inspected for, decided per column by the caller
/// before any data is read. The scanner ([`column_stats`]) computes only what's
/// requested, so columns nothing asks about are never touched.
///
/// Holds the nulls request today; range/enum/examples requests (bounds to test,
/// an allowed set, an expected set) will become further fields as those checks
/// are added.
#[derive(Default, Clone)]
pub struct ColumnNeeds {
    /// Count nulls and sample the row numbers where they occur.
    pub nulls: bool,
}

impl ColumnNeeds {
    /// True if anything at all is requested. Columns for which this is false are
    /// skipped entirely.
    pub fn any(&self) -> bool {
        self.nulls
    }

    /// Combine two requests, taking the union (used to merge what several checks
    /// independently ask of the same column).
    pub fn merge(self, other: Self) -> Self {
        ColumnNeeds {
            nulls: self.nulls || other.nulls,
        }
    }
}

/// The statistics gathered for a column, populated only for the fields its
/// [`ColumnNeeds`] requested (others keep their default). Grows alongside
/// `ColumnNeeds` as checks are added.
#[derive(Default)]
pub struct ColumnStats {
    /// Total number of null values (when `nulls` was requested).
    pub null_count: usize,
    /// 1-based row numbers of the first few nulls, capped by the caller's limit.
    pub null_rows: Vec<usize>,
}

/// Gathers the requested statistics for each column, in one pass over the file.
///
/// Returns a [`ColumnStats`] for each requested column that exists in the file
/// and asks for something ([`ColumnNeeds::any`]); others are absent from the
/// map. Null row numbers are 1-based and in file order, capped at `limit` while
/// `null_count` is always the true total.
///
/// The work is kept to the minimum the requests imply:
///
/// - Per-row-group `null_count` statistics resolve a nulls request with no nulls
///   for free — the common case for a `required` column — so it's never scanned.
/// - When a scan is unavoidable, the row iterator is projected to just the
///   columns that need it, leaving the rest of a wide table undecoded.
///
/// Either way memory is bounded: rows stream one at a time and only the capped
/// sample is retained.
pub fn column_stats(
    path: &Path,
    needs: &HashMap<String, ColumnNeeds>,
    limit: usize,
) -> Result<HashMap<String, ColumnStats>, parquet::errors::ParquetError> {
    let file = File::open(path).map_err(|e| {
        parquet::errors::ParquetError::General(format!("Cannot open file: {e}"))
    })?;
    let reader = SerializedFileReader::new(file)?;
    let meta = reader.metadata();
    let schema = meta.file_metadata().schema();

    // Resolve the columns that ask for something to their position in the (flat)
    // schema; drop ones absent from the file. Position doubles as the
    // column-chunk index per row group.
    let requested: Vec<(String, usize, &ColumnNeeds)> = needs
        .iter()
        .filter(|(_, n)| n.any())
        .filter_map(|(name, n)| {
            schema
                .get_fields()
                .iter()
                .position(|f| f.name() == name)
                .map(|i| (name.clone(), i, n))
        })
        .collect();

    let mut stats: HashMap<String, ColumnStats> = requested
        .iter()
        .map(|(name, _, _)| (name.clone(), ColumnStats::default()))
        .collect();

    // A column must read data pages only when a request can't be satisfied from
    // metadata. Today that's a nulls request whose count statistics don't
    // already prove there are none.
    let to_scan: Vec<usize> = requested
        .iter()
        .filter(|(_, idx, n)| n.nulls && !nulls_provably_absent(meta, *idx))
        .map(|(_, idx, _)| *idx)
        .collect();

    if to_scan.is_empty() {
        return Ok(stats);
    }

    let projection = Type::group_type_builder("schema")
        .with_fields(to_scan.iter().map(|&i| schema.get_fields()[i].clone()).collect())
        .build()?;

    for (idx, row) in reader.get_row_iter(Some(projection))?.enumerate() {
        let row = row?;
        for (name, field) in row.get_column_iter() {
            let (Some(stat), Some(need)) = (stats.get_mut(name), needs.get(name)) else {
                continue;
            };
            if need.nulls && matches!(field, Field::Null) {
                stat.null_count += 1;
                if stat.null_rows.len() < limit {
                    stat.null_rows.push(idx + 1);
                }
            }
        }
    }

    Ok(stats)
}

/// Whether column `col`'s `null_count` statistics prove it holds no nulls. False
/// when any row group lacks the statistic, since absence isn't proof.
fn nulls_provably_absent(meta: &ParquetMetaData, col: usize) -> bool {
    let mut total = 0;
    for rg in meta.row_groups() {
        match rg.column(col).statistics().and_then(|s| s.null_count_opt()) {
            Some(n) => total += n,
            None => return false,
        }
    }
    total == 0
}

fn format_logical_type(lt: LogicalType) -> String {
    match lt {
        LogicalType::String => "String".into(),
        LogicalType::Map => "Map".into(),
        LogicalType::List => "List".into(),
        LogicalType::Enum => "Enum".into(),
        LogicalType::Decimal { precision, scale } => format!("Decimal({precision},{scale})"),
        LogicalType::Date => "Date".into(),
        LogicalType::Time {
            is_adjusted_to_u_t_c,
            unit,
        } => {
            let u = format_time_unit(unit);
            let tz = if is_adjusted_to_u_t_c { "UTC" } else { "local" };
            format!("Time({u},{tz})")
        }
        LogicalType::Timestamp {
            is_adjusted_to_u_t_c,
            unit,
        } => {
            let u = format_time_unit(unit);
            let tz = if is_adjusted_to_u_t_c { "UTC" } else { "local" };
            format!("Timestamp({u},{tz})")
        }
        LogicalType::Integer {
            bit_width,
            is_signed,
        } => {
            let sign = if is_signed { "i" } else { "u" };
            format!("Integer({sign}{bit_width})")
        }
        LogicalType::Unknown => "Unknown".into(),
        LogicalType::Json => "Json".into(),
        LogicalType::Bson => "Bson".into(),
        LogicalType::Uuid => "Uuid".into(),
        LogicalType::Float16 => "Float16".into(),
    }
}

fn format_time_unit(unit: TimeUnit) -> &'static str {
    match unit {
        TimeUnit::MILLIS(_) => "ms",
        TimeUnit::MICROS(_) => "us",
        TimeUnit::NANOS(_) => "ns",
    }
}

fn parquet_type_to_dict_type(field: &Type) -> String {
    let info = field.get_basic_info();

    // Logical type takes precedence; physical type is the fallback.
    // Unhandled logical types (Map, List, Time, Json, Bson, Uuid, Unknown) fall
    // through — Time lands on "number" via INT32/INT64, the rest on "string".
    // We'll handle nested types, at least List, later.
    if let Some(logical) = info.logical_type() {
        match logical {
            LogicalType::String => return "string".into(),
            LogicalType::Enum => return "enum".into(),
            LogicalType::Date => return "date".into(),
            LogicalType::Timestamp { .. } => return "datetime".into(),
            LogicalType::Integer { .. } | LogicalType::Float16 | LogicalType::Decimal { .. } => {
                return "number".into()
            }
            _ => {}
        }
    }

    match field.get_physical_type() {
        PhysicalType::BOOLEAN => "boolean".into(),
        PhysicalType::INT32 | PhysicalType::INT64 => "number".into(),
        PhysicalType::INT96 => "datetime".into(),
        PhysicalType::FLOAT | PhysicalType::DOUBLE => "number".into(),
        PhysicalType::BYTE_ARRAY | PhysicalType::FIXED_LEN_BYTE_ARRAY => "string".into(),
    }
}
