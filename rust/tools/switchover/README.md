# Step 6: the finished exe's checks

- `compare.py final.json <step 3 reports...>` - the Rust exe's own Benchmark (Thorough + Network) against step 3's
  C# 1.48 numbers (`../speedproof/results/full2.json`, `r5.json`), case by case.
- `mask.py report.json log.txt out-dir name` - drive labels, network paths, the user's name and the report's folder
  taken out before a report goes into this public repository.

`results/`:

| file | what |
|---|---|
| final | the Rust exe's Benchmark: `Choppy.exe --benchmark final.json --thorough --network --drives C,D,F,X,N` in test mode (C, D, F, N, X - step 3's full run's drives; E: left out) |
| hdd5 | the two cases final left in doubt (D>C, X>C mixed), head to head with step 3's harness: the C# from this branch `--benchmark ... --routes "D>C,X>C" --cases mixed --rounds 5 --size-down 1 --rust <rust exe>` |
| final-table.md | `compare.py` on final + step 3's full2 / r5, then `../speedproof/table.py` on hdd5 |

Rerun only with nothing else running (no build, no other test copy): the disks are timed.
