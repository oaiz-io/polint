# Expected-move allowlists

One file per workstream commit whose oracle expects something to move. `gate.sh`
takes one through `POLINT_GATE_ALLOW` and hands it to `factrows.py`; both read
the same file.

```
# a fact family that may differ, and how
family <FamilyLabel> <absent|any|summary-column2|i1b-line-rule>
# a provider output digest that may differ (I1a)
digest <provider.id>
```

A family or provider not named here may not move. The rules:

| rule | meaning |
|---|---|
| `absent` | the family may lose every row; a family that still has rows must match |
| `any` | the family may differ freely; the delta is counted and reported, never judged |
| `summary-column2` | columns 1, 3 and 4 identical on every row, column 2 free (W3 commit 0) |
| `i1b-line-rule` | `summary_control` key set identical, every differing line's parts column moving only as the I1b line rule permits, attributes column unchanged (W3 commit 2) |

A file here is written by the commit whose oracle needs it and is not shared
between workstreams: an allowlist that accumulates entries stops being an
oracle.
