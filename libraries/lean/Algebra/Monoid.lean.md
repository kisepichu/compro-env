+++
title = "Monoid (Lean)"
+++

## Overview

`Algebra.Monoid` is a bundled structure carrying an identity element, a binary
operation, and proofs of the identity and associativity laws. It is the Lean
counterpart of the Rust `Monoid` trait and the C++ `Monoid` concept.

## Provided instances

- `addMonoid : Monoid Int` — integer addition with proofs of both identity
  sides and associativity.

## Laws

- **Identity:** `op id x = x` and `op x id = x`
- **Associativity:** `op (op a b) c = op a (op b c)`
