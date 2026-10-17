#!/usr/bin/env bash
# Builds an installable Android APK (arm64) of Money Tracker.
#
#   scripts/build_app.sh [options]
#
#   --debug         debug build instead of release (inspectable WebView, bigger, slower)
#   --skip-checks   skip the validation gate (fmt, clippy, tests, guards)
#   --install       install the result on the device adb sees (adb install -r)
#   --allow-debug-key  let --install put a debug-signed APK on a REAL phone (see below)
#   --out DIR       where to put the APK (default: target/release-apk)
#   -h, --help      this text
#
# Signing
#   Default: signed with the Android *debug* key, so the file installs on a phone but
#   is labelled TEST-SIGNED. The debug key is public: never distribute that file.
#   The debug key is the same on every machine, so anyone can build an APK that Android
#   accepts as an UPDATE of an app signed with it (and that then reads its private data).
#   For that reason --install refuses to put a debug-signed APK on a real phone unless
#   you also pass --allow-debug-key (an emulator is always fine).
#
#   Release signing: set the path of YOUR keystore and its key alias. The passwords are
#   asked for just before signing, or taken from MT_KS_PASS / MT_KEY_PASS, and are given
#   to apksigner alone (never exported to the build, never on a command line). Do not
#   `export` them in your shell profile: pass them inline for one run:
#
#     export MT_KEYSTORE=/path/outside/the/repo/money-tracker.jks
#     export MT_KEY_ALIAS=money-tracker
#     MT_KS_PASS=... scripts/build_app.sh      # or leave it out to be asked
#
#   A release keystore is created once, by you, outside the repository:
#     keytool -genkeypair -v -keystore money-tracker.jks -alias money-tracker \
#       -keyalg RSA -keysize 4096 -validity 10000
#   Back it up somewhere safe: losing it means a new signature, and Android then makes
#   you uninstall the app (and lose its data, so make an encrypted backup first).
#
# Environment (auto-detected when unset): ANDROID_HOME, NDK_HOME, JAVA_HOME.

set -euo pipefail

start_dir="$PWD"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# The signing passwords are taken out of the environment at once, so that nothing the
# build runs (pnpm, cargo build scripts, Gradle and its long-lived daemon) can inherit
# them. They are handed to apksigner, and only to it, at the end.
ks_pass="${MT_KS_PASS-}"
key_pass="${MT_KEY_PASS-}"
unset MT_KS_PASS MT_KEY_PASS

profile=release
run_checks=1
do_install=0
allow_debug_key=0
out="$root/target/release-apk"

usage() { sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
step() { printf '\n==> %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --debug) profile=debug ;;
    --skip-checks) run_checks=0 ;;
    --install) do_install=1 ;;
    --allow-debug-key) allow_debug_key=1 ;;
    --out) shift; [ $# -gt 0 ] || die "--out needs a directory"; out="$1" ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

# --- environment -------------------------------------------------------------

step "Checking the toolchain"
for tool in pnpm cargo rustup python3; do
  command -v "$tool" >/dev/null || die "'$tool' is not installed or not on PATH"
done

if [ -z "${ANDROID_HOME:-}" ]; then
  for candidate in "$HOME/Library/Android/sdk" "$HOME/Android/Sdk" /opt/homebrew/share/android-commandlinetools; do
    if [ -d "$candidate/platform-tools" ]; then ANDROID_HOME="$candidate"; break; fi
  done
fi
[ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME" ] || die "ANDROID_HOME is not set and no Android SDK was found"
export ANDROID_HOME ANDROID_SDK_ROOT="$ANDROID_HOME"

if [ -z "${NDK_HOME:-}" ]; then
  NDK_HOME="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1 || true)"
fi
[ -n "${NDK_HOME:-}" ] && [ -d "$NDK_HOME" ] || die "NDK_HOME is not set and no NDK was found under $ANDROID_HOME/ndk"
export NDK_HOME

if [ -z "${JAVA_HOME:-}" ] && [ -x /usr/libexec/java_home ]; then
  JAVA_HOME="$(/usr/libexec/java_home 2>/dev/null || true)"
  [ -n "$JAVA_HOME" ] && export JAVA_HOME
fi

build_tools="$(ls -d "$ANDROID_HOME"/build-tools/* 2>/dev/null | sort -V | tail -1 || true)"
[ -n "$build_tools" ] || die "no build-tools under $ANDROID_HOME (sdkmanager 'build-tools;<version>')"
apksigner="$build_tools/apksigner"
aapt2="$build_tools/aapt2"
[ -x "$apksigner" ] || die "apksigner not found in $build_tools"

rustup target list --installed | grep -qx aarch64-linux-android \
  || die "Rust target missing: rustup target add aarch64-linux-android"

printf 'SDK:  %s\nNDK:  %s\nTools: %s\n' "$ANDROID_HOME" "$NDK_HOME" "$(basename "$build_tools")"

# --- signing choice (decided early so a typo fails before a long build) -------

signing=debug
if [ -n "${MT_KEYSTORE:-}" ]; then
  signing=release
  # Resolved against where the script was started, with symlinks followed, so that a
  # relative path cannot slip a key into the repository.
  keystore="$(cd "$start_dir" && python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$MT_KEYSTORE")"
  [ -f "$keystore" ] || die "MT_KEYSTORE points at a file that does not exist: $MT_KEYSTORE"
  [ -n "${MT_KEY_ALIAS:-}" ] || die "set MT_KEY_ALIAS to the key alias inside $keystore"
  case "$keystore" in
    "$root"/*) die "keep the keystore outside the repository, not at $keystore" ;;
  esac
elif [ "$profile" = release ]; then
  debug_ks="$HOME/.android/debug.keystore"
  [ -f "$debug_ks" ] || die "no $debug_ks (build any debug APK once, or set MT_KEYSTORE to sign with your own key)"
fi

# --- validation gate ------------------------------------------------------------

if [ "$run_checks" -eq 1 ]; then
  step "Validation gate (use --skip-checks to skip)"
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo check -p mt-storage -p mt-backup --benches --features mt-storage/bench-support
  if command -v cargo-llvm-cov >/dev/null; then
    cargo llvm-cov -p mt-core --fail-under-lines 85 --summary-only
  else
    echo "(cargo-llvm-cov not installed: skipping the core coverage gate; cargo install cargo-llvm-cov --locked)"
  fi
  [ -d node_modules ] || pnpm install --frozen-lockfile
  pnpm check
  pnpm test
  python3 -m unittest discover -s scripts -p 'test_*.py'
  python3 scripts/check_offline.py config .
  python3 scripts/check_offline.py source .
  python3 scripts/check_debug_redaction.py
  python3 scripts/check_workflows.py
fi

# --- build ------------------------------------------------------------------------

step "Building the $profile APK (arm64)"
build_marker="$(mktemp)"
trap 'rm -f "$build_marker"' EXIT
[ -d node_modules ] || pnpm install --frozen-lockfile
if [ "$profile" = debug ]; then
  pnpm tauri android build --apk --debug --target aarch64
  built="$root/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
else
  pnpm tauri android build --apk --target aarch64
  built="$root/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk"
fi
[ -f "$built" ] && [ "$built" -nt "$build_marker" ] \
  || die "the build did not produce a fresh $built (a stale file is never signed)"

# --- sign, verify -------------------------------------------------------------------

version="$(python3 - <<'PY'
import json
print(json.load(open("src-tauri/tauri.conf.json")).get("version", "0.0.0"))
PY
)"
mkdir -p "$out"

if [ "$profile" = debug ]; then
  # Gradle already signed the debug build with the debug key.
  label=debug
  apk="$out/money-tracker-$version-arm64-debug.apk"
  cp "$built" "$apk"
elif [ "$signing" = release ]; then
  step "Signing with your release key"
  label=signed
  apk="$out/money-tracker-$version-arm64-signed.apk"
  if [ -z "$ks_pass" ]; then
    [ -t 0 ] || die "MT_KS_PASS is not set and there is no terminal to ask on"
    read -rs -p "Keystore password: " ks_pass; echo
  fi
  # The passwords exist in the environment of this one command only.
  MT_KS_PASS="$ks_pass" MT_KEY_PASS="${key_pass:-$ks_pass}" "$apksigner" sign \
    --ks "$keystore" --ks-key-alias "$MT_KEY_ALIAS" \
    --ks-pass env:MT_KS_PASS --key-pass env:MT_KEY_PASS --out "$apk" "$built"
  ks_pass=""; key_pass=""
else
  step "Signing with the DEBUG key (test only)"
  label=TEST-SIGNED
  apk="$out/money-tracker-$version-arm64-TEST-SIGNED.apk"
  "$apksigner" sign --ks "$HOME/.android/debug.keystore" \
    --ks-pass pass:android --key-pass pass:android --out "$apk" "$built"
fi

step "Verifying"
# A file that fails a check is removed: nothing unverified is left under a normal name.
"$apksigner" verify --print-certs "$apk" | sed -n '1,3p' || { rm -f "$apk"; die "the signature does not verify"; }
if [ "$profile" = release ]; then
  python3 scripts/check_offline.py apk "$apk" "$aapt2" || { rm -f "$apk"; die "the APK failed the offline guard; it was deleted"; }
fi

size="$(du -h "$apk" | cut -f1)"
sum="$(shasum -a 256 "$apk" | cut -d' ' -f1)"
printf '\nAPK:    %s\nSize:   %s\nSHA-256 %s\n' "$apk" "$size" "$sum"
if [ "$label" = TEST-SIGNED ] || [ "$label" = debug ]; then
  printf '\nNOTE: signed with the public debug key. Fine for your own phone, never to hand out.\n'
  printf '      Switching later to a release-signed build needs an uninstall (make an\n'
  printf '      encrypted backup in the app first).\n'
fi

# --- install ---------------------------------------------------------------------------

if [ "$do_install" -eq 1 ]; then
  step "Installing on the connected device"
  adb="$ANDROID_HOME/platform-tools/adb"
  [ -x "$adb" ] || die "adb not found in $ANDROID_HOME/platform-tools"
  devices="$("$adb" devices | awk 'NR>1 && $2=="device" {print $1}')"
  [ -n "$devices" ] || die "no device: connect a phone with USB debugging, or start an emulator"
  [ "$(printf '%s\n' "$devices" | wc -l | tr -d ' ')" -eq 1 ] \
    || die "more than one device is connected; unplug extras or set ANDROID_SERIAL"
  if [ "$label" = TEST-SIGNED ] || [ "$label" = debug ]; then
    serial="$(printf '%s\n' "$devices" | head -1)"
    case "$serial" in
      emulator-*) ;;
      *) [ "$allow_debug_key" -eq 1 ] \
           || die "refusing to put a debug-signed APK on a real phone ($serial): anyone can sign an 'update' with that key. Pass --allow-debug-key if you accept that for a throw-away test, or sign with your own key (MT_KEYSTORE)." ;;
    esac
  fi
  "$adb" install -r "$apk"
  printf 'Installed. Open "Money Tracker" on the device.\n'
else
  printf '\nInstall on a connected phone with:  adb install -r "%s"\n' "$apk"
  printf 'or copy the file to the phone and open it (allow installs from that source).\n'
fi
