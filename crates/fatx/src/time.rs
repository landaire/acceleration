//! FATX_TIME_STAMP: local time in two-second resolution, packed into a big-endian `u32` from the
//! most significant bit as year since 1980 (7 bits), month (4), day (5), hour (5), minute (6) and
//! seconds divided by two (5).

use std::fmt;

const BASE_YEAR: u16 = 1980;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(u32);

impl Timestamp {
	pub const fn from_raw(raw: u32) -> Self {
		Self(raw)
	}

	pub const fn raw(self) -> u32 {
		self.0
	}

	pub const fn year(self) -> u16 {
		BASE_YEAR + (self.0 >> 25) as u16
	}

	pub const fn month(self) -> u8 {
		((self.0 >> 21) & 0xf) as u8
	}

	pub const fn day(self) -> u8 {
		((self.0 >> 16) & 0x1f) as u8
	}

	pub const fn hour(self) -> u8 {
		((self.0 >> 11) & 0x1f) as u8
	}

	pub const fn minute(self) -> u8 {
		((self.0 >> 5) & 0x3f) as u8
	}

	pub const fn second(self) -> u8 {
		(self.0 & 0x1f) as u8 * 2
	}

	/// Whether every field names a real calendar date and time of day.
	pub fn is_valid(self) -> bool {
		(1..=12).contains(&self.month())
			&& (1..=days_in_month(self.year(), self.month())).contains(&self.day())
			&& self.hour() < 24
			&& self.minute() < 60
			&& self.second() < 60
	}

	/// Seconds since the Unix epoch, reading the stored local time as UTC. The volume does not
	/// record the console's time zone.
	pub fn unix_seconds(self) -> Option<i64> {
		if !self.is_valid() {
			return None;
		}

		let days = days_from_civil(i64::from(self.year()), i64::from(self.month()), i64::from(self.day()));
		let seconds = i64::from(self.hour()) * 3600 + i64::from(self.minute()) * 60 + i64::from(self.second());

		Some(days * 86400 + seconds)
	}
}

impl fmt::Display for Timestamp {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
			self.year(),
			self.month(),
			self.day(),
			self.hour(),
			self.minute(),
			self.second()
		)
	}
}

impl fmt::Debug for Timestamp {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Timestamp({self} raw {:#010x})", self.0)
	}
}

fn days_in_month(year: u16, month: u8) -> u8 {
	match month {
		2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
		2 => 28,
		4 | 6 | 9 | 11 => 30,
		_ => 31,
	}
}

/// Days from 1970-01-01 to a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
	let year = if month <= 2 { year - 1 } else { year };
	let era = year.div_euclid(400);
	let year_of_era = year - era * 400;
	let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
	let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

	era * 146097 + day_of_era - 719468
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn decodes_fields_from_most_significant_bit() {
		// Creation time of a file in a devkit flash volume.
		let timestamp = Timestamp::from_raw(0x3d29_9815);

		assert_eq!(timestamp.to_string(), "2010-09-09 19:00:42");
		assert_eq!(timestamp.unix_seconds(), Some(1_284_058_842));
	}

	#[test]
	fn rejects_impossible_dates() {
		// February 30th.
		let raw = (30 << 25) | (2 << 21) | (30 << 16);

		assert!(!Timestamp::from_raw(raw).is_valid());
		assert_eq!(Timestamp::from_raw(raw).unix_seconds(), None);
		assert!(!Timestamp::from_raw(0).is_valid());
	}
}
