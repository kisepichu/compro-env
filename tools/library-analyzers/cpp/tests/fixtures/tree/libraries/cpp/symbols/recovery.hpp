#ifndef CE_FIXTURE_SYMBOLS_RECOVERY_HPP
#define CE_FIXTURE_SYMBOLS_RECOVERY_HPP
struct Before {
    int value;
};
long long before_fn();
struct MidBroken {
    int value
};
long long after_fn();
#endif  // CE_FIXTURE_SYMBOLS_RECOVERY_HPP
