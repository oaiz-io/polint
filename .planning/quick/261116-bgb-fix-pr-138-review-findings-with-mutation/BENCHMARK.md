# JS benchmark validation

`make js-bench` passed (exit 0). Overall wall time: 163.368 s; maximum child peak RSS: 5,645,402,112 bytes. Release builds used the unchanged final source hashes in `SOURCE-SHA256.txt`.

Every JSON field except the top-level `cost` and `polint_version` exactly matches the committed PR-head report; the saved pre-fix baseline was also checked against `git show HEAD:tests/js-analysis-bench/baseline/report.json`. This includes accuracy, unresolved-call counts, false-complete labels, per-case results, package outcomes, decisions, and gap ranking.

The run used local macOS ARM64 while the prepared Linux CI validation ran concurrently. Historical AX102 samples have different machine and operating-system conditions; these samples do not establish a cross-machine speed or memory ratio.

| Package | Base dump ms | Base peak RSS bytes | Refined dump ms | Refined peak RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| async | 381 | 124715008 | 1906 | 502906880 |
| axios | 564 | 161316864 | 2017 | 466092032 |
| chalk | 36 | 27213824 | 80 | 43843584 |
| commander | 209 | 78102528 | 1423 | 242925568 |
| dayjs | 37 | 26722304 | 98 | 32538624 |
| underscore | 181 | 70123520 | 808 | 180256768 |
| zod | 8401 | 1791967232 | 35254 | 5645402112 |

The fresh report updates only measured cost/version content in `tests/js-analysis-bench/baseline/report.{json,md}`. The preceding timing attempt produced a report, but the macOS `/usr/bin/time -l` wrapper exited 1 because `sysctl kern.clockrate` was denied. The subsequent clean make run passed; its measurements above are the final samples.

## Release microbenchmarks

100 runs over 20,000 sites each:

| Workload | Time for 100 runs | Unused expensive index builds |
| --- | ---: | ---: |
| Completeness, empty refined edges | 0.256083 ms | 0 |
| Completeness, fully resolved edges | 5.801667 ms | 0 |
| TS refinement, empty unresolved set | 0.028000 ms | 0 |
| TS refinement, Go-only unresolved calls | 1.504333 ms | 0 |

The microbenchmarks are opt-in release tests, not timing assertions. The regression tests assert zero index builds on these inactive paths and one shared index on the active completeness path.
