//! Parquet reader for data-dict.yaml validation.

use parquet::basic::{LogicalType, Type as PhysicalType};
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::schema::types::Type;
use std::fs::File;
use std::path::Path;

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

fn parquet_type_to_dict_type(field: &Type) -> String {
    let info = field.get_basic_info();

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
