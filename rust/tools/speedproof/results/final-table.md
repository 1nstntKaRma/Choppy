| Route | Case | Size | Rust (s) | C# 1.48 (s) | Windows (s) | Rust vs C# | Rust vs Windows | C# vs Windows | Runs |
|---|---|---|---|---|---|---|---|---|---|
| C>C | big | 2 GB | 0.814 | 0.813 | 0.961 | -0.1% | Faster +15.4% | Faster +15.5% | 3 (spread 7%) |
| C>C | small | 190 MB, 1000 files | 0.645 | 0.616 | 3.482 | -4.7% | Faster +81.5% | Faster +82.3% | 3 (spread 30%) |
| C>C | mixed | 588 MB, 403 files | 0.395 | 0.401 | 1.597 | +1.5% | Faster +75.3% | Faster +74.9% | 3 (spread 4%) |
| C>D | big | 1 GB | 4.780 | 4.822 | 4.825 | +0.9% | Tie +0.9% | Tie +0.0% | 5 (spread 4%) |
| C>D | small | 94 MB, 500 files | 1.978 | 2.051 | 3.299 | +3.6% | Faster +40.0% | Faster +37.8% | 5 (spread 41%) |
| C>D | mixed | 588 MB, 403 files | 4.806 | 4.905 | 5.487 | +2.0% | Faster +12.4% | Faster +10.6% | 5 (spread 25%) |
| D>C | big | 1 GB | 5.634 | 5.718 | 5.669 | +1.5% | Tie +0.6% | Tie -0.9% | 3 (spread 4%) |
| D>C | small | 94 MB, 500 files | 2.037 | 2.064 | 5.606 | +1.3% | Faster +63.7% | Faster +63.2% | 3 (spread 7%) |
| D>C | mixed | 588 MB, 403 files | 4.155 | 4.196 | 6.894 | +1.0% | Faster +39.7% | Faster +39.1% | 3 (spread 1%) |
| D>D | big | 1 GB | 10.778 | 10.735 | 11.483 | -0.4% | Faster +6.1% | Faster +6.5% | 3 (spread 1%) |
| D>D | small | 94 MB, 500 files | 3.210 | 3.257 | 7.513 | +1.4% | Faster +57.3% | Faster +56.6% | 3 (spread 16%) |
| D>D | mixed | 588 MB, 403 files | 9.165 | 9.551 | 14.001 | +4.0% | Faster +34.5% | Faster +31.8% | 3 (spread 10%) |
| C>F | big | 2 GB | 2.399 | 2.395 | 2.999 | -0.2% | Faster +20.0% | Faster +20.2% | 3 (spread 6%) |
| C>F | small | 190 MB, 1000 files | 0.540 | 0.592 | 4.233 | +8.8% | Faster +87.2% | Faster +86.0% | 3 (spread 21%) |
| C>F | mixed | 588 MB, 403 files | 0.864 | 0.843 | 2.501 | -2.5% | Faster +65.5% | Faster +66.3% | 3 (spread 14%) |
| F>C | big | 2 GB | 2.176 | 2.208 | 2.227 | +1.4% | Tie +2.3% | Tie +0.9% | 3 (spread 5%) |
| F>C | small | 190 MB, 1000 files | 0.643 | 0.627 | 3.687 | -2.6% | Faster +82.5% | Faster +83.0% | 3 (spread 26%) |
| F>C | mixed | 588 MB, 403 files | 0.827 | 0.813 | 2.063 | -1.7% | Faster +59.9% | Faster +60.6% | 3 (spread 1%) |
| F>F | big | 2 GB | 2.740 | 2.828 | 4.186 | +3.1% | Faster +34.5% | Faster +32.5% | 3 (spread 9%) |
| F>F | small | 190 MB, 1000 files | 1.653 | 1.587 | 4.462 | -4.2% | Faster +62.9% | Faster +64.4% | 3 (spread 16%) |
| F>F | mixed | 588 MB, 403 files | 1.459 | 1.454 | 2.933 | -0.3% | Faster +50.2% | Faster +50.4% | 3 (spread 8%) |
| C>X | big | 1 GB | 5.222 | 5.195 | 5.116 | -0.5% | Tie -2.1% | Tie -1.6% | 5 (spread 10%) |
| C>X | small | 94 MB, 500 files | 2.901 | 2.750 | 3.626 | -5.5% | Faster +20.0% | Faster +24.1% | 5 (spread 31%) |
| C>X | mixed | 588 MB, 403 files | 5.866 | 5.678 | 6.471 | -3.3% | Faster +9.3% | Faster +12.3% | 5 (spread 8%) |
| X>C | big | 1 GB | 5.946 | 5.829 | 5.979 | -2.0% | Tie +0.6% | Tie +2.5% | 3 (spread 3%) |
| X>C | small | 94 MB, 500 files | 2.472 | 2.545 | 8.891 | +2.9% | Faster +72.2% | Faster +71.4% | 3 (spread 7%) |
| X>C | mixed | 588 MB, 403 files | 5.153 | 5.144 | 9.383 | -0.2% | Faster +45.1% | Faster +45.2% | 3 (spread 2%) |
| X>X | big | 1 GB | 11.612 | 11.540 | 12.093 | -0.6% | Tie +4.0% | Faster +4.6% | 3 (spread 3%) |
| X>X | small | 94 MB, 500 files | 4.409 | 4.245 | 11.287 | -3.9% | Faster +60.9% | Faster +62.4% | 3 (spread 5%) |
| X>X | mixed | 588 MB, 403 files | 10.497 | 10.260 | 16.536 | -2.3% | Faster +36.5% | Faster +38.0% | 3 (spread 6%) |
| C>N | big | 512 MB | 46.175 | 46.449 | 46.341 | +0.6% | Tie +0.4% | Tie -0.2% | 3 (spread 1%) |
| C>N | small | 50 MB, 250 files | 12.622 | 12.542 | 23.460 | -0.6% | Faster +46.2% | Faster +46.5% | 3 (spread 11%) |
| C>N | mixed | 588 MB, 403 files | 61.314 | 62.254 | 82.201 | +1.5% | Faster +25.4% | Faster +24.3% | 3 (spread 1%) |
| N>C | big | 512 MB | 10.160 | 10.189 | 10.348 | +0.3% | Tie +1.8% | Tie +1.5% | 5 (spread 24%) |
| N>C | small | 50 MB, 250 files | 2.408 | 2.449 | 27.130 | +1.7% | Faster +91.1% | Faster +91.0% | 5 (spread 10%) |
| N>C | mixed | 588 MB, 403 files | 17.459 | 16.983 | 54.414 | -2.8% | Faster +67.9% | Faster +68.8% | 5 (spread 9%) |
| N>N | big | 512 MB | 0.725 | 0.732 | 1.248 | +1.0% | Faster +41.9% | Faster +41.3% | 3 (spread 6%) |
| N>N | small | 50 MB, 250 files | 6.630 | 6.443 | 83.385 | -2.9% | Faster +92.0% | Faster +92.3% | 3 (spread 12%) |
| N>N | mixed | 588 MB, 403 files | 9.662 | 9.923 | 133.334 | +2.6% | Faster +92.8% | Faster +92.6% | 3 (spread 5%) |
| C>E | big | 2 GB | 2.513 | 2.536 | 3.009 | +0.9% | Faster +16.5% | Faster +15.7% | 3 (spread 1%) |
| C>E | small | 190 MB, 1000 files | 1.419 | 1.478 | 4.177 | +4.0% | Faster +66.0% | Faster +64.6% | 5 (spread 23%) |
| C>E | mixed | 588 MB, 403 files | 1.340 | 1.320 | 2.576 | -1.5% | Faster +48.0% | Faster +48.8% | 5 (spread 22%) |
| E>C | big | 2 GB | 2.100 | 2.115 | 2.182 | +0.7% | Tie +3.8% | Tie +3.1% | 3 (spread 3%) |
| E>C | small | 190 MB, 1000 files | 1.274 | 1.296 | 3.637 | +1.7% | Faster +65.0% | Faster +64.4% | 5 (spread 11%) |
| E>C | mixed | 588 MB, 403 files | 1.097 | 1.123 | 2.037 | +2.3% | Faster +46.1% | Faster +44.9% | 5 (spread 9%) |
| E>E | big | 2 GB | 4.606 | 4.627 | 5.410 | +0.5% | Faster +14.9% | Faster +14.5% | 3 (spread 1%) |
| E>E | small | 190 MB, 1000 files | 1.638 | 1.617 | 4.482 | -1.3% | Faster +63.4% | Faster +63.9% | 3 (spread 10%) |
| E>E | mixed | 588 MB, 403 files | 1.928 | 1.887 | 3.455 | -2.2% | Faster +44.2% | Faster +45.4% | 3 (spread 8%) |

beyond the noise (Rust vs C#): none
newly Slower (Rust Slower vs Windows where C# isn't): none
