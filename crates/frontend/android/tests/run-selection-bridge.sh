#!/usr/bin/env bash
# An already-booted disposable x86_64 emulator is required; never installs Tau.
set -euo pipefail
: "${ANDROID_SERIAL:?Set ANDROID_SERIAL to a disposable x86_64 emulator}"
[[ $ANDROID_SERIAL == emulator-* ]] || { echo 'Emulator only' >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
sdk=${ANDROID_SDK_ROOT:-$HOME/android-sdk}
tools=$sdk/build-tools/35.0.0
jar=$sdk/platforms/android-35/android.jar
clang=$sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin/x86_64-linux-android29-clang
out=$(mktemp -d)
trap '"$sdk/platform-tools/adb" uninstall app.tau.selectiontest >/dev/null 2>&1 || true; rm -rf "$out"' EXIT
mkdir -p "$out/classes" "$out/dex"
cat > "$out/AndroidManifest.xml" <<'XML'
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.tau.selectiontest">
<uses-sdk android:minSdkVersion="29" android:targetSdkVersion="35"/>
<application android:theme="@android:style/Theme.Material.NoActionBar" android:extractNativeLibs="true" android:debuggable="true">
<activity android:name="app.tau.rust.MainActivity" android:exported="true"><meta-data android:name="android.app.lib_name" android:value="tau_frontend"/></activity>
</application>
<instrumentation android:name="app.tau.rust.SelectionBridgeTest" android:targetPackage="app.tau.selectiontest"/>
</manifest>
XML
"$clang" -shared -fPIC -std=c11 "$root/tests/native-input-stub.c" -landroid -o "$out/libtau_frontend.so"
javac -source 8 -target 8 -Xlint:-options -classpath "$jar" -d "$out/classes" \
    "$root/java/app/tau/rust/MainActivity.java" "$root/tests/SelectionBridgeTest.java"
mapfile -t classes < <(find "$out/classes" -name '*.class' | sort)
"$tools/d8" --min-api 29 --lib "$jar" --output "$out/dex" "${classes[@]}"
"$tools/aapt2" link -I "$jar" --manifest "$out/AndroidManifest.xml" -o "$out/test.apk"
python3 - "$out" <<'PY'
import sys, zipfile
from pathlib import Path
p=Path(sys.argv[1])
with zipfile.ZipFile(p/'test.apk','a',compression=zipfile.ZIP_DEFLATED) as z:
    z.write(p/'libtau_frontend.so','lib/x86_64/libtau_frontend.so')
    z.write(p/'dex/classes.dex','classes.dex')
PY
"$tools/zipalign" -f 4 "$out/test.apk" "$out/aligned.apk"
"$tools/apksigner" sign --ks "$HOME/.android/debug.keystore" --ks-key-alias androiddebugkey \
    --ks-pass pass:android --key-pass pass:android --out "$out/signed.apk" "$out/aligned.apk"
"$sdk/platform-tools/adb" install -r "$out/signed.apk"
timeout 60 "$sdk/platform-tools/adb" shell am instrument -w app.tau.selectiontest/app.tau.rust.SelectionBridgeTest | tee "$out/result"
grep -q 'PASS: floating toolbar' "$out/result"
