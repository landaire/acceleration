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

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or [MIT License](../LICENSE-MIT) at your option.
