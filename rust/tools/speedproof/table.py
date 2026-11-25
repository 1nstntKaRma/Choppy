"""Step 3's table from a three-way Benchmark report (Choppy.exe --benchmark out.json ... --rust choppy.exe).

Usage: python table.py report.json [more.json ...]   (later reports replace a route's case from earlier ones)
Prints per case: Rust, C# 1.48 and Windows (average seconds to the disk), Rust vs C# (how much less time Rust took;
below 0 = more), each one's verdict against Windows (4% = Marginal Tie, as the Benchmark judges), the runs' spread,
and whether Rust vs C# is beyond the noise (more than 4% apart AND outside both sides' run spread).
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


def main(paths):
    cases = {}
    order = []
    for p in paths:
        d = load(p)
        rust = {(r["route"], r["case"]): r for r in d.get("rustResults", [])}
        for r in d["results"]:
            key = (r["route"], r["case"])
            if key not in rust:
                continue
            if key not in cases:
                order.append(key)
            cases[key] = (r, rust[key], p, d.get("rounds"))
    print("| Route | Case | Size | Rust (s) | C# 1.48 (s) | Windows (s) | Rust vs C# | Rust vs Windows | C# vs Windows | Runs |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    beyond = []
    for key in order:
        cs, rs, p, rounds = cases[key]
        if cs.get("error") or rs.get("error"):
            print(f"| {key[0]} | {key[1]} | | error: {rs.get('error') or cs.get('error')} |")
            continue
        c, r, w = cs["choppy"]["seconds"], rs["choppy"]["seconds"], cs["windows"]["seconds"]
        rvc = (c - r) / c
        rvw, cvw = rs["timeSaved"], cs["timeSaved"]
        size = cs["bytes"] / 2**20
        size_s = f"{size / 1024:.0f} GB" if size >= 1024 else f"{size:.0f} MB"
        if cs["files"] > 1:
            size_s += f", {cs['files']} files"
        noise = max(spread(cs["choppy"]["runs"]), spread(rs["choppy"]["runs"]))
        flag = abs(rvc) >= TIE and abs(rvc) > noise / 2
        if flag:
            beyond.append(key)
        print(f"| {key[0].replace(' -> ', '>').replace(':', '')} | {key[1]} | {size_s} | {r:.3f} | {c:.3f} | {w:.3f} | "
              f"{rvc * 100:+.1f}%{' **' if flag else ''} | {verdict(rvw)} {rvw * 100:+.1f}% | {verdict(cvw)} {cvw * 100:+.1f}% | "
              f"{len(rs['choppy']['runs'])} (spread {noise * 100:.0f}%) |")
    newly_slower = [k for k in order if not cases[k][0].get("error") and not cases[k][1].get("error")
                    and verdict(cases[k][1]["timeSaved"]) == "Slower" and verdict(cases[k][0]["timeSaved"]) != "Slower"]
    print()
    print("beyond the noise (Rust vs C#):", ", ".join(f"{a} {b}" for a, b in beyond) or "none")
    print("newly Slower (Rust Slower vs Windows where C# isn't):", ", ".join(f"{a} {b}" for a, b in newly_slower) or "none")


if __name__ == "__main__":
    main(sys.argv[1:])
