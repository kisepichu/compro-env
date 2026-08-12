+++
title = "Monoid (Rust)"
+++

## Overview

`Monoid` describes an associative binary operation with a two-sided identity.
It is the smallest shared abstraction used by segment tree, prefix-sum, and
similar data structures across the Rust library set.

## Provided instances

- `AddMonoid` for `i64` integer addition.

## Laws

- **Identity:** `op(id, x) = op(x, id) = x`
- **Associativity:** `op(op(a, b), c) = op(a, op(b, c))`
