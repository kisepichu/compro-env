pub mod outer {
    pub struct Outer;
    pub mod inner {
        pub struct Inner;
        pub fn deep() {}
    }
}
pub fn top() {}
