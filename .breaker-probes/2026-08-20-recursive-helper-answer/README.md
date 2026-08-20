# recursive-helper-answer — a recursive helper computes the resume answer per dispatch

## pyhc1 — sumto(s) = triangular s(s+1)/2 (self-recursive), resume (sumto s) (+ s 1)
Seed (n%3)+1. Body 1000*tick#1 + tick#2.
n=10 s0=2: sumto(2)=3 (s->3), sumto(3)=6 => 3006. n=0 s0=1: sumto(1)=1 (s->2), sumto(2)=3 => 1003.
Verified 3006/1003 x3 + opt-sweep 0-div. Distinct from pyhn1 (helper computes NEXT-STATE, simple 2s+1):
here a RECURSIVE helper computes the ANSWER and must run to completion per dispatch while state threads.

## NOTE: pylc1 (let-chain-before-resume) was DROPPED — a landed pylc1 already covers the let-chain axis
(collision check caught it: 14c line ~16497 "pylc1 probe: arm binds a LET CHAIN ..."). Replaced with pyhc1.
