# [Feature Name]

[One or two sentences on what this feature is and which capability's behavioral requirements its
cases witness. Cases are shown in a display of the canonical representation; the same case in any
other display denotes the same representation and executes identically.]

## [Group Name]

### Case: [a short description of what this case shows]

**Input:**

```cadenza
<a program in the canonical display>
```

**Output:**

```
<the exact output its execution produces, or a diagnostic code for a rejected program>
```

**Notes:** [optional — which requirement this witnesses, or a subtlety worth stating.]

<!--
  AUTHORING RULES (delete before finalizing):
  - A case is EXECUTABLE and has a DEFINITE output; a case with no definite output is not a case.
  - A case covers ONE behavior; prefer many small cases so a behavior-gate failure names the
    construct that broke.
  - Output is byte-exact and deterministic; serialize values under the canonical value form.
  - Every behavioral requirement in the corresponding capability spec is witnessed by at least one
    case here.
-->
