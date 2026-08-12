#pragma once

// Monoid concept shared across the C++ library set.
// A monoid (M, op, id) has an associative operation with a two-sided identity.

#include <concepts>

namespace algebra {

template <class M>
concept Monoid = requires(const typename M::T& a, const typename M::T& b) {
    { M::id() } -> std::convertible_to<typename M::T>;
    { M::op(a, b) } -> std::convertible_to<typename M::T>;
};

struct AddMonoid {
    using T = long long;
    static T id() { return 0; }
    static T op(const T& a, const T& b) { return a + b; }
};

static_assert(Monoid<AddMonoid>);

} // namespace algebra
