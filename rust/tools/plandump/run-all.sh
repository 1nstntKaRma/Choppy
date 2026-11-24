#!/bin/bash
# Step 1's side-by-side check: the C# Choppy (this branch, with --plan-dump) against the Rust one. Builds both, then
# prints SAME / DIFF for each check: every drive and drive pair on this PC, settings files of every kind, faked USB
# links, the Scanner on a test tree (copy, move, the same folder, into itself, relative paths, a network path through
# \\localhost\C$, an unreadable folder), --usb. Everything is read-only except the test tree in $WORK (removed at
# the end). Optional: HDD_SRC=D: HDD_DST=X: also scans a test tree from one hard disk to another (removed after).
# Usage (Git Bash): bash rust/tools/plandump/run-all.sh        WORK defaults to %TEMP%\choppy-plandump
# paths as C:/... : the Windows programs get them as they are
here=$(cygpath -m "$(cd "$(dirname "$0")" && pwd)")
repo=$(cygpath -m "$(cd "$(dirname "$0")/../../.." && pwd)")
export WORK="${WORK:-$(cygpath -m "$TEMP")/choppy-plandump}"
export CS="$WORK/cs/Choppy.exe"
export RS="$repo/rust/target/x86_64-pc-windows-msvc/release/choppy.exe"
export MSYS2_ARG_CONV_EXCL='*' DIFFN=12   # Git Bash must not rewrite \\server\share paths
mkdir -p "$WORK"; cd "$WORK" || exit 1
VER=$(tr -d '\r\n' < "$repo/VERSION")
dotnet build "$repo/src/Choppy.csproj" -c Release -nologo -v q "-p:Version=$VER.0" -o "$WORK/cs" | grep -E " error |Build succeeded"
(cd "$repo/rust" && cargo build -p choppy --release 2>&1 | grep -E "^error|Finished")
C() { bash "$here/compare.sh" "$@"; }

C drives
INI='VerifyCopies=1\r\nPolicy=Skip\r\nExplorerTakeover=1\r\nUltracopierRunBackup="C:\\\\x y\\\\u.exe" /tray\r\nView=extended\r\n' C set-old
INI='VerifyCopies=1\r\nPolicy=KeepBoth\r\nSpeedLimitMbps=478.150656\r\nLimitValueMbps=100\r\nCloseWhenDone=0\r\nCloseAfterSeconds=5\r\nExplorerTakeover=1\r\nSpeedUnit=mbs\r\nView=extended\r\nBenchRunsQuick=5\r\nBenchRunsThorough=100\r\nSettingsVersion=2\r\n' C set-v2
INI='policy= Replace, Skip \r\nSpeedLimitMbps= 1e3 \r\nLimitValueMbps=-5\r\nCloseAfterSeconds=1000\r\nCloseWhenDone=x\r\nverifycopies=1\r\nBenchRunsQuick=+7\r\nBenchRunsThorough= 99\r\nSettingsVersion= 2\r\n=nokey\r\nnoequals\r\n' C set-edge
INI='Policy=3\r\nSettingsVersion=3\r\nSpeedLimitMbps=0.1\r\nLimitValueMbps=1.5E-05\r\nCloseAfterSeconds=0\r\nSpeedUnit=\r\n' C set-num
INI='\xef\xbb\xbfPolicy=ReplaceOlder\nSpeedLimitMbps=12.5\rSettingsVersion=2\nUltracopierRunBackup=C:\\\\\xc3\xbc\xc3\xa9.exe' C set-crlf
INI='Policy=ask\r\nSpeedLimitMbps=abc\r\nLimitValueMbps=Infinity\r\nCloseAfterSeconds=-1\r\nSettingsVersion=1\r\nExplorerTakeover=0\r\n' C set-bad
INI='SpeedLimitMbps=0\r\nLimitValueMbps=0.000015\r\nSettingsVersion=2\r\nPolicy=1\r\n' C set-g1
INI='SpeedLimitMbps=100000000000000.5\r\nSettingsVersion=2\r\n' C set-g2
CHOPPY_FAKE_USB=2,3,1 C drives-fakeusb
CHOPPY_FAKE_USB=4,4 C drives-fakeusb2

# scans: run twice (a folder's listed times settle once it has been listed), folder access times masked
scan() {
  name=$1; shift
  C "$name" "$@" > /dev/null; C "$name" "$@" > /dev/null
  for x in cs rs; do sed -E 's/ a [0-9]+ m / a * m /' "out/$name.$x.txt" > "out/$name.$x.masked"; done
  if cmp -s "out/$name.cs.masked" "out/$name.rs.masked"; then echo "SAME $name ($(wc -l < "out/$name.cs.txt") lines, folder access times masked)"
  else echo "DIFF $name"; diff "out/$name.cs.masked" "out/$name.rs.masked" | head -10; fi
}
B='\'
rm -rf fx; python "$here/fixture.py" fx > /dev/null
F=$(cygpath -w "$WORK/fx")
U="$B$B"'localhost'"$B"'C$'"${F:2}"
scan scan-main --scan copy "$F${B}dst" "$F${B}src${B}a.txt" "$F${B}src${B}big.bin" "$F${B}src${B}Same.TXT" "$F${B}src${B}hidden.txt" "$F${B}src${B}sub1" "$F${B}src${B}link" "$F${B}src${B}long" "$F${B}src${B}conflict-dir" "$F${B}src${B}many" "$F${B}src${B}missing.txt"
scan scan-into-itself --scan copy "$F${B}src${B}sub1${B}zz" "$F${B}src${B}sub1"
scan scan-same-folder --scan copy "$F${B}dst" "$F${B}dst${B}x.bin" "$F${B}dst${B}folder" "$F${B}dst${B}a.txt"
scan scan-move-same --scan move "$F${B}dst${B}" "$F${B}dst${B}x.bin" "$F${B}src${B}a.txt"
scan scan-move-all --scan move "$F${B}dst" "$F${B}src"
scan scan-no-dest --scan copy "$F${B}nodest" "$F${B}src${B}sub1"
scan scan-relative --scan copy "fx${B}dst" "fx${B}src${B}sub1" "fx${B}src${B}" "\"fx${B}src${B}many\""
scan scan-unc --scan copy "$U${B}dst" "$U${B}src${B}sub1" "$F${B}src${B}a.txt"
scan scan-unc-src --scan copy "$F${B}dst" "$U${B}src${B}sub1" "$U${B}src${B}many"
scan scan-unreadable --scan copy "$F${B}nodest" "C:${B}System Volume Information" "$F${B}src${B}sub1"
python "$here/cleanup.py" "$F" > /dev/null
if [ -n "$HDD_SRC" ] && [ -n "$HDD_DST" ]; then
  python "$here/fixture.py" "$HDD_SRC${B}choppy-plandump-test" > /dev/null
  mkdir -p "$(cygpath -u "$HDD_DST${B}choppy-plandump-test")"
  cp -r "$(cygpath -u "$HDD_SRC${B}choppy-plandump-test${B}dst")" "$(cygpath -u "$HDD_DST${B}choppy-plandump-test")/"
  scan scan-hdd --scan copy "$HDD_DST${B}choppy-plandump-test${B}dst" "$HDD_SRC${B}choppy-plandump-test${B}src" "$HDD_SRC${B}choppy-plandump-test${B}src${B}big.bin"
  python "$here/cleanup.py" "$HDD_SRC${B}choppy-plandump-test" "$HDD_DST${B}choppy-plandump-test" > /dev/null
fi

# --usb (milliseconds masked), with and without a faked link
for fake in "" "2,3,0"; do
  rm -rf data; mkdir -p data
  CHOPPY_FAKE_USB=$fake CHOPPY_DATA="$WORK/data" "$CS" --usb "$WORK/out/usb.cs.txt"
  CHOPPY_FAKE_USB=$fake CHOPPY_DATA="$WORK/data" "$RS" --usb "$WORK/out/usb.rs.txt"
  for x in cs rs; do sed -E 's/\[[0-9]+ ms\]/[* ms]/' "out/usb.$x.txt" > "out/usb.$x.masked"; done
  if cmp -s out/usb.cs.masked out/usb.rs.masked; then echo "SAME --usb fake='$fake' (ms masked)"; else echo "DIFF --usb fake='$fake'"; diff out/usb.cs.masked out/usb.rs.masked; fi
done

# --selftest: the Rust lines are the C# ones of step 1 (names, order, results)
rm -rf data; mkdir -p data
CHOPPY_DATA="$WORK/data" "$CS" --selftest "$WORK/out/selftest.cs.txt"; echo "C# self-test exit $?"
CHOPPY_DATA="$WORK/data" "$RS" --selftest "$WORK/out/selftest.rs.txt"; echo "Rust self-test exit $?"
# line ends are compared without the CR (grep drops it; both files end their lines with CR LF)
grep -vE '^(PASS|FAIL) (temp|pipeline|hash |Ctrl[+]V|bench:)|^SKIP' "$WORK/out/selftest.cs.txt" | tr -d '\015' > "$WORK/out/selftest.cs.step1.txt"
tr -d '\015' < "$WORK/out/selftest.rs.txt" > "$WORK/out/selftest.rs.step1.txt"
if cmp -s "$WORK/out/selftest.cs.step1.txt" "$WORK/out/selftest.rs.step1.txt"; then echo "SAME --selftest step 1 ($(grep -c '^PASS' "$WORK/out/selftest.rs.txt") checks)"
else echo "DIFF --selftest"; diff "$WORK/out/selftest.cs.step1.txt" "$WORK/out/selftest.rs.step1.txt"; fi
