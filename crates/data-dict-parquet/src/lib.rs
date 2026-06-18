//! Parquet reader for data-dict.yaml validation.

use parquet::basic::{LogicalType, TimeUnit, Type as PhysicalType};
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::schema::types::Type;
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
pub fn column_types(path: &Path) -> Result<Vec<(String, String)>, parquet::errors::ParquetError> {
    let file = File::open(path)
        .map_err(|e| parquet::errors::ParquetError::General(format!("Cannot open file: {e}")))?;
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
pub fn column_type_info(path: &Path) -> Result<Vec<ColumnTypeInfo>, parquet::errors::ParquetError> {
    let file = File::open(path)
        .map_err(|e| parquet::errors::ParquetError::General(format!("Cannot open file: {e}")))?;
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
