// SPDX-License-Identifier: GPL-3.0

//! Volume operations: listing, mounting, and unmounting via udisks2 D-Bus.
//!
//! # Mount strategy
//!
//! `mount_default` — delegates entirely to udisks2 (`Filesystem.Mount` with no
//! explicit mount point). udisks2 chooses `/run/media/$USER/<label>`. No
//! privilege escalation needed for removable devices; polkit handles it.
//!
//! `mount_at_path` — writes a persistent `/etc/fstab` entry via
//! `Block.AddConfigurationItem`, then calls `Filesystem.Mount`. udisks2 owns
//! the fstab write and enforces the `modify-system-configuration` polkit action,
//! which triggers the desktop GUI authentication dialog. Once the entry exists,
//! the subsequent `Filesystem.Mount` is authorised by the fstab record without
//! further prompts.

use crate::util::{ay_to_string, prop_str};
use std::collections::HashMap;
use zbus::proxy::MethodFlags;
use zbus::zvariant::{Array, OwnedObjectPath, OwnedValue, Signature, Value};
use zbus::Connection;

#[derive(Debug, thiserror::Error)]
pub(crate) enum VolumeError {
    #[error("device {device_path} not found in udisks2")]
    DeviceNotFound { device_path: String },
    #[error("mount path must be absolute (got \"{path}\"); use / or ~")]
    PathNotAbsolute { path: String },
    #[error("mount path must not contain \"..\" (got \"{path}\")")]
    PathContainsParentDir { path: String },
    #[error("a mount path is required for a direct mount")]
    MountPathRequired,
    #[error(transparent)]
    Dbus(#[from] zbus::Error),
}

pub(crate) type Result<T> = std::result::Result<T, VolumeError>;

// ---------------------------------------------------------------------------
// Low-level helpers
// ---------------------------------------------------------------------------

/// Resolve the udisks2 D-Bus object path **and** IdUUID for a block device path
/// (e.g. `/dev/sda1`) using an existing connection.
/// Returns `(object_path, uuid)` — uuid may be empty.
async fn resolve_device(conn: &Connection, device_path: &str) -> Result<(String, String)> {
    let objects = conn
        .call_method(
            Some("org.freedesktop.UDisks2"),
            "/org/freedesktop/UDisks2",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await?;

    let map: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        objects.body().deserialize()?;

    for (path, interfaces) in &map {
        if let Some(block_props) = interfaces.get("org.freedesktop.UDisks2.Block") {
            if let Some(dev_val) = block_props.get("Device") {
                // Device property is `ay` (NUL-terminated byte array)
                let dev = ay_to_string(dev_val);
                if dev == device_path {
                    let uuid = prop_str(block_props, "IdUUID");
                    return Ok((path.to_string(), uuid));
                }
            }
        }
    }

    Err(VolumeError::DeviceNotFound {
        device_path: device_path.to_string(),
    })
}

/// Convert a Rust `&str` to a udisks2-style `ay` D-Bus value.
///
/// udisks2 represents all path and string fields in configuration items as
/// NUL-terminated byte arrays (`ay`).  This helper appends the NUL terminator
/// and wraps the bytes in a `zvariant::Value::Array`.
fn str_to_ay(s: &str) -> Value<'static> {
    let mut bytes: Vec<u8> = s.bytes().collect();
    bytes.push(0); // NUL terminator required by udisks2

    // "y" is the D-Bus type signature for a single byte (uint8).
    let sig = Signature::from_bytes(b"y").expect("\"y\" is a valid D-Bus signature");
    let mut arr = Array::new(&sig);
    for b in bytes {
        arr.append(Value::U8(b))
            .expect("invariant: Value::U8 always matches signature \"y\"");
    }
    Value::Array(arr)
}

/// Expand a leading `~` to `$HOME` and verify the result is a non-empty
/// absolute path without `..` components.
fn resolve_path(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(VolumeError::MountPathRequired);
    }

    let expanded = if trimmed == "~" {
        std::env::var("HOME").unwrap_or_else(|_| trimmed.to_string())
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        format!("{home}/{rest}")
    } else {
        trimmed.to_string()
    };

    if !expanded.starts_with('/') {
        return Err(VolumeError::PathNotAbsolute { path: expanded });
    }

    // Reject `..` to prevent traversal sequences ending up in fstab entries.
    if std::path::Path::new(&expanded)
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(VolumeError::PathContainsParentDir { path: expanded });
    }

    Ok(expanded)
}

// ---------------------------------------------------------------------------
// fstab configuration via udisks2
// ---------------------------------------------------------------------------

/// Write a persistent `/etc/fstab` entry for `device` via the udisks2
/// `Block.AddConfigurationItem` D-Bus method.
///
/// udisks2 enforces the `org.freedesktop.udisks2.modify-system-configuration`
/// polkit action, which triggers the desktop GUI authentication dialog.
///
/// **Why `Proxy::call_with_flags` instead of `Connection::call_method`:**
/// `Connection::call_method` never sets the D-Bus
/// `ALLOW_INTERACTIVE_AUTHORIZATION` message flag. Without that flag polkit
/// treats the call as non-interactive and denies it immediately — no dialog
/// appears. `Proxy::call_with_flags` with `MethodFlags::AllowInteractiveAuth`
/// sets the flag, which causes polkit to contact the registered authentication
/// agent and show the GUI password prompt.
///
/// # Arguments
/// - `conn`       — active system D-Bus connection
/// - `obj_path`   — udisks2 D-Bus object path for the block device
/// - `fsname`     — fstab spec field, e.g. `"UUID=abc123"` or `"/dev/sda1"`
/// - `dir`        — absolute mount point path
/// - `options`    — mount options string (e.g. `"ro,noatime"`); `""` → `"defaults"`
async fn add_fstab_entry(
    conn: &Connection,
    obj_path: &str,
    fsname: &str,
    dir: &str,
    options: &str,
) -> Result<()> {
    let opts = if options.trim().is_empty() {
        "defaults"
    } else {
        options.trim()
    };

    // The item type is `(sa{sv})`:
    //   s       = "fstab"  (configuration type tag)
    //   a{sv}   = dict of field name → typed value
    //
    // String fields use `ay` (NUL-terminated byte arrays); numeric fields use `i`.
    let mut dict: HashMap<&str, Value<'_>> = HashMap::new();
    dict.insert("fsname", str_to_ay(fsname));
    dict.insert("dir", str_to_ay(dir));
    dict.insert("type", str_to_ay("auto")); // let the kernel auto-detect
    dict.insert("opts", str_to_ay(opts));
    dict.insert("freq", Value::I32(0));
    dict.insert("passno", Value::I32(0));

    let call_opts: HashMap<&str, Value<'_>> = HashMap::new();
    let body = (("fstab", dict), call_opts);

    // Build a Proxy so we can use call_with_flags to set AllowInteractiveAuth.
    let proxy = zbus::Proxy::new(
        conn,
        "org.freedesktop.UDisks2",
        obj_path,
        "org.freedesktop.UDisks2.Block",
    )
    .await?;

    proxy
        .call_with_flags::<_, _, ()>(
            "AddConfigurationItem",
            MethodFlags::AllowInteractiveAuth.into(),
            &body,
        )
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Mount functions
// ---------------------------------------------------------------------------

/// **Managed mount** — delegates entirely to udisks2 via D-Bus.
///
/// udisks2 chooses the mount point (typically `/run/media/$USER/<label>`).
/// No privilege escalation required for removable devices; polkit handles
/// authorization transparently.
///
/// `options` — comma-separated flags (e.g. `"ro,noatime"`), or `""` for defaults.
pub(crate) async fn mount_default(device: &str, options: &str) -> Result<String> {
    let conn = Connection::system().await?;
    let (obj_path, _) = resolve_device(&conn, device).await?;

    let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
    if !options.trim().is_empty() {
        opts.insert("options", Value::from(options.trim()));
    }

    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        obj_path.as_str(),
        "org.freedesktop.UDisks2.Filesystem",
    )
    .await?;

    let reply: Option<String> = proxy
        .call_with_flags("Mount", MethodFlags::AllowInteractiveAuth.into(), &opts)
        .await?;

    let mount_point = reply.unwrap_or_default();
    Ok(format!("Mounted {device} at {mount_point}"))
}

/// **Mount at an explicit path** — optionally writes a persistent `/etc/fstab`
/// entry via udisks2 `Block.AddConfigurationItem`, then calls `Filesystem.Mount`.
///
/// - `persistent = true` — writes fstab first (requires
///   `org.freedesktop.udisks2.modify-system-configuration` polkit action, which
///   triggers a GUI password dialog), then mounts using the fstab record.
/// - `persistent = false` — passes `"mount-point"` directly in the
///   `Filesystem.Mount` options dict; no fstab write occurs.  polkit still
///   enforces `org.freedesktop.udisks2.filesystem-mount` for the mount call.
///
/// Both paths use `Proxy::call_with_flags(AllowInteractiveAuth)` so the polkit
/// agent can show its GUI dialog.  `Connection::call_method` never sets that flag
/// and would result in a silent polkit denial on interactive sessions.
///
/// - Expands a leading `~` to `$HOME`.
/// - Creates the target directory if it does not exist (best-effort for
///   user-owned paths; fails silently for system paths).
/// - Uses `UUID=<id>` as the fstab spec when the device has a UUID (stable
///   across renames/reboots); falls back to the raw device path otherwise.
/// - `options` — comma-separated flags (e.g. `"ro,noatime"`), or `""` for `"defaults"`.
pub(crate) async fn mount_at_path(
    device: &str,
    mount_path: &str,
    options: &str,
    persistent: bool,
) -> Result<String> {
    let resolved = resolve_path(mount_path)?;

    // Best-effort mkdir for user-owned paths (~/…).
    // For system paths the user must create the directory first, or the mount
    // call will return a clear error.
    let _ = tokio::fs::create_dir_all(&resolved).await;

    let conn = Connection::system().await?;
    let (obj_path, uuid) = resolve_device(&conn, device).await?;

    // Prefer UUID= spec — it remains valid if the device node is renamed.
    let fsname = if uuid.is_empty() {
        device.to_string()
    } else {
        format!("UUID={uuid}")
    };

    if persistent {
        // Write the fstab entry.  udisks2 enforces polkit; the desktop agent
        // shows a GUI password dialog here.
        add_fstab_entry(&conn, &obj_path, &fsname, &resolved, options).await?;
    }

    let mut mount_opts: HashMap<&str, Value<'_>> = HashMap::new();
    if !options.trim().is_empty() {
        mount_opts.insert("options", Value::from(options.trim()));
    }
    if !persistent {
        // No fstab entry — pass the desired path directly in the Mount options.
        mount_opts.insert("mount-point", Value::from(resolved.as_str()));
    }

    let fs_proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        obj_path.as_str(),
        "org.freedesktop.UDisks2.Filesystem",
    )
    .await?;

    let mount_point: Option<String> = fs_proxy
        .call_with_flags(
            "Mount",
            MethodFlags::AllowInteractiveAuth.into(),
            &mount_opts,
        )
        .await?;

    Ok(format!(
        "Mounted {device} at {}",
        mount_point.unwrap_or_default()
    ))
}

// ---------------------------------------------------------------------------
// Unmount / Format
// ---------------------------------------------------------------------------

/// Unmount a block device via udisks2 D-Bus.
pub(crate) async fn unmount(device: &str) -> Result<String> {
    let conn = Connection::system().await?;
    let (path, _) = resolve_device(&conn, device).await?;

    let opts: HashMap<&str, Value<'_>> = HashMap::new();

    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        path.as_str(),
        "org.freedesktop.UDisks2.Filesystem",
    )
    .await?;

    proxy
        .call_with_flags::<_, _, ()>("Unmount", MethodFlags::AllowInteractiveAuth.into(), &opts)
        .await?;

    Ok(format!("Unmounted {device}"))
}

/// Create a new partition and format it via udisks2 `PartitionTable.CreatePartitionAndFormat`.
///
/// `block_device_path` — udisks2 object path of the whole-disk block device
///                       (e.g. `/org/freedesktop/UDisks2/block_devices/sda`).
/// `size_bytes`        — partition size; 0 means "use all available space".
pub(crate) async fn create_partition(
    block_device_path: &str,
    size_bytes: u64,
    fs_type: &str,
    label: &str,
) -> Result<String> {
    let conn = Connection::system().await?;

    let mut format_opts: HashMap<&str, Value<'_>> = HashMap::new();
    if !label.is_empty() {
        format_opts.insert("label", Value::from(label));
    }

    let create_opts: HashMap<&str, Value<'_>> = HashMap::new();

    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        block_device_path,
        "org.freedesktop.UDisks2.PartitionTable",
    )
    .await?;

    // offset=0 lets udisks2 place the partition at the first available gap.
    let _: Option<zbus::zvariant::OwnedObjectPath> = proxy
        .call_with_flags(
            "CreatePartitionAndFormat",
            MethodFlags::AllowInteractiveAuth.into(),
            &(0u64, size_bytes, "", "", create_opts, fs_type, format_opts),
        )
        .await?;

    Ok(format!("Created {fs_type} partition ({size_bytes} bytes)"))
}

/// Format a block device via udisks2 D-Bus.
pub(crate) async fn format(device_path: &str, fs_type: &str, label: &str) -> Result<String> {
    let conn = Connection::system().await?;
    let (path, _) = resolve_device(&conn, device_path).await?;

    let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
    if !label.is_empty() {
        opts.insert("label", Value::from(label));
    }

    let proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UDisks2",
        path.as_str(),
        "org.freedesktop.UDisks2.Block",
    )
    .await?;

    proxy
        .call_with_flags::<_, _, ()>(
            "Format",
            MethodFlags::AllowInteractiveAuth.into(),
            &(fs_type, opts),
        )
        .await?;

    Ok(format!("Formatted {device_path} as {fs_type}"))
}
