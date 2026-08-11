mod b;
#[path = "custom/where.rs"]
mod custom;
use crate::b::helper;
use crate::b::*;
use self::b::helper as also_helper;
use std::collections::{HashMap, BTreeMap as BM};
use serde::Deserialize;
extern crate serde;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(not(unix))]
use std::path::PathBuf;
include!("generated.rs.snippet");
