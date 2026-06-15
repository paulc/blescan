pub mod bridge;
pub mod characteristic_data;
pub mod commands;
pub mod dump;
pub mod enumerate;
pub mod event;
pub mod js;
pub mod notify;
pub mod poll;
pub mod scan;
pub mod scanner;
pub mod types;
pub mod util;
pub mod write;
pub mod write_read;

use std::sync::atomic::{AtomicU64, AtomicUsize};

// Default connection parameters
pub static CONNECT_TIMEOUT: AtomicU64 = AtomicU64::new(5);
pub static ENUMERATE_TIMEOUT: AtomicU64 = AtomicU64::new(5);
pub static WRITE_TIMEOUT: AtomicU64 = AtomicU64::new(5);
pub static DISCONNECT_TIMEOUT: AtomicU64 = AtomicU64::new(2);
pub static MAX_TASKS: AtomicUsize = AtomicUsize::new(10);

// Include UUID maps
use crate::util::parse_uuid; // Needed for uuid_map include
include!(concat!(env!("OUT_DIR"), "/uuid_map.rs"));
