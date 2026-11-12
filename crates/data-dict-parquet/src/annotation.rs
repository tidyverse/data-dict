//! A column's type annotation, read from either of the two places the Parquet
//! format records it.

use parquet::basic::{ConvertedType, LogicalType, TimeUnit};
use parquet::format::{MicroSeconds, MilliSeconds};
use parquet::schema::types::Type;

/// The field's logical type, falling back to the legacy converted type when a
/// writer records only that: DuckDB (1.4 through 2.0, as of November 2026)
/// annotates a date column with just the converted type `DATE`. The parquet
/// crate derives a converted type from a logical one but never the reverse,
/// so a reader keying on the logical type alone would see such a column as
/// its bare physical type.
pub(crate) fn logical_type(field: &Type) -> Option<LogicalType> {
    field
        .get_basic_info()
        .logical_type()
        .or_else(|| from_converted(field))
}

fn from_converted(field: &Type) -> Option<LogicalType> {
    use ConvertedType::*;
    let integer = |bit_width, is_signed| {
        Some(LogicalType::Integer {
            bit_width,
            is_signed,
        })
    };
    // The format defines the legacy time and timestamp annotations as
    // UTC-adjusted instants.
    match field.get_basic_info().converted_type() {
        // `INTERVAL` has no logical counterpart.
        NONE | INTERVAL => None,
        UTF8 => Some(LogicalType::String),
        ENUM => Some(LogicalType::Enum),
        DATE => Some(LogicalType::Date),
        TIME_MILLIS => Some(LogicalType::Time {
            is_adjusted_to_u_t_c: true,
            unit: TimeUnit::MILLIS(MilliSeconds {}),
        }),
        TIME_MICROS => Some(LogicalType::Time {
            is_adjusted_to_u_t_c: true,
            unit: TimeUnit::MICROS(MicroSeconds {}),
        }),
        TIMESTAMP_MILLIS => Some(LogicalType::Timestamp {
            is_adjusted_to_u_t_c: true,
            unit: TimeUnit::MILLIS(MilliSeconds {}),
        }),
        TIMESTAMP_MICROS => Some(LogicalType::Timestamp {
            is_adjusted_to_u_t_c: true,
            unit: TimeUnit::MICROS(MicroSeconds {}),
        }),
        INT_8 => integer(8, true),
        INT_16 => integer(16, true),
        INT_32 => integer(32, true),
        INT_64 => integer(64, true),
        UINT_8 => integer(8, false),
        UINT_16 => integer(16, false),
        UINT_32 => integer(32, false),
        UINT_64 => integer(64, false),
        DECIMAL if field.is_primitive() => Some(LogicalType::Decimal {
            precision: field.get_precision(),
            scale: field.get_scale(),
        }),
        DECIMAL => None,
        JSON => Some(LogicalType::Json),
        BSON => Some(LogicalType::Bson),
        LIST => Some(LogicalType::List),
        // Legacy writers put `MAP_KEY_VALUE` on the map group itself; the
        // format's compatibility rules read it as `MAP`.
        MAP | MAP_KEY_VALUE => Some(LogicalType::Map),
    }
}

#[cfg(test)]
mod tests {
    use super::logical_type;
    use parquet::basic::{ConvertedType, LogicalType, TimeUnit, Type as PhysicalType};
    use parquet::schema::types::Type;

    fn converted(physical: PhysicalType, converted: ConvertedType) -> Type {
        Type::primitive_type_builder("c", physical)
            .with_converted_type(converted)
            .with_precision(if converted == ConvertedType::DECIMAL {
                9
            } else {
                -1
            })
            .with_scale(if converted == ConvertedType::DECIMAL {
                2
            } else {
                -1
            })
            .build()
            .unwrap()
    }

    #[test]
    fn converted_type_stands_in_for_an_absent_logical_type() {
        let date = converted(PhysicalType::INT32, ConvertedType::DATE);
        assert_eq!(date.get_basic_info().logical_type(), None);
        assert_eq!(logical_type(&date), Some(LogicalType::Date));

        let stamp = converted(PhysicalType::INT64, ConvertedType::TIMESTAMP_MICROS);
        assert!(matches!(
            logical_type(&stamp),
            Some(LogicalType::Timestamp {
                is_adjusted_to_u_t_c: true,
                unit: TimeUnit::MICROS(_)
            })
        ));

        let decimal = converted(PhysicalType::INT32, ConvertedType::DECIMAL);
        assert_eq!(
            logical_type(&decimal),
            Some(LogicalType::Decimal {
                precision: 9,
                scale: 2
            })
        );

        let plain = converted(PhysicalType::INT32, ConvertedType::NONE);
        assert_eq!(logical_type(&plain), None);
    }
}
