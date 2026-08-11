#[path = "../../../../../libraries/rust/a.rs"]
mod a_lib;
mod helper;
use crate::a_lib::helper as _;
use crate::helper::greet;
use std::io::Write;
fn main() {
    greet();
}
