//! Sans-io scaffolding shared by the Xbox 360 storage readers.
//!
//! A reader that needs bytes from a source is written as a [`Machine`]. [`Machine::step`]
//! names the next byte range the machine needs, or returns its output; the caller reads that
//! range from any source and passes the bytes to [`Machine::feed`]. The machine does no I/O of
//! its own, so the same parser runs over a file, a memory map, a network connection, or a
//! device that only the caller can reach. [`drive`] and [`drive_async`] run a machine to
//! completion against a [`ReadAt`] or [`AsyncReadAt`] source.
//!
//! A machine that also modifies its source is a [`WriteMachine`]. Its [`Effect::Write`] steps
//! name bytes to write, in the order they must reach the source. [`drive_write`] and
//! [`drive_write_async`] run one against a source that implements [`ReadAt`] and [`WriteAt`], or
//! their async forms. [`MemorySource`] is a writable in-memory image and [`FileSource`] a
//! writable image file.
//!
//! Each crate keeps its own error type as [`Machine::Error`]. A failed or short read is a
//! [`ReadError`], which the drivers and [`expect_len`] convert into the machine's error.
//!
//! A failed integrity check, such as an ECC mismatch or a record the driver would skip, does
//! not stop a read. The value comes back as a [`Checked`] with the failures as warnings of the
//! crate's own warning type, and the caller decides whether to ignore them or treat them as an
//! [`IntegrityError`].
//!
//! # Example
//!
//! ```
//! use std::ops::Range;
//!
//! use xenon_io::Machine;
//! use xenon_io::ReadError;
//! use xenon_io::SliceSource;
//! use xenon_io::Step;
//! use xenon_io::drive;
//! use xenon_io::expect_len;
//!
//! #[derive(Debug, thiserror::Error)]
//! enum Error {
//!     #[error(transparent)]
//!     Read(#[from] ReadError),
//! }
//!
//! /// Reads a big-endian length at offset 0, then that many bytes after it.
//! struct ReadBlob {
//!     requested: Option<Range<u64>>,
//!     length: Option<u32>,
//!     blob: Option<Vec<u8>>,
//! }
//!
//! impl Machine for ReadBlob {
//!     type Output = Vec<u8>;
//!     type Error = Error;
//!
//!     fn step(&mut self) -> Result<Step<Vec<u8>>, Error> {
//!         if let Some(blob) = self.blob.take() {
//!             return Ok(Step::Done(blob));
//!         }
//!
//!         let range = match self.length {
//!             None => 0..4,
//!             Some(length) => 4..4 + u64::from(length),
//!         };
//!         self.requested = Some(range.clone());
//!
//!         Ok(Step::Read(range))
//!     }
//!
//!     fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
//!         let range = self.requested.take().expect("a read was requested");
//!         expect_len(&range, bytes)?;
//!         match self.length {
//!             None => self.length = Some(u32::from_be_bytes(bytes.try_into().expect("length checked"))),
//!             Some(_) => self.blob = Some(bytes.to_vec()),
//!         }
//!
//!         Ok(())
//!     }
//! }
//!
//! let source = SliceSource([0, 0, 0, 2, 0xab, 0xcd, 0xef]);
//! let blob = drive(ReadBlob { requested: None, length: None, blob: None }, &source)?;
//! assert_eq!(blob, [0xab, 0xcd]);
//! # Ok::<(), Error>(())
//! ```

pub mod checked;
pub mod error;
pub mod file;
pub mod io;
pub mod machine;

pub use checked::Checked;
pub use checked::IntegrityError;
pub use checked::IntegrityPolicy;
pub use error::ReadError;
pub use error::ReadErrorKind;
pub use error::WriteError;
pub use file::FileSource;
pub use io::AsyncReadAt;
pub use io::AsyncWriteAt;
pub use io::MemorySource;
pub use io::ReadAt;
pub use io::ReadOnly;
pub use io::SliceSource;
pub use io::WriteAt;
pub use io::drive;
pub use io::drive_async;
pub use io::drive_write;
pub use io::drive_write_async;
pub use machine::Effect;
pub use machine::Machine;
pub use machine::Step;
pub use machine::WriteMachine;
pub use machine::expect_len;
