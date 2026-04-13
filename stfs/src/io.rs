use std::ops::Range;

use crate::error::StfsError;

pub trait ReadAt {
    fn read_at(&self, range: Range<usize>) -> Result<impl AsRef<[u8]>, StfsError>;
}

pub struct SliceReader<'a>(pub &'a [u8]);

impl<'a> ReadAt for SliceReader<'a> {
    fn read_at(&self, range: Range<usize>) -> Result<impl AsRef<[u8]>, StfsError> {
        if range.end > self.0.len() {
            return Err(StfsError::ReadError {
                offset: range.start,
                message: format!(
                    "read range {}..{} exceeds data length {}",
                    range.start,
                    range.end,
                    self.0.len()
                ),
            });
        }
        Ok(&self.0[range])
    }
}
