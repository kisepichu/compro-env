#ifndef CE_FIXTURE_SYMBOLS_MACROS_HPP
#define CE_FIXTURE_SYMBOLS_MACROS_HPP
#define CE_DECLARE_STRUCT(Name)   \
    struct Name {                 \
        int value;                \
    }
struct Real {
    int value;
};
CE_DECLARE_STRUCT(FromMacro);
#endif  // CE_FIXTURE_SYMBOLS_MACROS_HPP
