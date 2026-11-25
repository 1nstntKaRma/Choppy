"""Masks a Benchmark report and its log before they go into the (public) repository, as step 3's results/ are:
drive labels blanked, network paths made generic (\\\\nas\\NAS), the user's name taken out of paths.

Usage: python mask.py report.json log.txt out-dir [name]   -> out-dir/name.json and out-dir/name-log.txt
"""
import json
import os
import re
import sys

UNC = re.compile(r"\\\\[^\\\s\"|]+\\[^\\\s\"|]+")
FOLDER = ""


def clean(text, labels, user):
    # the folder the report was saved in (its last line names the HTML there), then network paths, to a mark a label
    # (like "NAS") can't touch
    if FOLDER:
        text = re.sub(re.escape(FOLDER), "<folder>", text, flags=re.IGNORECASE)
    text = UNC.sub("\x00", text)
    for label in labels:
        text = text.replace(label, "")
    if user:
        text = re.sub(re.escape(user), "user", text, flags=re.IGNORECASE)
    return text.replace("\x00", r"\\nas\NAS")


def walk(v, labels, user):
    if isinstance(v, str):
        return clean(v, labels, user)
    if isinstance(v, list):
        return [walk(x, labels, user) for x in v]
    if isinstance(v, dict):
        return {k: walk(x, labels, user) for k, x in v.items()}
    return v


def main(report, log, out, name="final"):
    global FOLDER
    FOLDER = os.path.dirname(os.path.abspath(report))
    with open(report, encoding="utf-8-sig") as f:
        d = json.load(f)
    labels = sorted({x["label"] for x in d.get("drives", []) if x.get("label")}, key=len, reverse=True)
    user = os.environ.get("USERNAME", "")
    for x in d.get("drives", []):
        x["label"] = ""
    d = walk(d, labels, user)
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, name + ".json"), "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(d, ensure_ascii=False, indent=2) + "\n")
    with open(log, encoding="utf-8-sig") as f:
        lines = f.read()
    with open(os.path.join(out, name + "-log.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write(clean(lines, labels, user))
    print("masked:", ", ".join(labels) or "(no labels)", "+ network paths + the user's name")


if __name__ == "__main__":
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
