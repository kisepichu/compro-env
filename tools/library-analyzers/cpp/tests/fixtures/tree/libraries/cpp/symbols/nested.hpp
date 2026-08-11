#ifndef CE_FIXTURE_SYMBOLS_NESTED_HPP
#define CE_FIXTURE_SYMBOLS_NESTED_HPP
namespace outer {
struct Outer {
    int value;
};
namespace inner {
struct Inner {
    int value;
};
long long deep();
}  // namespace inner
namespace {
struct Hidden {
    int flag;
};
}  // namespace
}  // namespace outer
long long top();
#endif  // CE_FIXTURE_SYMBOLS_NESTED_HPP
