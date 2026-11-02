//! The kinds of storage device whose partitions the kernel lays out itself.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
	/// The SATA hard disk with the fixed retail layout (`drivers/sata/disk.c` SataDiskInitialize).
	RetailHdd,
	/// The SATA hard disk of a development kit, laid out by the table in sector 0.
	DevkitHdd,
	/// A memory unit on a USB port, `\Device\Mu0` and `\Device\Mu0System` (`usb/mass/mass.c`).
	UsbMemoryUnit,
	/// The memory unit in NAND flash, `\Device\BuiltInMuSfc` (`sfcx/mu.c`).
	SfcMemoryUnit,
	/// The whole eMMC of a console with the memory unit at 48 MiB, `\Device\BuiltInMuMmc`
	/// (`sfcx/mmcx.c`).
	MmcMemoryUnit,
	/// The built-in memory unit of a console with USB flash, `\Device\BuiltInMuUsb`
	/// (`usb/mass/mass.c`). A dump of only the memory unit region of an eMMC has this layout.
	UsbBuiltInMemoryUnit,
	/// The `Xbox360\Data####` files of a USB storage device, joined in order (`fs/fatw/fileinfo.c`).
	UsbStorage,
	/// A retail hard disk read through the transfer cable, `\Device\TransferCable`.
	TransferCable,
}

impl DeviceKind {
	pub const ALL: [Self; 8] = [
		Self::RetailHdd,
		Self::DevkitHdd,
		Self::UsbMemoryUnit,
		Self::SfcMemoryUnit,
		Self::MmcMemoryUnit,
		Self::UsbBuiltInMemoryUnit,
		Self::UsbStorage,
		Self::TransferCable,
	];

	pub const fn description(self) -> &'static str {
		match self {
			Self::RetailHdd => "retail hard disk",
			Self::DevkitHdd => "development kit hard disk",
			Self::UsbMemoryUnit => "USB memory unit",
			Self::SfcMemoryUnit => "NAND flash memory unit",
			Self::MmcMemoryUnit => "eMMC with a built-in memory unit",
			Self::UsbBuiltInMemoryUnit => "built-in memory unit",
			Self::UsbStorage => "USB storage files",
			Self::TransferCable => "hard disk through the transfer cable",
		}
	}
}

impl fmt::Display for DeviceKind {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.description())
	}
}
