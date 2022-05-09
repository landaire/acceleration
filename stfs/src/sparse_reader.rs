use std::io::Read;

/// `SparseReader` helps reading data that is fragmented at various locations and
/// potentially has chunks of differing sizes.
///
/// # Example:
///
/// ```compile_fail
/// let first = [0u8, 1, 2, 3];
/// let second = [4u8];
/// let third = [5u8];
/// let mappings = [first.as_slice(), second.as_slice(), third.as_slice()];
/// let mut reader = SparseReader::new(&mappings);
/// let mut output = [0u8; 6];
/// assert!(matches!(reader.read(&mut output), Ok(6)));
///
/// assert_eq!([0u8, 1, 2, 3, 4, 5], output);
/// ```
pub struct SparseReader<'a, 'b> {
    mapping_index: usize,
    position: usize,
    mappings: &'b [&'a [u8]],
}

impl<'a, 'b> SparseReader<'a, 'b> {
    pub fn new(mappings: &'b [&'a [u8]]) -> SparseReader<'a, 'b> {
        SparseReader { mapping_index: 0, position: 0, mappings }
    }
}

impl<'a, 'b> Read for SparseReader<'a, 'b> {
    fn read(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
        let mut bytes_remaining = buf.len();
        let mut bytes_read = 0;

        if self.mapping_index >= self.mappings.len() {
            return Ok(0);
        }

        for (idx, mapping) in self.mappings.iter().skip(self.mapping_index).enumerate() {
            let (mapping_start, mapping_len) = if idx == 0 {
                (self.position, mapping.len() - self.position)
            } else {
                (0, mapping.len())
            };

            let bytes_to_copy = std::cmp::min(bytes_remaining, mapping_len);

            buf[..bytes_to_copy].copy_from_slice(&mapping[mapping_start..(mapping_start+bytes_to_copy)]);
            buf = &mut buf[bytes_to_copy..];
            bytes_read += bytes_to_copy;
            bytes_remaining -= bytes_to_copy;

            if bytes_remaining == 0 {
                self.mapping_index = idx + self.mapping_index;
                self.position = mapping_start + bytes_to_copy;

                if self.position == self.mappings[self.mapping_index].len() {
                    self.mapping_index += 1;
                    self.position = 0;
                }

                break;
            }
        }

        Ok(bytes_read)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::SparseReader;

    #[test]
    fn sparse_reader_works() {
        let first = [0u8, 1, 2, 3];
        let second = [4u8];
        let third = [5u8];
        let mappings = [first.as_slice(), second.as_slice(), third.as_slice()];
        let mut reader = SparseReader::new(&mappings);
        let mut output = [0u8; 6];
        assert!(matches!(reader.read(&mut output), Ok(6)));

        assert_eq!([0u8, 1, 2, 3, 4, 5], output);
    }

    #[test]
    fn sparse_reader_works_multiple_reads() {
        let first = [0u8, 1, 2, 3];
        let second = [4u8];
        let third = [5u8];
        let mappings = [first.as_slice(), second.as_slice(), third.as_slice()];
        let mut reader = SparseReader::new(&mappings);

        for i in 0..6 {
            let mut output = [0xFFu8];
            let result = reader.read(&mut output);
            println!("{:?}", result);
            println!("{}", i);
            assert!(matches!(result, Ok(1)));
            assert_eq!(output[0], i);
        }

        let mut output = [0xFFu8];
        assert!(matches!(reader.read(&mut output), Ok(0)));
    }
}
