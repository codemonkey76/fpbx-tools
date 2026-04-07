mod adapt;
mod export;
mod import;
mod uuid_remap;

pub use export::{export_domain_sql, export_domain_sql_v2};
pub use import::import_domain_sql;

/// Describe a domain rename to apply during restore.
/// All occurrences of `src_uuid` and `src_name` in the SQL are replaced with
/// `dest_uuid` and `dest_name` respectively before import.
#[derive(Debug, Clone)]
pub struct DomainRename {
    pub src_uuid: String,
    pub src_name: String,
    pub dest_uuid: String,
    pub dest_name: String,
}
