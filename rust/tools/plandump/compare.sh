#!/bin/bash
# compare.sh <name> [--plan-dump args after the out file...]
# Runs the C# and the Rust `--plan-dump` with the same CHOPPY_DATA folder (fresh each time; settings.ini from $INI when
# set, printf %b escapes) and compares the two outputs byte for byte. Exit 0 = identical.
# Needs $WORK, $CS (the C# Choppy.exe built on the rust branch) and $RS (the Rust choppy.exe); run-all.sh sets them.
name=$1; shift
D="$WORK/data"
run() {
  rm -rf "$D"; mkdir -p "$D"
  if [ -n "$INI" ]; then printf '%b' "$INI" > "$D/settings.ini"; fi
  CHOPPY_DATA="$D" "$1" --plan-dump "$2" "${@:3}"
  echo "exit $?" > "$2.exit"
}
mkdir -p "$WORK/out"; rm -f "$WORK/out/$name".*
run "$CS" "$WORK/out/$name.cs.txt" "$@"
run "$RS" "$WORK/out/$name.rs.txt" "$@"
if cmp -s "$WORK/out/$name.cs.txt" "$WORK/out/$name.rs.txt" && cmp -s "$WORK/out/$name.cs.txt.exit" "$WORK/out/$name.rs.txt.exit"; then
  echo "SAME $name ($(wc -l < "$WORK/out/$name.cs.txt") lines)"
else
  echo "DIFF $name"; diff "$WORK/out/$name.cs.txt" "$WORK/out/$name.rs.txt" | head -"${DIFFN:-40}"; cat "$WORK/out/$name.cs.txt.exit" "$WORK/out/$name.rs.txt.exit"
  exit 1
fi
