// SPDX-License-Identifier: GPL-3.0

use cosmic_config::CosmicConfigEntry;
use serde::{Deserialize, Serialize};

use crate::message::FsType;

pub const CONFIG_VERSION: u64 = 1;

/// Persisted user preferences. Fields are stored individually by key name in cosmic-config.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[derive(cosmic_config::cosmic_config_derive::CosmicConfigEntry)]
#[serde(default)]
pub struct Config {
    pub default_fs_type: FsType,
}
