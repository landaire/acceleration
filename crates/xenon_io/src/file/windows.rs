use std::fs::File;
use std::io;
use std::os::windows::fs::FileExt;

pub(super) fn read_exact_at(file: &File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
	while !bytes.is_empty() {
		match file.seek_read(bytes, offset) {
			Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
			Ok(read) => {
				bytes = &mut bytes[read..];
				offset += read as u64;
			}
			Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
			Err(error) => return Err(error),
		}
	}

	Ok(())
}

pub(super) fn write_all_at(file: &File, mut data: &[u8], mut offset: u64) -> io::Result<()> {
	while !data.is_empty() {
		match file.seek_write(data, offset) {
			Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
			Ok(written) => {
				data = &data[written..];
				offset += written as u64;
			}
			Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
			Err(error) => return Err(error),
		}
	}

	Ok(())
}
