| Route | Case | Size | Rust (s) | C# 1.48 (s) | Windows now (s) | Windows then (s) | Rust vs C# 1.48 | Rust vs Windows | C# 1.48 vs Windows | Runs |
|---|---|---|---|---|---|---|---|---|---|---|
| C>C | big | 2 GB | 0.800 | 0.813 | 0.951 | 0.961 | +1.6% | Faster +15.8% | Faster +15.5% | 3 (spread 4%) |
| C>C | small | 190 MB, 1000 files | 0.635 | 0.616 | 3.564 | 3.482 | -3.1% | Faster +82.2% | Faster +82.3% | 3 (spread 20%) |
| C>C | mixed | 588 MB, 403 files | 0.391 | 0.401 | 1.622 | 1.597 | +2.5% | Faster +75.9% | Faster +74.9% | 3 (spread 4%) |
| C>D | big | 1 GB | 4.904 | 4.822 | 4.918 | 4.825 | -1.7% | Tie +0.3% | Tie +0.0% | 3 (spread 3%) |
| C>D | small | 94 MB, 500 files | 2.091 | 2.051 | 3.261 | 3.299 | -2.0% | Faster +35.9% | Faster +37.8% | 3 (spread 41%) |
| C>D | mixed | 588 MB, 403 files | 4.804 | 4.905 | 6.350 | 5.487 | +2.1% | Faster +24.3% | Faster +10.6% | 3 (spread 13%) |
| D>C | big | 1 GB | 5.685 | 5.718 | 5.684 | 5.669 | +0.6% | Tie -0.0% | Tie -0.9% | 3 (spread 4%) |
| D>C | small | 94 MB, 500 files | 2.024 | 2.064 | 5.912 | 5.606 | +1.9% | Faster +65.8% | Faster +63.2% | 3 (spread 3%) |
| D>C | mixed | 588 MB, 403 files | 4.926 | 4.196 | 7.827 | 6.894 | -17.4% ** | Faster +37.1% | Faster +39.1% | 3 (spread 2%) |
| D>D | big | 1 GB | 10.734 | 10.735 | 11.069 | 11.483 | +0.0% | Tie +3.0% | Faster +6.5% | 3 (spread 3%) |
| D>D | small | 94 MB, 500 files | 3.149 | 3.257 | 7.976 | 7.513 | +3.3% | Faster +60.5% | Faster +56.6% | 3 (spread 16%) |
| D>D | mixed | 588 MB, 403 files | 8.093 | 9.551 | 11.432 | 14.001 | +15.3% ** | Faster +29.2% | Faster +31.8% | 3 (spread 5%) |
| C>F | big | 2 GB | 2.382 | 2.395 | 2.972 | 2.999 | +0.5% | Faster +19.8% | Faster +20.2% | 3 (spread 6%) |
| C>F | small | 190 MB, 1000 files | 0.537 | 0.592 | 4.214 | 4.233 | +9.3% | Faster +87.2% | Faster +86.0% | 3 (spread 40%) |
| C>F | mixed | 588 MB, 403 files | 0.914 | 0.843 | 2.510 | 2.501 | -8.4% | Faster +63.6% | Faster +66.3% | 3 (spread 41%) |
| F>C | big | 2 GB | 2.169 | 2.208 | 2.215 | 2.227 | +1.8% | Tie +2.1% | Tie +0.9% | 3 (spread 5%) |
| F>C | small | 190 MB, 1000 files | 0.593 | 0.627 | 3.827 | 3.687 | +5.4% | Faster +84.5% | Faster +83.0% | 3 (spread 26%) |
| F>C | mixed | 588 MB, 403 files | 0.846 | 0.813 | 2.025 | 2.063 | -4.1% | Faster +58.2% | Faster +60.6% | 3 (spread 14%) |
| F>F | big | 2 GB | 2.827 | 2.828 | 4.234 | 4.186 | +0.0% | Faster +33.2% | Faster +32.5% | 3 (spread 9%) |
| F>F | small | 190 MB, 1000 files | 1.623 | 1.587 | 4.617 | 4.462 | -2.3% | Faster +64.8% | Faster +64.4% | 3 (spread 18%) |
| F>F | mixed | 588 MB, 403 files | 1.385 | 1.454 | 2.867 | 2.933 | +4.7% ** | Faster +51.7% | Faster +50.4% | 3 (spread 6%) |
| C>X | big | 1 GB | 5.234 | 5.195 | 5.212 | 5.116 | -0.8% | Tie -0.4% | Tie -1.6% | 3 (spread 9%) |
| C>X | small | 94 MB, 500 files | 2.353 | 2.750 | 3.771 | 3.626 | +14.4% | Faster +37.6% | Faster +24.1% | 3 (spread 31%) |
| C>X | mixed | 588 MB, 403 files | 5.523 | 5.678 | 7.148 | 6.471 | +2.7% | Faster +22.7% | Faster +12.3% | 3 (spread 8%) |
| X>C | big | 1 GB | 5.882 | 5.829 | 5.889 | 5.979 | -0.9% | Tie +0.1% | Tie +2.5% | 3 (spread 2%) |
| X>C | small | 94 MB, 500 files | 2.496 | 2.545 | 8.612 | 8.891 | +1.9% | Faster +71.0% | Faster +71.4% | 3 (spread 2%) |
| X>C | mixed | 588 MB, 403 files | 5.882 | 5.144 | 10.903 | 9.383 | -14.3% ** | Faster +46.1% | Faster +45.2% | 3 (spread 2%) |
| X>X | big | 1 GB | 11.528 | 11.540 | 11.926 | 12.093 | +0.1% | Tie +3.3% | Faster +4.6% | 3 (spread 6%) |
| X>X | small | 94 MB, 500 files | 4.254 | 4.245 | 11.543 | 11.287 | -0.2% | Faster +63.1% | Faster +62.4% | 3 (spread 4%) |
| X>X | mixed | 588 MB, 403 files | 10.311 | 10.260 | 16.950 | 16.536 | -0.5% | Faster +39.2% | Faster +38.0% | 3 (spread 4%) |
| C>N | big | 512 MB | 46.019 | 46.449 | 46.369 | 46.341 | +0.9% | Tie +0.8% | Tie -0.2% | 3 (spread 1%) |
| C>N | small | 50 MB, 250 files | 11.864 | 12.542 | 23.141 | 23.460 | +5.4% ** | Faster +48.7% | Faster +46.5% | 3 (spread 9%) |
| C>N | mixed | 588 MB, 403 files | 61.412 | 62.254 | 81.186 | 82.201 | +1.4% | Faster +24.4% | Faster +24.3% | 3 (spread 1%) |
| N>C | big | 512 MB | 9.993 | 10.189 | 10.431 | 10.348 | +1.9% | Faster +4.2% | Tie +1.5% | 3 (spread 9%) |
| N>C | small | 50 MB, 250 files | 2.307 | 2.449 | 26.691 | 27.130 | +5.8% ** | Faster +91.4% | Faster +91.0% | 3 (spread 8%) |
| N>C | mixed | 588 MB, 403 files | 14.935 | 16.983 | 51.132 | 54.414 | +12.1% ** | Faster +70.8% | Faster +68.8% | 3 (spread 18%) |
| N>N | big | 512 MB | 0.695 | 0.732 | 1.207 | 1.248 | +5.1% ** | Faster +42.4% | Faster +41.3% | 3 (spread 8%) |
| N>N | small | 50 MB, 250 files | 6.351 | 6.443 | 79.507 | 83.385 | +1.4% | Faster +92.0% | Faster +92.3% | 3 (spread 4%) |
| N>N | mixed | 588 MB, 403 files | 9.678 | 9.923 | 127.878 | 133.334 | +2.5% | Faster +92.4% | Faster +92.6% | 3 (spread 3%) |

Rust vs Windows in this run: Faster 31 / Marginal Tie 8 / Slower 0 of 39
the 39 cases added up: Rust 278.2 s, C# 1.48 (step 3) 283.6 s, Windows (this run) 629.5 s
more than 4% from C# 1.48 beyond the runs' spread (**): D>C mixed (-17.4%), D>D mixed (+15.3%), F>F mixed (+4.7%), X>C mixed (-14.3%), C>N small (+5.4%), N>C small (+5.8%), N>C mixed (+12.1%), N>N big (+5.1%)

Head to head (C# 1.48 in its process, Rust in a warm child, Windows; the same files, 5 rounds):

| Route | Case | Size | Rust (s) | C# 1.48 (s) | Windows (s) | Rust vs C# | Rust vs Windows | C# vs Windows | Runs |
|---|---|---|---|---|---|---|---|---|---|
| D>C | mixed | 588 MB, 403 files | 4.791 | 4.845 | 7.759 | +1.1% | Faster +38.2% | Faster +37.6% | 5 (spread 6%) |
| X>C | mixed | 588 MB, 403 files | 5.405 | 5.487 | 9.951 | +1.5% | Faster +45.7% | Faster +44.9% | 5 (spread 2%) |

beyond the noise (Rust vs C#): none
newly Slower (Rust Slower vs Windows where C# isn't): none
