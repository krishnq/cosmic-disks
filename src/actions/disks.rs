// SPDX-License-Identifier: GPL-3.0

//! Disk scanning via the udisks2 D-Bus interface.

use crate::util::{ay_to_string, prop_bool, prop_mount_points, prop_path, prop_str, prop_u64};
use bytesize::ByteSize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DiskError {
    #[error(transparent)]
    Dbus(#[from] zbus::Error),
    #[error(transparent)]
    DbusFdo(#[from] zbus::fdo::Error),
    #[error(transparent)]
    Udisks(#[from] udisks2::Error),
}

const DRIVE_IFACE: &str = "org.freedesktop.UDisks2.Drive";
const BLOCK_IFACE: &str = "org.freedesktop.UDisks2.Block";
const FS_IFACE: &str = "org.freedesktop.UDisks2.Filesystem";
const PARTITION_IFACE: &str = "org.freedesktop.UDisks2.Partition";
const PARTITION_TABLE_IFACE: &str = "org.freedesktop.UDisks2.PartitionTable";

/// Coarse lifecycle state of a block device, derived from udisks2 data at scan time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartitionState {
    /// Filesystem is mounted at one or more paths right now.
    Mounted,
    /// Filesystem is recognised but not currently mounted.
    Unmounted,
    /// No recognised filesystem — raw or unformatted block device.
    Unformatted,
}

/// A matching `/etc/fstab` entry, looked up on demand when a partition is selected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FstabEntry {
    /// The spec field as written in fstab (e.g. `UUID=…`, `LABEL=…`, `/dev/sda1`).
    pub spec: String,
    /// Configured mount point.
    pub mount_point: String,
    /// Configured mount options string.
    pub options: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDevice {
    pub device: String,
    /// Filesystem UUID from udisks2 (`IdUUID`), used for `UUID=` fstab matching.
    pub uuid: String,
    pub label: String,
    pub fs_type: String,
    pub size: u64,
    pub offset: u64,
    pub mount_points: Vec<String>,
    pub drive_id: String,
    /// Coarse lifecycle state derived at scan time.
    pub state: PartitionState,
    /// GPT partition UUID (`Partition.UUID`), used for `PARTUUID=` fstab matching.
    pub part_uuid: String,
    /// GPT partition label (`Partition.Name`), used for `PARTLABEL=` fstab matching.
    pub part_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Drive {
    pub id: String,
    pub model: String,
    pub vendor: String,
    pub size: u64,
    pub removable: bool,
    pub partitions: Vec<BlockDevice>,
    /// udisks2 object path of the whole-disk block device (e.g. `.../block_devices/sda`).
    /// Used for `PartitionTable.CreatePartitionAndFormat`. Empty if unavailable.
    pub block_device: String,
}

impl Drive {
    pub(crate) fn display_name(&self) -> String {
        let name = if !self.vendor.is_empty() && !self.model.is_empty() {
            format!("{} {}", self.vendor.trim(), self.model.trim())
        } else if !self.model.is_empty() {
            self.model.trim().to_string()
        } else {
            "Unknown Drive".to_string()
        };
        format!("{} ({})", name, ByteSize::b(self.size))
    }
}

impl BlockDevice {
    pub(crate) fn display_size(&self) -> String {
        ByteSize::b(self.size).to_string()
    }
}

// ---------------------------------------------------------------------------
// fstab lookup — called on demand when the user selects a partition
// ---------------------------------------------------------------------------

/// Read `/etc/fstab` and return the first entry matching this device by UUID,
/// LABEL, PARTUUID, PARTLABEL, or device path.  Returns `None` if no match or
/// fstab is unreadable.
pub(crate) async fn lookup_fstab(
    uuid: &str,
    label: &str,
    device: &str,
    part_uuid: &str,
    part_label: &str,
) -> Option<FstabEntry> {
    let content = tokio::fs::read_to_string("/etc/fstab").await.ok()?;
    parse_fstab(&content, uuid, label, device, part_uuid, part_label)
}

fn parse_fstab(
    content: &str,
    uuid: &str,
    label: &str,
    device: &str,
    part_uuid: &str,
    part_label: &str,
) -> Option<FstabEntry> {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        let spec = cols[0];
        let matched = (!uuid.is_empty() && spec.eq_ignore_ascii_case(&format!("UUID={uuid}")))
            || (!label.is_empty() && spec.eq_ignore_ascii_case(&format!("LABEL={label}")))
            || (!part_uuid.is_empty()
                && spec.eq_ignore_ascii_case(&format!("PARTUUID={part_uuid}")))
            || (!part_label.is_empty()
                && spec.eq_ignore_ascii_case(&format!("PARTLABEL={part_label}")))
            || spec == device;
        if matched {
            return Some(FstabEntry {
                spec: spec.to_string(),
                mount_point: cols[1].to_string(),
                options: cols[3].to_string(),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Drive scan
// ---------------------------------------------------------------------------

pub(crate) async fn scan_drives() -> Result<Vec<Drive>, DiskError> {
    let client = udisks2::Client::new().await?;
    let objects = client.object_manager().get_managed_objects().await?;

    let mut drives: Vec<Drive> = Vec::new();
    let mut drive_map: HashMap<String, usize> = HashMap::new();

    for (path, interfaces) in &objects {
        let Some(props) = interfaces.get(DRIVE_IFACE) else {
            continue;
        };

        let id = path.to_string();
        let idx = drives.len();
        drives.push(Drive {
            id: id.clone(),
            model: prop_str(props, "Model"),
            vendor: prop_str(props, "Vendor"),
            size: prop_u64(props, "Size"),
            removable: prop_bool(props, "Removable"),
            partitions: Vec::new(),
            block_device: String::new(),
        });
        drive_map.insert(id, idx);
    }

    for (path, interfaces) in &objects {
        let Some(block_props) = interfaces.get(BLOCK_IFACE) else {
            continue;
        };

        let device = block_props
            .get("Device")
            .map(ay_to_string)
            .unwrap_or_default();

        if device.is_empty() || device.starts_with("/dev/loop") {
            continue;
        }

        let has_partition = interfaces.get(PARTITION_IFACE).is_some();
        let has_filesystem = interfaces.get(FS_IFACE).is_some();
        let drive_id = prop_path(block_props, "Drive");

        // Capture the whole-disk block device's object path so the UI can call
        // PartitionTable.CreatePartitionAndFormat on it.
        if interfaces.get(PARTITION_TABLE_IFACE).is_some() && !has_partition {
            if let Some(&idx) = drive_map.get(&drive_id) {
                drives[idx].block_device = path.to_string();
            }
        }

        // Skip the whole-disk block device (e.g. /dev/nvme0n1 alongside its partitions).
        // Keep it only if it has a filesystem directly on it (whole-disk formatted device).
        if !has_partition && !has_filesystem {
            continue;
        }
        let mount_points = interfaces
            .get(FS_IFACE)
            .map(prop_mount_points)
            .unwrap_or_default();
        let offset = interfaces
            .get(PARTITION_IFACE)
            .map(|p| prop_u64(p, "Offset"))
            .unwrap_or(0);
        let part_uuid = interfaces
            .get(PARTITION_IFACE)
            .map(|p| prop_str(p, "UUID"))
            .unwrap_or_default();
        let part_label = interfaces
            .get(PARTITION_IFACE)
            .map(|p| prop_str(p, "Name"))
            .unwrap_or_default();

        let fs_type = prop_str(block_props, "IdType");
        let state = if !mount_points.is_empty() {
            PartitionState::Mounted
        } else if has_filesystem || !fs_type.is_empty() {
            PartitionState::Unmounted
        } else {
            PartitionState::Unformatted
        };

        let block = BlockDevice {
            device,
            uuid: prop_str(block_props, "IdUUID"),
            label: prop_str(block_props, "IdLabel"),
            fs_type,
            size: prop_u64(block_props, "Size"),
            offset,
            mount_points,
            drive_id: drive_id.clone(),
            state,
            part_uuid,
            part_label,
        };

        if let Some(&idx) = drive_map.get(&drive_id) {
            drives[idx].partitions.push(block);
        }
    }

    for drive in &mut drives {
        drive.partitions.sort_by_key(|p| p.offset);
    }
    drives.sort_by(|a, b| a.model.cmp(&b.model));

    Ok(drives)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fstab_matches_uuid() {
        let content = "UUID=abc-123  /mnt/data  ext4  defaults  0  2\n";
        let result = parse_fstab(content, "abc-123", "", "/dev/sda1", "", "");
        let entry = result.expect("should match UUID");
        assert_eq!(entry.mount_point, "/mnt/data");
        assert_eq!(entry.options, "defaults");
        assert_eq!(entry.spec, "UUID=abc-123");
    }

    #[test]
    fn parse_fstab_matches_label() {
        let content = "LABEL=mydata  /mnt/usb  vfat  ro,noatime  0  0\n";
        let result = parse_fstab(content, "", "mydata", "/dev/sdb1", "", "");
        let entry = result.expect("should match LABEL");
        assert_eq!(entry.mount_point, "/mnt/usb");
        assert_eq!(entry.options, "ro,noatime");
    }

    #[test]
    fn parse_fstab_matches_device_path() {
        let content = "/dev/sdc1  /boot  ext4  defaults  0  1\n";
        let result = parse_fstab(content, "", "", "/dev/sdc1", "", "");
        let entry = result.expect("should match device path");
        assert_eq!(entry.mount_point, "/boot");
    }

    #[test]
    fn parse_fstab_no_match() {
        let content = "UUID=other-uuid  /mnt/other  ext4  defaults  0  2\n";
        assert!(parse_fstab(content, "abc-123", "mydata", "/dev/sda1", "", "").is_none());
    }

    #[test]
    fn parse_fstab_skips_comments_and_short_lines() {
        let content =
            "# this is a comment\nUUID=abc-123\n\nUUID=abc-123  /mnt/data  ext4  defaults  0  2\n";
        let result = parse_fstab(content, "abc-123", "", "", "", "");
        assert!(result.is_some());
    }

    #[test]
    fn parse_fstab_matches_partuuid() {
        let content =
            "PARTUUID=11111111-2222-3333-4444-555555555555  /mnt/data  ext4  defaults  0  2\n";
        let result = parse_fstab(
            content,
            "",
            "",
            "/dev/nvme0n1p1",
            "11111111-2222-3333-4444-555555555555",
            "",
        );
        let entry = result.expect("should match PARTUUID");
        assert_eq!(entry.mount_point, "/mnt/data");
    }

    #[test]
    fn parse_fstab_matches_partlabel() {
        let content = "PARTLABEL=mypart  /mnt/data  ext4  defaults  0  2\n";
        let result = parse_fstab(content, "", "", "/dev/nvme0n1p1", "", "mypart");
        let entry = result.expect("should match PARTLABEL");
        assert_eq!(entry.mount_point, "/mnt/data");
    }
}
