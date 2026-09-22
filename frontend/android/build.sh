#!/usr/bin/env bash
set -euo pipefail
app=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$app/.." && pwd)
sdk=${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/android-sdk}}
ndk=${ANDROID_NDK_HOME:-$sdk/ndk/27.2.12479018}
tools=$sdk/build-tools/${ANDROID_BUILD_TOOLS_VERSION:-35.0.0}
llvm=$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin
abi=${ANDROID_ABI:-arm64-v8a}
case "$abi" in
    arm64-v8a) triple=aarch64-linux-android; clang=aarch64-linux-android29-clang ;;
    x86_64) triple=x86_64-linux-android; clang=x86_64-linux-android29-clang ;;
    *) echo "Unsupported ABI: $abi (use arm64-v8a or x86_64)" >&2; exit 1 ;;
esac
target=$(realpath -m "${CARGO_TARGET_DIR:-$root/target}")
out=$target/android/$abi
keystore=${ANDROID_KEYSTORE:-$HOME/.android/debug.keystore}
jar=$sdk/platforms/android-35/android.jar
for tool in "$llvm/llvm-strip" "$llvm/$clang" "$tools/aapt2" "$tools/d8" "$tools/zipalign" "$tools/apksigner"; do
    test -x "$tool" || { echo "Missing Android build tool: $tool" >&2; exit 1; }
done
test -f "$jar" || { echo "Install Android SDK platform 35" >&2; exit 1; }
cd "$root"
variable=${triple//-/_}
export "CC_$variable=$llvm/$clang" "AR_$variable=$llvm/llvm-ar"
cargo build --release --locked --target "$triple" -p tau-frontend --lib --target-dir "$target" \
    --config net.git-fetch-with-cli=true \
    --config "target.$triple.linker=\"$llvm/$clang\"" \
    --config "target.$triple.rustflags=[\"-C\", \"link-arg=-Wl,-z,max-page-size=16384\", \"-C\", \"link-arg=-Wl,-z,common-page-size=16384\"]"
mkdir -p "$out/classes" "$out/dex"
# Keep Cargo's original for symbolication; never strip shared cache artifacts.
cp "$target/$triple/release/libtau_frontend.so" "$out/libtau_frontend.so"
"$llvm/llvm-strip" --strip-unneeded "$out/libtau_frontend.so"
find "$out/classes" "$out/dex" -type f -delete
javac -encoding UTF-8 -source 8 -target 8 -Xlint:-options -classpath "$jar" \
    -d "$out/classes" "$app/android/java/app/tau/rust/MainActivity.java"
mapfile -t classes < <(find "$out/classes" -name '*.class' | sort)
"$tools/d8" --release --min-api 29 --lib "$jar" --output "$out/dex" "${classes[@]}"
rm -f "$out/resources.zip" "$out/unsigned.apk" "$out/aligned.apk" \
    "$out/tau-frontend-$abi.apk" "$out/tau-frontend-$abi.apk.idsig"
"$tools/aapt2" compile --dir "$app/android/res" -o "$out/resources.zip"
"$tools/aapt2" link -I "$jar" --manifest "$app/android/AndroidManifest.xml" \
    -o "$out/unsigned.apk" "$out/resources.zip"
python3 - "$out/unsigned.apk" "$out/libtau_frontend.so" "$abi" \
    "$out/dex/classes.dex" <<'PY'
import sys
import zipfile
with zipfile.ZipFile(sys.argv[1], 'a', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as apk:
    # A standard, directly installable APK: PackageManager extracts this library.
    apk.write(sys.argv[2], 'lib/' + sys.argv[3] + '/libtau_frontend.so')
    apk.write(sys.argv[4], 'classes.dex', compress_type=zipfile.ZIP_DEFLATED)
PY
"$tools/zipalign" -f -P 16 4 "$out/unsigned.apk" "$out/aligned.apk"
if ! test -f "$keystore"; then
    mkdir -p "$(dirname "$keystore")"
    keytool -genkeypair -keystore "$keystore" -storepass android -keypass android \
        -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US"
fi
"$tools/apksigner" sign --ks "$keystore" --ks-key-alias androiddebugkey \
    --ks-pass pass:android --key-pass pass:android \
    --out "$out/tau-frontend-$abi.apk" "$out/aligned.apk"
"$tools/apksigner" verify --verbose "$out/tau-frontend-$abi.apk"
"$tools/zipalign" -c -P 16 4 "$out/tau-frontend-$abi.apk"
printf '\nBuilt %s\n' "$out/tau-frontend-$abi.apk"
