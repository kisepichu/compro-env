pub struct Point {
    pub x: i64,
    pub y: i64,
}
pub enum Color {
    Red,
    Green,
    Blue,
}
pub union Bytes {
    a: u32,
    b: [u8; 4],
}
pub trait Monoid {
    type Item;
    const NAME: &'static str;
    fn identity() -> Self;
    fn combine(self, other: Self) -> Self;
}
impl Point {
    pub fn origin() -> Self {
        Self { x: 0, y: 0 }
    }
    pub fn shifted(&self, dx: i64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y,
        }
    }
}
impl Monoid for Point {
    type Item = i64;
    const NAME: &'static str = "Point";
    fn identity() -> Self {
        Self { x: 0, y: 0 }
    }
    fn combine(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }
}
pub fn zero() -> i64 {
    0
}
pub type Coord = (i64, i64);
pub const PI: f64 = 3.14;
pub static NAME: &str = "basic";
macro_rules! shout {
    ($x:expr) => {
        println!("!!! {}", $x)
    };
}
