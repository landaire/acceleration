use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

pub(super) fn read_exact_at(file: &File, bytes: &mut [u8], offset: u64) -> io::Result<()> {
	file.read_exact_at(bytes, offset)
}

pub(super) fn write_all_at(file: &File, data: &[u8], offset: u64) -> io::Result<()> {
	file.write_all_at(data, offset)
}
