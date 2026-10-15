# xenon_types

Common types for Xbox 360 (Xenon) file formats. Provides newtypes for identifiers, keys, and addresses shared across the Xbox 360 ecosystem.

## Types

- `TitleId` - 32-bit game/application identifier
- `MediaId` - 32-bit media identifier
- `SavegameId` - 32-bit savegame identifier
- `ConsoleId` - 5-byte console identifier (hex serialized)
- `ProfileId` - 8-byte profile/XUID identifier (hex serialized)
- `DeviceId` - 20-byte device identifier (hex serialized)
- `VirtualAddress` - 32-bit virtual memory address
- `AesKey` - 16-byte AES-128 key
- `Version` - Xbox version (major.minor.build.revision packed in u32)
- `ContentId` - 20-byte XContent content identifier (hex serialized)
- `Sha1Hash` - 20-byte SHA-1 digest (hex serialized)
- `FileOffset` - absolute offset into an on-disk file
- `GameRegion` - game region bitflags (keyvault and XEX)
- FILETIME helpers: `filetime_from_xe_bytes`, `filetime_to_unix_secs`

## Feature Flags

- `jiff` - `filetime_to_timestamp` and `timestamp_to_filetime` conversions to and from `jiff::Timestamp`

## License

Licensed under either of Apache License, Version 2.0 or MIT License at your option; see [LICENSE](../LICENSE).
