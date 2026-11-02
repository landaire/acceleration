//! FATX_TIME_STAMP: local time in two-second resolution, packed into a big-endian `u32` from the
//! most significant bit as year since 1980 (7 bits), month (4), day (5), hour (5), minute (6) and
//! seconds divided by two (5).

use std::fmt;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

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

	/// The timestamp the driver stores for a time `seconds` and `nanoseconds` after the Unix
	/// epoch, in the time zone the caller chooses. Like fatsup.c FatxTimeToFatTimestamp, the
	/// time is rounded up to the next two seconds; `None` when the year is outside 1980 to
	/// 2107, which the driver rejects.
	pub fn from_unix(seconds: i64, nanoseconds: u32) -> Option<Self> {
		const TICKS_PER_SECOND: i128 = 10_000_000;

		// The driver adds 19999999 100-nanosecond ticks, then drops the odd second.
		let ticks = i128::from(seconds) * TICKS_PER_SECOND + i128::from(nanoseconds / 100);
		let rounded = (ticks + 2 * TICKS_PER_SECOND - 1).div_euclid(TICKS_PER_SECOND);
		let rounded = i64::try_from(rounded).ok()?;
		let (year, month, day) = civil_from_days(rounded.div_euclid(86400));
		let second_of_day = rounded.rem_euclid(86400);

		Self::from_fields(
			u16::try_from(year).ok()?,
			month,
			day,
			(second_of_day / 3600) as u8,
			(second_of_day / 60 % 60) as u8,
			(second_of_day % 60) as u8,
		)
	}

	/// [`Self::from_unix`] for a system time, read as the time zone of the volume.
	pub fn from_system_time(time: SystemTime) -> Option<Self> {
		match time.duration_since(UNIX_EPOCH) {
			Ok(after) => Self::from_unix(i64::try_from(after.as_secs()).ok()?, after.subsec_nanos()),
			// Before 1970, and so before the first year a timestamp holds.
			Err(_) => None,
		}
	}

	/// A timestamp of a calendar date and time of day; an odd second rounds down, as the field
	/// holds seconds divided by two. `None` for an impossible date or a year outside 1980 to
	/// 2107.
	pub fn from_fields(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<Self> {
		const LAST_YEAR: u16 = BASE_YEAR + 0x7f;

		if !(BASE_YEAR..=LAST_YEAR).contains(&year) {
			return None;
		}

		let raw = (u32::from(year - BASE_YEAR) << 25)
			| (u32::from(month & 0xf) << 21)
			| (u32::from(day & 0x1f) << 16)
			| (u32::from(hour & 0x1f) << 11)
			| (u32::from(minute & 0x3f) << 5)
			| u32::from((second / 2) & 0x1f);
		let timestamp = Self(raw);
		let fits = timestamp.month() == month && timestamp.day() == day && timestamp.hour() == hour;

		(fits && timestamp.minute() == minute && second < 60 && timestamp.is_valid()).then_some(timestamp)
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

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
	let days = days + 719468;
	let era = days.div_euclid(146097);
	let day_of_era = days - era * 146097;
	let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
	let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
	let month_index = (5 * day_of_year + 2) / 153;
	let day = day_of_year - (153 * month_index + 2) / 5 + 1;
	let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
	let year = year_of_era + era * 400 + i64::from(month <= 2);

	(year, month as u8, day as u8)
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

	#[test]
	fn writes_round_up_to_two_seconds() {
		// 2010-09-09 19:00:42 UTC.
		let even = 1_284_058_842;

		assert_eq!(Timestamp::from_unix(even, 0).unwrap().raw(), 0x3d29_9815);
		assert_eq!(Timestamp::from_unix(even - 1, 0).unwrap().raw(), 0x3d29_9815);
		assert_eq!(Timestamp::from_unix(even - 2, 100).unwrap().raw(), 0x3d29_9815);
		assert_eq!(Timestamp::from_unix(even, 100).unwrap().to_string(), "2010-09-09 19:00:44");
		// Fractions below the driver's 100-nanosecond tick are dropped.
		assert_eq!(Timestamp::from_unix(even, 99).unwrap().raw(), 0x3d29_9815);
		// 23:59:59 on the last day of a year rounds into the next year.
		assert_eq!(Timestamp::from_unix(1_293_839_999, 0).unwrap().to_string(), "2011-01-01 00:00:00");
	}

	#[test]
	fn years_outside_the_field_are_rejected() {
		assert_eq!(Timestamp::from_unix(315_532_800, 0).unwrap().to_string(), "1980-01-01 00:00:00");
		assert_eq!(Timestamp::from_unix(315_532_799, 0).unwrap().to_string(), "1980-01-01 00:00:00");
		assert_eq!(Timestamp::from_unix(315_532_797, 0), None);
		assert_eq!(Timestamp::from_fields(2107, 12, 31, 23, 59, 58).unwrap().year(), 2107);
		assert_eq!(Timestamp::from_fields(2108, 1, 1, 0, 0, 0), None);
		assert_eq!(Timestamp::from_fields(2010, 2, 30, 0, 0, 0), None);
		assert_eq!(Timestamp::from_fields(2010, 1, 1, 24, 0, 0), None);
	}
}
