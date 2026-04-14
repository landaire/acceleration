# STFS Hash Table Structure

STFS (Secure Transacted File System) is the filesystem used by Xbox 360 content packages (CON, LIVE, PIRS). Every data block is covered by a SHA-1 hash stored in a hash table. Hash tables are organized in a tree with up to three levels.

## Constants

- Block size: 0x1000 (4096 bytes)
- Hashes per hash table: 0xAA (170)
- Hash entry size: 0x18 (24 bytes) = 0x14 SHA-1 hash + 1 status byte + 3-byte next block pointer

## Hash Table Levels

The hash tree has up to three levels, determined by the total number of allocated blocks:

| Level | Name in code | Covers | Condition |
|-------|-------------|--------|-----------|
| 0 (First) | Level-0 | Up to 0xAA (170) data blocks | `allocated <= 0xAA` |
| 1 (Second) | Level-1 | Up to 0xAA^2 (28,900) data blocks | `allocated <= 0x70E4` |
| 2 (Third) | Level-2 | Up to 0xAA^3 (4,913,000) data blocks | `allocated <= 0x4AF768` |

Each level-0 hash table contains 0xAA entries, one per data block it covers. Each level-1 table contains entries covering level-0 tables (each entry's hash is the SHA-1 of a level-0 hash table block). Level-2 covers level-1 tables the same way.

For a small package (<=170 blocks), there is a single level-0 hash table. For a medium package (<=28,900 blocks), there is one level-1 table pointing to multiple level-0 tables. The top-level table is always the one whose hash is stored in the volume descriptor's `top_hash_table_hash` field.

## Block Layout and the "Sex" Flag

STFS packages have a property called "sex" (Female=0, Male=1), determined by bit 0 of the volume descriptor's `block_separation` byte:

```
female: (!block_separation) & 1 == 0
male:   (!block_separation) & 1 == 1
```

The sex determines how hash table blocks are interleaved with data blocks. Female packages allocate 1 block per hash table; male packages allocate 2 blocks per hash table (one active, one for write-back during transactions).

This maps directly to the read-only vs read-write distinction:

| Sex | Block step [0] | Block step [1] | Usage |
|-----|---------------|---------------|-------|
| Female (0) | 0xAB | 0x718F | Read-only packages (LIVE, PIRS). Hash tables occupy 1 block each. |
| Male (1) | 0xAC | 0x723A | Read-write packages (CON, save games). Hash tables occupy 2 blocks each (active + backup). |

The block step values encode the spacing between hash tables at each level. Step [0] is the distance between consecutive level-0 hash tables. Step [1] is the distance between level-1 hash tables.

For female packages, level-0 tables appear every 0xAB blocks (0xAA data + 1 hash). For male, every 0xAC blocks (0xAA data + 2 hash).

## Active Index

Male packages have two copies of each hash table block. The "active index" selects which copy is current. This enables atomic updates: write new hashes to the inactive copy, then flip the active index.

The active index is stored in different places depending on the level:

- **Top-level table**: `(volume_descriptor.block_separation & 2) >> 1` (bit 1 of block_separation)
- **Tables below the top**: Bit 6 (0x40) of the parent hash entry's status byte, shifted right by 6

The active index shifts the hash table's address by `active_index << 12` (one block width) from its base position.

For female packages, the active index is always effectively 0 since there's only one copy.

## Computing a Hash Table Block Address

To find the hash entry for data block N:

1. Compute the backing hash block number for level-0:
   - For `N < 0xAA`: block 0
   - For `N >= 0xAA`: `(N / 0xAA) * block_step[0] + ((N / 0xAA^2) + 1) << sex`, plus an additional `1 << sex` if `N >= 0xAA^2`

2. Convert to a file address:
   ```
   base = hash_block_number * BLOCK_SIZE + first_table_address
   ```

3. Add the active index offset (from the parent level's status byte or from the volume descriptor for the top table):
   ```
   address = base + (active_index << 12)
   ```

4. Index into the hash table to find the specific entry:
   ```
   entry_address = address + (N % 0xAA) * 0x18
   ```

## Data Block Address Computation

Logical data block numbers don't map 1:1 to file offsets because hash tables are interleaved. `compute_data_block_num` converts a logical block number to a "true" block number that accounts for the hash table blocks occupying space in the file:

```
true_block = ((N + 0xAA) / 0xAA) << sex + N
```

With additional corrections for blocks past the first and second hash tree level boundaries. The file offset is then:

```
offset = true_block * BLOCK_SIZE + first_table_address
```

Where `first_table_address = (header_size + 0xFFF) & 0xFFFFF000` (the first 4K-aligned offset after the XContent header).

## Hash Entry Format

Each 0x18-byte hash entry:

| Offset | Size | Field |
|--------|------|-------|
| 0x00 | 0x14 | SHA-1 hash of the block this entry covers |
| 0x14 | 0x01 | Status byte (bit 6 = active index for child table) |
| 0x15 | 0x03 | Next block number (24-bit big-endian, used for block chain traversal in file tables) |

## Hash Verification

To verify a data block's integrity, check two things:

1. **Data block hash**: SHA-1 the data block contents and compare against the block_hash in the level-0 hash entry.

2. **Hash chain**: Walk from the top table down. At each level, SHA-1 the entire hash table block and compare against the expected hash from the parent level (or the `top_hash_table_hash` from the volume descriptor for the top level).

Both checks must pass for the block to be considered valid.
