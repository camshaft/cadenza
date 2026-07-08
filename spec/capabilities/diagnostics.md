# Capability — Diagnostics

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the machine-readable diagnostics the compiler emits. Requirements realize
> [Core Principle XI](../../constitution.md) and [Core Principle II](../../constitution.md) and
> trace to [overview §13](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete diagnostic record is pinned at the
> declared-default location.

## Purpose And Scope

This capability fixes that every diagnostic the compiler emits is machine-actionable: it carries a
stable code an agent can branch on, a precise span, and a reference to the rule it enforces, and the
diagnostics of a run are emitted in a deterministic order. It states these properties; the concrete
record shape is the declared diagnostics-schema default.

## Diagnostic Content

### Every Diagnostic Has A Stable Code

Every diagnostic the compiler emits MUST carry a machine-readable code that is stable across changes to unrelated diagnostics.

The code a diagnostic carries MUST NOT change when the diagnostic's human-readable wording changes.

### The Code Set Is Pinned Outside The Specification

The set of diagnostic codes and the rejection each code names MUST be pinned at the declared-default location so that two builds emit the same code for the same rejection.

A diagnostic code that an executable-semantics case references MUST resolve to an entry in that pinned code set.

### Every Diagnostic Has A Precise Span

Every diagnostic the compiler emits MUST carry a source span identifying the construct it concerns.

### Every Diagnostic Attributes A Rule

Every diagnostic the compiler emits MUST name the rule or requirement it enforces so that an agent can trace the diagnostic to its cause.

### Every Diagnostic Carries A Severity

Every diagnostic the compiler emits MUST carry a severity that distinguishes an error, which denies a produced component, from a non-error such as a warning, which may accompany a produced component, so that a consumer decides from the diagnostic itself whether the outcome it reports is a failure.

The severity a diagnostic carries MUST be independent of the diagnostic's kind, so that whether an outcome is a failure is read from the severity rather than inferred from whether the outcome is a rejection, a decline, or a trap.

## Determinism

### Diagnostics Are Emitted In A Deterministic Order

The sequence of diagnostics the compiler emits for a program MUST be a deterministic function of the program's source.

## Machine-Readability

### Diagnostics Are Machine-Readable

The compiler MUST expose its diagnostics in a machine-readable form rather than only as human-formatted text.

## A Diagnostic Carries A Route To A Fix

### A Rejection Carries A Structural Fix

A diagnostic that reports a rejection MUST carry a proposed fix expressed as a structural edit of the program's abstract syntax tree, not a textual patch.

### A Confirmed Fix Is Marked Verified

A fix whose application the compiler has confirmed recompiles the program clean and clears the diagnostic MUST be marked verified.

### An Unconfirmed Fix Carries An Applicability Marker

A fix the compiler cannot so confirm MUST carry an applicability marker declaring it a heuristic, so an agent can branch on it.

### A Fix Is A Deterministic Function Of The Source

A proposed fix and its verified-or-heuristic status MUST be a deterministic function of the source.

## Diagnosis Is Complete And Cascade-Aware

### Diagnosis Reports The Maximal Independent Set In One Pass

The compiler MUST recover from an error and report the maximal set of independent problems in one pass rather than only the first.

### A Diagnostic Distinguishes Primary From Derived

The compiler MUST mark each diagnostic as primary or as derived from another, so an agent fixes root causes rather than cascades.

### A Diagnostic Names Its Kind

The compiler MUST expose a machine-branchable kind for each outcome distinguishing a rejection (the program is ill-formed), a decline (the compiler does not yet handle the construct), and a trap (a runtime halt), so an agent routes around compiler limits rather than chasing them.
