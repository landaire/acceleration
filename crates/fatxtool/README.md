# fatxtool

Command-line inspector, recovery tool and editor for Xbox 360 FATX volumes, built on [`fatx`](../fatx/).

- Lists the directory tree of a volume, deleted files and directories included.
- Copies live files out, one at a time or all at once.
- Scans free clusters for deleted directory records and known file types, and recovers the data of deleted files with a confidence rating.
- Identifies hard disk, memory unit and USB storage images, prints their partitions, and opens a volume by its partition name.
- Writes a devkit partition table to a disk image file.
- Formats volumes and creates, replaces, deletes and moves files and directories in image files, as the console does.

```text
fatxtool /path/to/partition.bin list --deleted
fatxtool /path/to/partition.bin extract /DataRec360.xex /tmp/DataRec360.xex
fatxtool /path/to/partition.bin extract-all /tmp/partition
fatxtool /path/to/partition.bin scan
fatxtool /path/to/partition.bin recover /Content/deleted.bin /tmp/deleted.bin
fatxtool /path/to/partition.bin info
```

The input is mapped read-only for these commands. Only `partition-table write` and the commands under [Changing a volume](#changing-a-volume) change it, and they only open regular files. An output that is the input under another name is refused.

## Choosing the volume

By default the whole file is the volume. On a whole disk or memory unit image, `partitions` shows what the image holds, and `--partition` opens a volume by its name:

```text
fatxtool /path/to/hdd.bin partitions
fatxtool /path/to/hdd.bin --partition Partition1 list
fatxtool /path/to/mu.bin --partition Mu0 extract-all /tmp/mu
```

`partitions` prints the device it found and why, such as the devkit partition table in sector 0, the security sector, or an `XTAF` header at the start of a partition. It then lists every partition with its range and whether a FATX volume starts there. Partition names are the console's device names, so they match what you see in kernel paths like `\Device\Harddisk0\Partition1`. Pass `--large-memory` for a development kit with the large memory, which moves its MemDump partition.

For a USB stick, give the `Xbox360` directory that holds the `Data0000`, `Data0001` and later files. `fatxtool` joins them the way the console does:

```text
fatxtool /media/usb/Xbox360 partitions
fatxtool /media/usb/Xbox360 --partition Storage list
```

Use `--offset` and `--length` for a volume at any other place in an image:

```text
fatxtool /path/to/mu.bin --offset 0 --length 0x1a4a1000 list
```

A volume file such as a flash `.fdf` file is often shorter than the partition its FAT was sized for. `fatxtool` finds that partition length from the FAT and the root directory, and says so when it uses it. Pass `--partition-length` to give it yourself.

A partition kept in a file that the console can grow always has a 32-bit FAT and 16 KiB clusters. A partition chosen with `--partition` is mounted that way when its layout says so. Otherwise pass `--alternate-partition`.

[`xflash`](../xflash/README.md) runs the same commands on the memory unit of a raw NAND image and on flash volume files, with `xflash nand.bin fatx <volume> <command>`.

## Devkit partition tables

`partition-table write` puts a devkit partition table in sector 0 of a disk image. It lays the partitions out in the order you give, from the same 512 KiB start as a retail disk. Each size is in bytes, and `rest` takes the remaining space:

```text
fatxtool /tmp/devkit.bin partition-table write Cache0=0x80000000 Cache1=0x80000000 DumpPartition=0x20e30000 SystemPartition=0x10000000 Partition1=rest
```

It only writes regular files, so a block device or a link to one is refused. It checks the table first and refuses partitions that overlap, run past the end of the image, or cover the table and the security sector. It does not format the partitions.

## Recovery

Deleting a file frees its clusters and keeps only its first cluster and size. `recover` reads the clusters that follow the first cluster, so it works for files that were stored in one piece.

```text
fatxtool partition.bin recover /Content/deleted.bin /tmp/deleted.bin
fatxtool partition.bin recover --cluster 0xa02 --size 7376896 /tmp/data.bin
fatxtool partition.bin recover --strategy contiguous-free /Content/deleted.bin /tmp/deleted.bin
```

It prints whether the bytes look like the type the name implies, a confidence rating, and the live files that hold any of those clusters now. With `--strategy contiguous-free`, clusters that live files hold are not read and are zero in the output.

`recover` writes to a temporary file beside the output and renames it into place only after every check passed, so a failed recovery leaves the output untouched. No command writes through a symbolic link, so a link already in the destination cannot send the output elsewhere.

## Integrity checks

A directory record the console would skip or a broken cluster chain prints a warning, and the command goes on. Pass `--strict` to stop with an error instead.

## Changing a volume

These commands change a volume in an image file. They refuse block devices, character devices, directories and symbolic links, so work on a copy of the disk or memory unit image.

```text
fatxtool /tmp/mu-copy.bin --partition Mu0 mkdir /Content
fatxtool /tmp/mu-copy.bin --partition Mu0 put save.bin /Content/save.bin
fatxtool /tmp/mu-copy.bin --partition Mu0 mv /Content/save.bin /save.bin
fatxtool /tmp/mu-copy.bin --partition Mu0 rm /save.bin
fatxtool /tmp/blank.bin format --cluster-size 0x4000
```

- `put` replaces a file of the same name, and `rm` deletes a file or an empty directory.
- Names are 1 to 42 characters, match without regard to case, and may not hold control characters or `" * + , / : ; < = > ? \ |`. A path is at most 250 characters, and a directory holds at most 4096 records.
- A deleted file keeps its record until a new entry in that directory takes the slot, so `recover` works on it until then.
- `format` writes an empty volume over the selected range the way the console formats one, with 16 KiB clusters and a serial number from the clock unless you give `--cluster-size` or `--serial`. It refuses to replace an existing volume without `--force`.

If the listing of the volume prints warnings, a change is refused until you pass `--force` after the command, such as `rm /save.bin --force`. With `--strict` a change is never made over warnings.

A volume file such as a flash `.fdf` file only gets new clusters from inside the file. `fatxtool` does not grow the file, so a full one has to free space with `rm` first.
