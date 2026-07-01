use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use parquet::basic::{LogicalType, Repetition, TimeUnit, Type as PhysicalType};
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::schema::types::Type;

use crate::ParquetError;

pub struct ColumnTypeInfo {
    pub name: String,
    pub dict_type: String,
    pub logical_type: Option<String>,
    pub physical_type: String,
}

/// Footer statistics that can settle data-level checks without reading values.
#[derive(Debug, Clone, Copy)]
pub struct ColumnMeta {
    /// Total nulls across all row groups, or `None` when any row group omits
    /// null-count statistics. Required Parquet fields always report `Some(0)`.
    pub null_count: Option<usize>,
    /// Number of rows in the file.
    pub row_count: usize,
    /// Distinct values when a single row group's footer provides the count.
    /// Multiple row-group counts cannot prove file-wide uniqueness.
    pub distinct_count: Option<usize>,
}

/// Read the inexpensive, footer-only statistics for each top-level column.
pub fn column_meta(path: &Path) -> Result<HashMap<String, ColumnMeta>, ParquetError> {
    let file =
        File::open(path).map_err(|e| ParquetError::General(format!("Cannot open file: {e}")))?;
    let reader = SerializedFileReader::new(file)?;
    let meta = reader.metadata();
    let fields = meta.file_metadata().schema().get_fields();

    Ok(fields
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            let required = field.get_basic_info().has_repetition()
                && field.get_basic_info().repetition() == Repetition::REQUIRED;
            let null_count = if required {
                Some(0)
            } else {
                meta.row_groups().iter().try_fold(0usize, |total, rg| {
                    rg.column(idx)
                        .statistics()
                        .and_then(|s| s.null_count_opt())
                        .map(|count| total + count as usize)
                })
            };
            let distinct_count = match meta.row_groups() {
                [row_group] => row_group
                    .column(idx)
                    .statistics()
                    .and_then(|statistics| statistics.distinct_count_opt())
                    .map(|count| count as usize),
                _ => None,
            };
            (
                field.name().to_string(),
                ColumnMeta {
                    null_count,
                    row_count: meta.file_metadata().num_rows() as usize,
                    distinct_count,
                },
            )
        })
        .collect())
}

/// Returns `(column_name, data_dict_type)` pairs for all columns.
pub fn column_types(path: &Path) -> Result<Vec<(String, String)>, ParquetError> {
    let file =
        File::open(path).map_err(|e| ParquetError::General(format!("Cannot open file: {e}")))?;
    let reader = SerializedFileReader::new(file)?;
    let schema = reader.metadata().file_metadata().schema();
    Ok(schema
        .get_fields()
        .iter()
        .map(|field| (field.name().to_string(), parquet_type_to_dict_type(field)))
        .collect())
}

pub fn column_type_info(path: &Path) -> Result<Vec<ColumnTypeInfo>, ParquetError> {
    let file =
        File::open(path).map_err(|e| ParquetError::General(format!("Cannot open file: {e}")))?;
    let reader = SerializedFileReader::new(file)?;
    let schema = reader.metadata().file_metadata().schema();
    Ok(schema
        .get_fields()
        .iter()
        .map(|field| {
            let info = field.get_basic_info();
            ColumnTypeInfo {
                name: field.name().to_string(),
                dict_type: parquet_type_to_dict_type(field),
                logical_type: info.logical_type().map(format_logical_type),
                physical_type: format!("{:?}", field.get_physical_type()),
            }
        })
        .collect())
}

fn format_logical_type(logical_type: LogicalType) -> String {
    match logical_type {
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
            let unit = format_time_unit(unit);
            let timezone = if is_adjusted_to_u_t_c { "UTC" } else { "local" };
            format!("Time({unit},{timezone})")
        }
        LogicalType::Timestamp {
            is_adjusted_to_u_t_c,
            unit,
        } => {
            let unit = format_time_unit(unit);
            let timezone = if is_adjusted_to_u_t_c { "UTC" } else { "local" };
            format!("Timestamp({unit},{timezone})")
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

    if let Some(logical) = info.logical_type() {
        match logical {
            LogicalType::String => return "string".into(),
            LogicalType::Enum => return "enum".into(),
            LogicalType::Date => return "date".into(),
            LogicalType::Timestamp { .. } => return "datetime".into(),
            LogicalType::Integer { .. } | LogicalType::Float16 | LogicalType::Decimal { .. } => {
                return "number".into();
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
