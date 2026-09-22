# Beta packaging sizes

Measured from 0.6.3 ARM64 Android / x64 Windows artifacts. Sizes are MiB (2^20 bytes),
not estimates from a debug build.

| Artifact / component | 0.6.2 | 0.6.3 |
|---|---:|---:|
| Android APK download | 43.11 | **11.08** |
| Android extracted native library | not extracted | 25.96 |
| Android APK + native library on device | 43.11 | **37.04** |
| Windows installer download | 11.46 | **9.25** |
| Windows application executable | 33.98 | **30.23** |
| Windows clean install (app + two launchers + markers) | 34.68 | **30.93** |

Android device totals additionally include OS-generated compiled DEX/code cache,
metadata and private app data. Windows totals exclude filesystem allocation
rounding, private user data, retained older beta versions and the downloaded
installer. No installer/user-data cleanup was added: existing rollback copies
remain intact. These are clean-package footprints, not a measurement of the
user's current device.

## Android

A normal `.apk`, not an outer ZIP to unpack. The native library is copied to a
packaging staging path, stripped with the NDK's `llvm-strip --strip-unneeded`,
and ZIP-deflated at level 9. `extractNativeLibs=true` lets Android's installer
extract it automatically. Exported/dynamic symbols and 16KiB ELF alignment are
preserved. Originals stay in Cargo's build output for symbolication.

Compression reduces download size; it does not make native executable code smaller
in memory. There is now an extracted library alongside the compressed APK. Stripping
symbols and using system fonts reduce the actual library size. Both platforms omit
all eight embedded DejaVu fonts (3.76 MiB total). Android uses its API-29 font matcher
where static system font filenames are unavailable, not a new bundled fallback.

Signed with the existing beta key and package ID for in-place updates; not debuggable.

Reference: the available Tau 1 0.5.14 APK is 36.74 MiB, but includes **four ABIs**.
Subtracting its non-ARM64 compressed native payloads gives roughly 16.65 MiB;
that is a normalization estimate, not a separately rebuilt Tau 1 ARM64 APK.

## Windows

The installed application is an ordinary, uncompressed PE executable, using system
Segoe UI/Consolas and OS fallback. The installer LZMA-compresses the payload; unpacking
is automatic. No JVM, PDBs, font files, redundant native DLL bundle or runtime
executable packer is shipped. Static CRT preserves the no-extra-prerequisites install.

Preset 6 is the default: good compression without preset 9/extreme's high memory/CPU
cost. `TAU_LZMA_PRESET=0..9` remains available. One active beta version needs about
30.93 MiB. Older version directories are intentionally retained for rollback and
can make an existing installation larger; private data remains separate in Tau2.

Full-workspace LTO/size-profile experiments were not used: these would invalidate
the shared build cache and cause expensive recompilation during the user's high-load
period. The current reductions require no risky executable packing or loss of features.
