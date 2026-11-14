#!/usr/bin/env bash
# Run the real native decoder inside the debug application's Android process.
set -euo pipefail
cd "$(dirname "$0")/.."
sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "$sdk_root" ]]; then
  sdk_root="$(sed -n 's/^sdk.dir=//p' local.properties)"
fi
build_tools="$sdk_root/build-tools/35.0.0"
android_jar="$sdk_root/platforms/android-35/android.jar"
test_dir="$PWD/build/wowtype-smoke"
mkdir -p "$test_dir/classes"
./gradlew assembleDebug
javac -cp "$android_jar:build/intermediates/javac/debug/compileDebugJavaWithJavac/classes" \
  -d "$test_dir/classes" doc/wowtype-smoke/Smoke.java
"$build_tools/d8" --lib "$android_jar" --output "$test_dir" "$test_dir/classes/smoke/Smoke.class"
"$build_tools/aapt" package -f -M doc/wowtype-smoke/AndroidManifest.xml \
  -I "$android_jar" -F "$test_dir/test.apk"
(cd "$test_dir" && zip -q test.apk classes.dex)
"$build_tools/apksigner" sign --ks "${DEBUG_KEYSTORE:-debug.keystore}" \
  --ks-key-alias "${DEBUG_KEY_ALIAS:-debug}" \
  --ks-pass "pass:${DEBUG_KEYSTORE_PASSWORD:-debug0}" "$test_dir/test.apk"
adb -e install -r build/outputs/apk/debug/Unexpected-Keyboard-debug.apk
adb -e install -r "$test_dir/test.apk"
adb -e shell am instrument -w smoke.wowtype/smoke.Smoke | tee "$test_dir/result.txt"
grep -qx PASS "$test_dir/result.txt"
