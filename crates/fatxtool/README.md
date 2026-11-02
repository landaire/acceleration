# fatxtool

Command-line inspector and recovery tool for Xbox 360 FATX volumes, built on [`fatx`](../fatx/).

- Lists the directory tree of a volume, deleted files and directories included.
- Copies live files out, one at a time or all at once.
- Scans free clusters for deleted directory records and known file types, and recovers the data of deleted files with a confidence rating.

```text
fatxtool /path/to/partition.bin list --deleted
fatxtool /path/to/partition.bin extract /DataRec360.xex /tmp/DataRec360.xex
fatxtool /path/to/partition.bin extract-all /tmp/partition
fatxtool /path/to/partition.bin scan
fatxtool /path/to/partition.bin recover /Content/deleted.bin /tmp/deleted.bin
fatxtool /path/to/partition.bin info
```

The input is mapped read-only. An output that is the input under another name is refused.

## Choosing the volume

By default the whole file is the volume. Use `--offset` and `--length` for a volume inside a larger image:

```text
fatxtool /path/to/mu.bin --offset 0 --length 0x1a4a1000 list
```

A volume file such as a flash `.fdf` file is often shorter than the partition its FAT was sized for. `fatxtool` finds that partition length from the FAT and the root directory, and says so when it uses it. Pass `--partition-length` to give it yourself.

A partition kept in a file that the console can grow always has a 32-bit FAT and 16 KiB clusters. Pass `--alternate-partition` to mount it that way.

[`xflash`](../xflash/README.md) runs the same commands on the memory unit of a raw NAND image and on flash volume files, with `xflash nand.bin fatx <volume> <command>`.

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
