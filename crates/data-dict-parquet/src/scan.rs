use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;

use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use parquet::schema::types::Type;

use crate::ParquetError;

/// What a column's data must be inspected for.
#[derive(Default, Clone)]
pub struct ColumnNeeds {
    /// Count nulls and sample the row numbers where they occur.
    pub nulls: bool,
    /// The set of allowed values (D04). When present, non-null values not in
    /// the set are counted and sampled. Values are the canonical string form
    /// produced by [`field_key`]; the caller must canonicalize its set to match.
    pub allowed: Option<HashSet<String>>,
}

impl ColumnNeeds {
    pub fn any(&self) -> bool {
        self.nulls || self.allowed.is_some()
    }

    pub fn merge(self, other: Self) -> Self {
        ColumnNeeds {
            nulls: self.nulls || other.nulls,
            allowed: self.allowed.or(other.allowed),
        }
    }
}

/// Statistics gathered by scanning a column's values.
#[derive(Default)]
pub struct ColumnStats {
    pub null_count: usize,
    /// 1-based row numbers, capped by the caller's limit.
    pub null_rows: Vec<usize>,
    /// Non-null values found outside the [`ColumnNeeds::allowed`] set.
    pub outside_count: usize,
    /// 1-based row numbers of outside values, capped by the caller's limit.
    pub outside_rows: Vec<usize>,
    /// Distinct offending values, capped by the caller's limit, in first-seen
    /// order.
    pub outside_values: Vec<String>,
}

/// Gather requested statistics in one projected, streaming pass over the file.
pub fn column_stats(
    path: &Path,
    needs: &HashMap<String, ColumnNeeds>,
    limit: usize,
) -> Result<HashMap<String, ColumnStats>, ParquetError> {
    let file =
        File::open(path).map_err(|e| ParquetError::General(format!("Cannot open file: {e}")))?;
    let reader = SerializedFileReader::new(file)?;
    let schema = reader.metadata().file_metadata().schema();

    let requested: Vec<(String, usize, &ColumnNeeds)> = needs
        .iter()
        .filter(|(_, need)| need.any())
        .filter_map(|(name, need)| {
            schema
                .get_fields()
                .iter()
                .position(|field| field.name() == name)
                .map(|index| (name.clone(), index, need))
        })
        .collect();

    let mut stats: HashMap<String, ColumnStats> = requested
        .iter()
        .map(|(name, _, _)| (name.clone(), ColumnStats::default()))
        .collect();

    // Fast path: settle the enum-membership need (D04) from dictionary pages
    // where the data conforms, sparing those columns the value scan. A column
    // still scanned for its nulls skips the redundant dictionary read.
    let proven: HashSet<&str> = requested
        .iter()
        .filter(|(_, _, need)| need.allowed.is_some() && !need.nulls)
        .filter_map(|(name, index, need)| {
            let allowed = need.allowed.as_ref()?;
            crate::dictionary::dictionary_conforms(&reader, *index, allowed)
                .ok()
                .filter(|&conforms| conforms)
                .map(|_| name.as_str())
        })
        .collect();

    let to_scan: Vec<usize> = requested
        .iter()
        .filter(|(name, _, need)| {
            need.nulls || (need.allowed.is_some() && !proven.contains(name.as_str()))
        })
        .map(|(_, index, _)| *index)
        .collect();

    if to_scan.is_empty() {
        return Ok(stats);
    }

    let projection = Type::group_type_builder("schema")
        .with_fields(
            to_scan
                .iter()
                .map(|&index| schema.get_fields()[index].clone())
                .collect(),
        )
        .build()?;

    for (index, row) in reader.get_row_iter(Some(projection))?.enumerate() {
        let row = row?;
        for (name, field) in row.get_column_iter() {
            let (Some(stat), Some(need)) = (stats.get_mut(name), needs.get(name)) else {
                continue;
            };
            if matches!(field, Field::Null) {
                if need.nulls {
                    stat.null_count += 1;
                    if stat.null_rows.len() < limit {
                        stat.null_rows.push(index + 1);
                    }
                }
                continue;
            }
            if let Some(allowed) = &need.allowed
                && !proven.contains(name.as_str())
                && let Some(key) = field_key(field)
                && !allowed.contains(&key)
            {
                stat.outside_count += 1;
                if stat.outside_rows.len() < limit {
                    stat.outside_rows.push(index + 1);
                }
                if stat.outside_values.len() < limit && !stat.outside_values.contains(&key) {
                    stat.outside_values.push(key);
                }
            }
        }
    }

    Ok(stats)
}

/// The canonical string form of a scalar field value, for set membership (D04).
/// `None` for kinds that can't be an `enum` value (a matching `enum` column
/// would already be an `M01` type mismatch). Each form follows the physical
/// width's `Display`; `Scalar::value_keys` on the spec side offers both float
/// widths so a `FLOAT` and a `DOUBLE` column each find a match.
fn field_key(field: &Field) -> Option<String> {
    Some(match field {
        Field::Bool(v) => v.to_string(),
        Field::Byte(v) => v.to_string(),
        Field::Short(v) => v.to_string(),
        Field::Int(v) => v.to_string(),
        Field::Long(v) => v.to_string(),
        Field::UByte(v) => v.to_string(),
        Field::UShort(v) => v.to_string(),
        Field::UInt(v) => v.to_string(),
        Field::ULong(v) => v.to_string(),
        Field::Float(v) => v.to_string(),
        Field::Double(v) => v.to_string(),
        Field::Str(v) => v.clone(),
        _ => return None,
    })
}
