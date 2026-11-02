pub use xenon_io::IntegrityPolicy;

use crate::raw::RawWarning;

/// A value read from flash and the integrity checks it failed.
pub type Checked<T> = xenon_io::Checked<T, RawWarning>;

pub type IntegrityError = xenon_io::IntegrityError<RawWarning>;
