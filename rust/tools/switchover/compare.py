"""Step 6: the finished Rust exe's own Benchmark against step 3's C# 1.48 numbers.

Usage: python compare.py final.json step3.json [more step3 .json ...]
  final.json  = `Choppy.exe --benchmark final.json --thorough --network ...` of the Rust exe (Choppy in its own process,
                as 1.48's Benchmark runs it)
  step3 .json = step 3's three-way reports (rust/tools/speedproof/results: full2.json, r5.json ...); later ones replace a
                route's case from earlier ones, as table.py does; their `results` are C# 1.48's own runs.
Prints per case: Rust now, C# 1.48 then and Windows then and now (average seconds to the disk), how much less time
Rust took than C# 1.48 (below 0 = more), each one's verdict against Windows in its own run (4% = Marginal Tie), and
which cases are more than 4% apart from C# 1.48 beyond the runs' spread.
"""
import json
import sys

TIE = 0.04


def load(path):
    with open(path, encoding="utf-8-sig") as f:
        return json.load(f)


def verdict(saved):
    return "Faster" if saved >= TIE else "Slower" if saved <= -TIE else "Tie"


def spread(runs):
    xs = [r["onDisk"] for r in runs]
    return (max(xs) - min(xs)) / (sum(xs) / len(xs)) if len(xs) > 1 else 0.0


def main(final, step3):
    then = {}
    for p in step3:
        for r in load(p)["results"]:
            if not r.get("error"):
                then[(r["route"], r["case"])] = r
    now = load(final)
    print("| Route | Case | Size | Rust (s) | C# 1.48 (s) | Windows now (s) | Windows then (s) | Rust vs C# 1.48 "
          "| Rust vs Windows | C# 1.48 vs Windows | Runs |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    beyond, rows, verdicts, totals = [], 0, {"Faster": 0, "Tie": 0, "Slower": 0}, [0.0, 0.0, 0.0]
    for r in now["results"]:
        key = (r["route"], r["case"])
        route = key[0].replace(" -> ", ">").replace(":", "")
        if r.get("error"):
            print(f"| {route} | {key[1]} | | error: {r['error']} |")
            continue
        rust, win = r["choppy"]["seconds"], r["windows"]["seconds"]
        size = r["bytes"] / 2**20
        size_s = f"{size / 1024:.0f} GB" if size >= 1024 else f"{size:.0f} MB"
        if r["files"] > 1:
            size_s += f", {r['files']} files"
        v = verdict(r["timeSaved"])
        verdicts[v] += 1
        c = then.get(key)
        if not c:
            print(f"| {route} | {key[1]} | {size_s} | {rust:.3f} | - | {win:.3f} | - | - | {v} {r['timeSaved'] * 100:+.1f}% "
                  f"| - | {len(r['choppy']['runs'])} |")
            continue
        cs, cw = c["choppy"]["seconds"], c["windows"]["seconds"]
        rvc = (cs - rust) / cs
        noise = max(spread(c["choppy"]["runs"]), spread(r["choppy"]["runs"]))
        flag = abs(rvc) >= TIE and abs(rvc) > noise / 2
        if flag:
            beyond.append(f"{route} {key[1]} ({rvc * 100:+.1f}%)")
        same_size = c["bytes"] == r["bytes"] and c["files"] == r["files"]
        rows += 1
        totals[0] += rust
        totals[1] += cs
        totals[2] += win
        print(f"| {route} | {key[1]} | {size_s}{'' if same_size else ' (sizes differ)'} | {rust:.3f} | {cs:.3f} | {win:.3f} "
              f"| {cw:.3f} | {rvc * 100:+.1f}%{' **' if flag else ''} | {v} {r['timeSaved'] * 100:+.1f}% "
              f"| {verdict(c['timeSaved'])} {c['timeSaved'] * 100:+.1f}% | {len(r['choppy']['runs'])} "
              f"(spread {noise * 100:.0f}%) |")
    print()
    print(f"Rust vs Windows in this run: Faster {verdicts['Faster']} / Marginal Tie {verdicts['Tie']} / "
          f"Slower {verdicts['Slower']} of {sum(verdicts.values())}")
    if rows:
        print(f"the {rows} cases added up: Rust {totals[0]:.1f} s, C# 1.48 (step 3) {totals[1]:.1f} s, "
              f"Windows (this run) {totals[2]:.1f} s")
    print("more than 4% from C# 1.48 beyond the runs' spread (**):", ", ".join(beyond) or "none")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2:])
