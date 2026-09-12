# Compose selection boundary patch

Compose Multiplatform 1.12.0 is the current stable version. Its 1.13.0-alpha01
selection code has the same defect. A pointer on the top edge is classified as
inside by getYDirection, but as before the text by getOffsetForPosition. When a
horizontal viewport reaches the line's end, the latter can return 0 while the
selection is marked forward. A selection starting at 5 then asks to draw 5..0.

The adjacent two-line source patch makes both comparisons strict and uses the
layout height for the bottom bound, matching the geometric classifier. Glyph
height can be smaller than layout height. With padding, changing only the two
operators still crashes a reverse drag (drawing 710..705). The regression uses
padded text to cover both issues. Selection, copying and auto-scroll remain intact.
It adds no runtime hook, reflection, selection reset or catch-and-continue policy.

`ComposeSelectionPatch` is a cached Gradle artifact transform. It applies the two
comparison changes to the pinned desktop JAR and Android AAR's classes.jar. It
checks the original class SHA-256, two changed branches and two height getter
replacements. Component metadata rejects any other version. Inputs in Gradle's
dependency cache remain
unchanged; only derived build artifacts are patched. The transform is registered
for both client modules so compilation, desktop shrinking and Android DEX consume
the same fix. Other dependencies are untouched. The explicit AAR artifact type
registration is required: AGP does not define its default transform attributes.
The hash-only passthrough in the transform is JetBrains' empty Android wrapper;
the implementation lives in the separate AndroidX AAR.

This avoids a full framework source fork for two comparisons. ASM is build-only.
The JVM changes are IFGT -> IFGE and IFLT -> IFLE: these branches skip the early
0/length return, hence the inverse of the Kotlin source comparisons. The height
lookup becomes TextLayoutResult.size.height (the low 32 bits of packed IntSize,
converted to float). It adds no call or allocation.
Control-flow structure and branch-target stack types remain unchanged. Preserve
the source patch and input checks together. A dependency upgrade must remove or
review this patch rather than silently bypass it.

Source: org.jetbrains.compose.foundation:foundation-desktop:1.12.0 source JAR,
commonMain/androidx/compose/foundation/text/selection/MultiWidgetSelectionDelegate.kt.
The same source is in androidx.compose.foundation:foundation-android:1.12.0.
Upstream source is Apache-2.0, Copyright The Android Open Source Project.

Before removing this patch, verify the fixed upstream source and rerun the real
horizontal-selection regression, including top/bottom edges, both directions and
copying. Also verify both resolved desktop and Android artifacts.
