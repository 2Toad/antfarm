use serde::{Deserialize, Serialize};

use crate::model::{Machine, Time};

/// The whole model at one instant. Cheap to share: front ends hold it behind an `Arc`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub seq: u64,
    pub at: Time,
    pub machine: Machine,
}
