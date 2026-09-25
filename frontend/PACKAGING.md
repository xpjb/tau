# Beta packaging sizes

Future releases use [the automated rollout](../docs/beta-release.md). Non-debug
Windows release builds no longer request unused SDK PDBs; 0.7.4 artifacts below
are historical and were not rebuilt just to remove that build-time warning.

## 0.7.4 protocol-18 native block-sync beta

| Measured component | MiB |
|---|---:|
| Windows x64 installer | **9.46** |
| Windows native app | **31.08** |
| Android ARM64 APK | **11.25** |
| Android extracted native library | **26.50** |

The rewrite is merged/pushed to `origin/tau2` at `ca87e4d`, and the matching
0.7.4/protocol-18 beta daemon is deployed. Windows was sent first, Android second.
Installer payload bytes match the fresh native executable. Android package
`app.tau.rust`, versionCode **9**, versionName **0.7.4-beta**, retains the preceding
beta certificate and is not debuggable. APK CRC/payload, v3 signature, ZIP alignment
and 16 KiB ELF LOAD alignment passed. Both builds used the managed wrapper,
registry-offline, one Cargo/Rayon job; Android Java packaging used one processor.
Microsoft SDK missing-debug-PDB linker warnings were nonblocking. No extra full
test suite or physical-device test was run for package delivery.

Checksums: `dist/Tau-Beta-0.7.4-SHA256SUMS.txt`. Upgrade both clients; old protocol-15
clients do not match the new daemon. Stable Tau installation/data remain separate.


## 0.7.3 protocol-15 beta

| Measured component | MiB |
|---|---:|
| Windows x64 installer | **9.42** |
| Windows native app | **31.00** |
| Android ARM64 APK | **11.34** |
| Android extracted native library | **26.57** |

Windows installer embeds exactly the freshly built native app. Android package
`app.tau.rust` has versionCode **8**, versionName **0.7.3-beta**, and the same beta
signing certificate as 0.7.2. APK signature, ZIP CRC, compressed native payload,
ZIP alignment and 16 KiB ELF LOAD alignment passed. Both packages were sent after
the matched beta daemon deployed. Device UI testing remains with the user.

## 0.7.2 topics beta

| Measured component | MiB |
|---|---:|
| Windows x64 installer | **9.39** |
| Windows native app | **30.85** |
| Android ARM64 APK | **11.29** |
| Android extracted native library | **26.46** |

Built sequentially with managed Cargo, one Cargo job and one Rayon thread. Java
packaging tools used one active processor. Windows archive has exactly the fresh
`app/Tau Beta.exe`; the compressed bytes were verified inside the installer. No JVM,
font, PDB or extra runtime bundle. Windows linker warnings concern absent Microsoft
static-library debug PDBs, not missing runtime DLLs.

Android package `app.tau.rust`, versionCode **7**, versionName **0.7.2-beta**, not
debuggable. V3 signing certificate matches the existing beta APK. ZIP CRC, stripped
ARM64 native payload identity, ZIP alignment and **16 KiB** ELF LOAD alignment passed.
The APK is directly installable, compressed/extracted normally, and development-key
signed for beta updates. Physical Windows/Android rendering is not claimed by these
build/package checks. Both packages were sent through Tau.

## 0.7.0 integrated beta

| Measured component | MiB |
|---|---:|
| Windows x64 installer | **9.38** |
| Windows native app | **30.69** |
| Android ARM64 APK | **11.23** |
| Android extracted native library | **26.33** |

Both targets built sequentially with one managed Cargo job. Installer archive bytes
match the fresh native EXE and contain no JVM/JAR, bundled fonts or PDBs. Only OS
DLLs are imported. Android v3 signature matches 0.6.3, package `app.tau.rust`,
versionCode 5 / 0.7.0-beta, non-debuggable, extracted/compressed native library.
CRC, native payload identity, ZIP alignment and 16 KiB ELF load alignment passed.
The APK is development-key signed for beta updates, not a Play release.

APK + extracted library totals 37.56 MiB before OS-generated code/cache and data.
These are package measurements, not physical-device footprint or rendering claims.

## Historical 0.6.3 baseline

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
