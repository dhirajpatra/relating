pub mod error;
pub mod mapping;
pub mod rest;
pub mod sql;
pub mod test_stub;

pub use error::ConnectorError;
pub use mapping::{map_rows_to_records, rows_content_hash, FieldMapping, RowMap};
pub use rest::{extract_records, fetch_rest_rows, PaginationConfig, RestConfig, RestFetch};
pub use sql::{fetch_sql_rows, validate_read_only_query, SqlConfig, SqlFetch};
