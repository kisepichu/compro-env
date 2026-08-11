#ifndef CE_FIXTURE_SYMBOLS_BASIC_HPP
#define CE_FIXTURE_SYMBOLS_BASIC_HPP
namespace algebra {
struct Point {
    long long x;
    long long y;
    Point();
    Point(long long x_, long long y_);
    ~Point();
    Point shifted(long long dx) const;
    static Point origin();
    Point operator+(const Point& other) const;
};
class Color {
   public:
    enum Kind { Red, Green, Blue };
    Kind kind;
};
union Bytes {
    unsigned int as_uint;
    unsigned char as_bytes[4];
};
enum class Signal { Low, High };
using Coord = long long;
typedef long long Weight;
template <class T>
concept Addable = requires(T a, T b) { a + b; };
template <class T>
struct Optional {
    bool has;
    T value;
};
template <class T>
T identity();
long long zero();
long long zero(int);
constexpr long long PI = 3;
}  // namespace algebra
#endif  // CE_FIXTURE_SYMBOLS_BASIC_HPP
