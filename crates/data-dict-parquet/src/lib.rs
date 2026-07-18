//! Parquet reader for data-dict.yaml validation.

mod dictionary;
mod metadata;
mod scan;
mod uniqueness;

pub use metadata::{
    ColumnMeta, ColumnTypeInfo, column_meta, column_type_info, column_types, uniqueness_barriers,
};
pub use parquet::errors::ParquetError;
pub use scan::{ColumnNeeds, ColumnStats, column_stats};
pub use uniqueness::{UniquenessCheck, UniquenessStats, uniqueness_stats};
