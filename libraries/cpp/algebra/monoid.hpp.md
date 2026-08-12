+++
title = "Monoid (C++)"
+++

## Overview

The `algebra::Monoid` concept requires an inner type `T`, an identity
`M::id()`, and an associative binary operation `M::op(a, b)`. Header-only
concept intended for segment trees and other associative aggregations.

## Provided instances

- `AddMonoid` for `long long` addition, checked with `static_assert`.

## Laws

- **Identity:** `M::op(M::id(), x) == M::op(x, M::id()) == x`
- **Associativity:** `M::op(M::op(a, b), c) == M::op(a, M::op(b, c))`
