// SPDX-License-Identifier: GPL-3.0

use std::collections::HashMap;
use zbus::zvariant::{Array, ObjectPath, OwnedValue, Value};

pub(crate) fn prop_str(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok().map(String::from))
        .unwrap_or_default()
}

pub(crate) fn prop_u64(props: &HashMap<String, OwnedValue>, key: &str) -> u64 {
    props
        .get(key)
        .and_then(|v| u64::try_from(v).ok())
        .unwrap_or(0)
}

pub(crate) fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> bool {
    props
        .get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

/// Decode a D-Bus `ay` (NUL-terminated byte array) `OwnedValue` to a `String`.
pub(crate) fn ay_to_string(v: &OwnedValue) -> String {
    let Ok(arr) = <&Array>::try_from(v) else {
        return String::new();
    };
    let bytes: Vec<u8> = arr
        .iter()
        .filter_map(|item| if let Value::U8(b) = item { Some(*b) } else { None })
        .collect();
    String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .to_string()
}

pub(crate) fn prop_path(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| <&ObjectPath>::try_from(v).ok().map(|p| p.to_string()))
        .unwrap_or_default()
}

pub(crate) fn prop_mount_points(props: &HashMap<String, OwnedValue>) -> Vec<String> {
    let Some(v) = props.get("MountPoints") else {
        return Vec::new();
    };
    let Ok(outer) = <&Array>::try_from(v) else {
        return Vec::new();
    };
    outer
        .iter()
        .filter_map(|item| {
            if let Value::Array(inner) = item {
                let bytes: Vec<u8> = inner
                    .iter()
                    .filter_map(|b| if let Value::U8(byte) = b { Some(*byte) } else { None })
                    .collect();
                let s = String::from_utf8_lossy(&bytes)
                    .trim_end_matches('\0')
                    .to_string();
                if s.is_empty() { None } else { Some(s) }
            } else {
                None
            }
        })
        .collect()
}
