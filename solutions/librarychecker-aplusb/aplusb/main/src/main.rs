use std::cell::RefCell;

thread_local! {
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
}

trait OutValue {
    fn write_out(&self, dst: &mut String);
}

macro_rules! impl_out_value_display {
    ($($ty:ty),* $(,)?) => {
        $(
            impl OutValue for $ty {
                fn write_out(&self, dst: &mut String) {
                    use std::fmt::Write as _;
                    write!(dst, "{}", self).unwrap();
                }
            }
        )*
    };
}

impl_out_value_display!(
    bool, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64,
);

impl OutValue for str {
    fn write_out(&self, dst: &mut String) {
        dst.push_str(self);
    }
}

impl OutValue for String {
    fn write_out(&self, dst: &mut String) {
        dst.push_str(self);
    }
}

impl<T: OutValue + ?Sized> OutValue for &T {
    fn write_out(&self, dst: &mut String) {
        (*self).write_out(dst);
    }
}

fn write_separated<'a, I, T>(iter: I, dst: &mut String)
where
    I: IntoIterator<Item = &'a T>,
    T: OutValue + 'a,
{
    let mut iter = iter.into_iter();
    if let Some(first) = iter.next() {
        first.write_out(dst);
        for value in iter {
            dst.push(' ');
            value.write_out(dst);
        }
    }
}

impl<T: OutValue> OutValue for [T] {
    fn write_out(&self, dst: &mut String) {
        write_separated(self.iter(), dst);
    }
}

impl<T: OutValue, const N: usize> OutValue for [T; N] {
    fn write_out(&self, dst: &mut String) {
        self.as_slice().write_out(dst);
    }
}

impl<T: OutValue> OutValue for Vec<T> {
    fn write_out(&self, dst: &mut String) {
        self.as_slice().write_out(dst);
    }
}

impl<T: OutValue> OutValue for std::collections::VecDeque<T> {
    fn write_out(&self, dst: &mut String) {
        write_separated(self.iter(), dst);
    }
}

impl<T: OutValue> OutValue for std::collections::BTreeSet<T> {
    fn write_out(&self, dst: &mut String) {
        write_separated(self.iter(), dst);
    }
}

impl<A: OutValue, B: OutValue> OutValue for (A, B) {
    fn write_out(&self, dst: &mut String) {
        self.0.write_out(dst);
        dst.push(' ');
        self.1.write_out(dst);
    }
}

impl<A: OutValue, B: OutValue, C: OutValue> OutValue for (A, B, C) {
    fn write_out(&self, dst: &mut String) {
        self.0.write_out(dst);
        dst.push(' ');
        self.1.write_out(dst);
        dst.push(' ');
        self.2.write_out(dst);
    }
}

fn out_clear() {
    OUTPUT.with(|out| out.borrow_mut().clear());
}

fn out_take() -> String {
    OUTPUT.with(|out| std::mem::take(&mut *out.borrow_mut()))
}

fn out_write<T: OutValue + ?Sized>(value: &T) {
    OUTPUT.with(|out| value.write_out(&mut out.borrow_mut()));
}

fn out_space() {
    OUTPUT.with(|out| out.borrow_mut().push(' '));
}

fn out_newline() {
    OUTPUT.with(|out| out.borrow_mut().push('\n'));
}

fn with_output(f: impl FnOnce()) -> String {
    out_clear();
    f();
    out_take()
}

macro_rules! out {
    () => {{
        out_newline();
    }};
    ($first:expr $(, $rest:expr)* $(,)?) => {{
        out_write(&$first);
        $(
            out_space();
            out_write(&$rest);
        )*
        out_newline();
    }};
}

use proconio::input;

fn solve(a: i64, b: i64) -> String {
    with_output(|| out!(a + b))
}

fn main() {
    input! {
    a: i64,
    b: i64,
    }
    print!("{}", solve(a, b));
}

#[cfg(test)]
mod tests {
    use super::*;

    // ↓ Add test cases here
    const CASES: &[(&str, &str)] = &[
        ("1234 5678\n", "6912\n"),
        ("1000000000 1000000000\n", "2000000000\n"),
    ];

    #[test]
    fn test_samples() {
        for (i, &(input, expected)) in CASES.iter().enumerate() {
            let result = {
                use proconio::input;
                input! {
                from proconio::source::once::OnceSource::from(input),
                a: i64,
                b: i64,
                }
                solve(a, b)
            };
            assert_eq!(
                result.trim(),
                expected.trim(),
                "case {}\n--- input ---\n{}",
                i + 1,
                input
            );
        }
    }
}
