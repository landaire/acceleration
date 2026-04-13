pub mod error;
pub mod file_table;
pub mod hash;
pub mod header;
pub mod io;
pub mod package;
pub mod types;
pub mod wrappers;

// Re-export key types at crate root for convenience
pub use error::StfsError;
pub use file_table::StfsFileEntry;
pub use file_table::StfsFileTable;
pub use file_table::StfsTreeNode;
pub use hash::HashTableMeta;
pub use header::XContentHeader;
pub use io::ReadAt;
pub use io::SliceReader;
pub use package::StfsPackage;
pub use types::*;
pub use wrappers::bytes::BytesStfsWrapper;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
