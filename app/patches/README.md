# Compose selection patches

Compose Multiplatform 1.12.0 is the current stable version. Its 1.13.0-alpha01
selection code has the same defects. These are build-time patches, not a full
framework fork. They preserve drag selection, copying and auto-scroll without a
runtime hook, reflection, selection reset or catch-and-continue policy.

## Top and bottom boundaries

A pointer on the top edge is classified as inside by getYDirection, but as before
the text by getOffsetForPosition. When a horizontal viewport reaches the line's
end, the latter can return 0 while the selection is marked forward. A selection
starting at 5 then asks to draw 5..0.

The delegate patch makes both comparisons strict and uses layout height for the
bottom bound, matching the geometric classifier. Glyph height can be smaller
than layout height. With padding, changing only the two operators still crashes
a reverse drag (drawing 710..705). The regression uses padded text to cover both
issues.

The JVM changes are IFGT -> IFGE and IFLT -> IFLE: these branches skip the early
0/length return, hence the inverse of the Kotlin source comparisons. The height
lookup becomes TextLayoutResult.size.height (the low 32 bits of packed IntSize,
converted to float). It adds no call or allocation.

## Multiline selection direction

The boundary patch shipped in Windows 0.5.14. It does not fix a separate mismatch
at horizontal edges. With a pointer left of a multiline text block, the widget
order says the selection is reversed, while the text offset can belong to a
later line. A real mouse drag in 553 characters produces start=1, end=68 and
handlesCrossed=true. Drawing then asks for the invalid range 68..1. Dragging right
of an earlier line can produce the opposite mismatch. Wrapped text has the same
problem.

SelectionManager.updateSelection now corrects direction after adjustment and
before publishing the selection or deriving its per-widget ranges. When both
anchors belong to one widget, direction follows their offsets. A copy is made
only when the flag disagrees. Offsets and multi-widget ordering stay unchanged.
This corrects the selection used by highlighting and copying, not just drawing.

The transform inserts the equivalent of the adjacent source patch after the
existing newSelection local is assigned. Both pinned platform methods use slot
9. Its stack frames use the existing locals and the maximum stack stays at six.
No local, field, method or runtime dependency is added.

## Build and verification

`ComposeSelectionPatch` is a cached Gradle artifact transform. It patches the
pinned desktop JAR and Android AAR's classes.jar. It checks both original class
SHA-256 values, the delegate's two changed branches and two height replacements,
and one selection-manager insertion. Missing, duplicate and unknown classes
fail the build. Component metadata rejects any other version.

Inputs in Gradle's dependency cache remain unchanged; only derived build
artifacts are patched. The transform is registered for both client modules so
compilation, desktop shrinking and Android DEX consume the same fixes. Other
dependencies are untouched. The explicit AAR artifact type registration is
required: AGP does not define its default transform attributes. The patch
attribute distinguishes raw JARs from raw AARs. AGP's AAR-to-JAR conversion keeps
the raw AAR marker, so only patching before conversion is valid. A shared
unpatched boolean allowed two orders and made release lint resolution ambiguous.
Both origins converge on the same patched marker. The hash-only passthrough is
JetBrains' empty Android wrapper; the implementation is in the AndroidX AAR.

Preserve the source patch and input checks together. A dependency upgrade must
remove or review these patches rather than silently bypass them. Before removal,
check the upstream source and rerun the real-window regression: padded single
lines, multiline and wrapped text, both drag directions, selection across widgets,
and copied contents. Also verify both resolved desktop and Android artifacts.

Sources: org.jetbrains.compose.foundation:foundation-desktop:1.12.0 source JAR,
commonMain/androidx/compose/foundation/text/selection/{MultiWidgetSelectionDelegate,SelectionManager}.kt.
The same source is in androidx.compose.foundation:foundation-android:1.12.0.
Upstream source is Apache-2.0, Copyright The Android Open Source Project.
