// SPDX-License-Identifier: GPL-3.0

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::actions::disks::{BlockDevice, Drive, FstabEntry};

// ---------------------------------------------------------------------------
// Top-level state machines
// ---------------------------------------------------------------------------

#[derive(Default)]
pub enum LoadState {
    #[default]
    Scanning,
    Error(String),
    Ready(Vec<Drive>),
}

#[derive(Default)]
pub enum ActiveDialog {
    #[default]
    None,
    Mount(MountLocationDialog),
    Unmount { device: String, mount_points: Vec<String> },
}

pub struct PartitionContext {
    pub partition: BlockDevice,
    pub fstab_entry: Option<FstabEntry>,
    pub format_panel: Option<FormatPanel>,
    pub operation_in_progress: Option<OperationKind>,
    pub operation_error: Option<String>,
}

pub struct UnallocatedContext {
    pub drive_id: String,
    pub create_panel: Option<CreatePartitionPanel>,
    pub operation_in_progress: Option<OperationKind>,
    pub operation_error: Option<String>,
}

#[derive(Default)]
pub enum SelectionState {
    #[default]
    None,
    Partition(PartitionContext),
    Unallocated(UnallocatedContext),
}

impl SelectionState {
    /// Returns `true` when the selected partition already has an `/etc/fstab` entry loaded.
    pub fn has_fstab_entry(&self) -> bool {
        if let SelectionState::Partition(ctx) = self {
            ctx.fstab_entry.is_some()
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MountFlag {
    ReadOnly,
    NoExec,
    NoSuid,
    NoAtime,
    NoDiratime,
    Sync,
}

impl MountFlag {
    pub const ALL: [MountFlag; 6] = [
        MountFlag::ReadOnly,
        MountFlag::NoExec,
        MountFlag::NoSuid,
        MountFlag::NoAtime,
        MountFlag::NoDiratime,
        MountFlag::Sync,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MountFlag::ReadOnly  => "ro",
            MountFlag::NoExec    => "noexec",
            MountFlag::NoSuid    => "nosuid",
            MountFlag::NoAtime   => "noatime",
            MountFlag::NoDiratime => "nodiratime",
            MountFlag::Sync      => "sync",
        }
    }

    pub fn label(self) -> String {
        match self {
            MountFlag::ReadOnly   => crate::fl!("mount-flag-ro"),
            MountFlag::NoExec     => crate::fl!("mount-flag-noexec"),
            MountFlag::NoSuid     => crate::fl!("mount-flag-nosuid"),
            MountFlag::NoAtime    => crate::fl!("mount-flag-noatime"),
            MountFlag::NoDiratime => crate::fl!("mount-flag-nodiratime"),
            MountFlag::Sync       => crate::fl!("mount-flag-sync"),
        }
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FsType {
    #[default]
    Ext4,
    Btrfs,
    Xfs,
    Vfat,
    Exfat,
    Ntfs,
}

impl FsType {
    pub const ALL: [FsType; 6] = [
        FsType::Ext4,
        FsType::Btrfs,
        FsType::Xfs,
        FsType::Vfat,
        FsType::Exfat,
        FsType::Ntfs,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FsType::Ext4  => "ext4",
            FsType::Btrfs => "btrfs",
            FsType::Xfs   => "xfs",
            FsType::Vfat  => "vfat",
            FsType::Exfat => "exfat",
            FsType::Ntfs  => "ntfs",
        }
    }
}

impl std::str::FromStr for FsType {
    type Err = ();

    /// Used for display (colour coding) only.  ext2/ext3 → Ext4 and
    /// fat16/fat32 → Vfat are aliases for visual grouping; this value must
    /// never be round-tripped back into a format or create-partition call.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ext4" | "ext3" | "ext2" => Ok(FsType::Ext4),
            "btrfs"                   => Ok(FsType::Btrfs),
            "xfs"                     => Ok(FsType::Xfs),
            "vfat" | "fat16" | "fat32" => Ok(FsType::Vfat),
            "exfat"                   => Ok(FsType::Exfat),
            "ntfs"                    => Ok(FsType::Ntfs),
            _                         => Err(()),
        }
    }
}

/// Tracks which D-Bus operation is currently in flight so the UI can show a spinner.
#[derive(Debug, Clone)]
pub enum OperationKind {
    Mounting(String),
    Unmounting(String),
    Formatting(String),
    CreatingPartition { drive_id: String },
}

/// State for the "mount at location" modal dialog.
pub struct MountLocationDialog {
    pub device: String,
    pub path: String,
    pub show_advanced: bool,
    pub selected_flags: HashSet<MountFlag>,
    /// Whether to write a persistent `/etc/fstab` entry before mounting.
    /// Only relevant when `path` is non-empty; ignored for managed mounts.
    pub add_to_fstab: bool,
    /// Snapshot of whether the partition had an fstab entry at the moment this
    /// dialog was opened.  Stored here rather than re-reading `SelectionState`
    /// in the view so that the disabled state is stable for the dialog's
    /// lifetime even if an async `FstabLoaded` arrives mid-session.
    /// When true the checkbox is shown checked-and-disabled and the fstab write
    /// is suppressed at confirm time regardless of `add_to_fstab`.
    pub already_in_fstab: bool,
}

impl MountLocationDialog {
    /// The effective value to pass to `mount_at_path`.
    /// Suppresses the write when the partition is already in fstab — guards
    /// against a race where `already_in_fstab` was set before the async lookup
    /// completed, or against a future UI path that bypasses the disabled checkbox.
    pub fn effective_add_to_fstab(&self) -> bool {
        self.add_to_fstab && !self.already_in_fstab
    }
}

/// State for the inline format panel that expands inside the partition detail card.
pub struct FormatPanel {
    pub device: String,
    pub fs_type: FsType,
    pub label: String,
}

/// State for the create-partition panel shown when unallocated space is selected.
pub struct CreatePartitionPanel {
    pub drive_id: String,
    /// udisks2 object path of the whole-disk block device (e.g. `.../block_devices/sda`).
    pub block_device: String,
    /// Maximum bytes available in the unallocated region.
    pub max_bytes: u64,
    /// User-typed size string (e.g. "10 GB").
    pub size_str: String,
    pub fs_type: FsType,
    pub label: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    LaunchUrl(String),
    ToggleContextPage(ContextPage),
    RefreshDisks,
    DisksScanned(Result<Vec<Drive>, String>),
    SelectPartition(BlockDevice),
    SelectUnallocated(String),
    /// Async fstab lookup completed after a partition was selected.
    FstabLoaded(Option<FstabEntry>),
    /// Open the "mount at location" dialog for a device.
    /// `prefill_path` pre-populates the path field (e.g. from an existing mount point).
    OpenMountDialog { device: String, prefill_path: Option<String> },
    /// User edited the path text field in the mount dialog.
    MountDialogPathChanged(String),
    /// User toggled the advanced-options section in the mount dialog.
    ToggleMountDialogAdvanced,
    /// User toggled a mount flag in the dialog advanced section.
    ToggleDialogMountFlag(MountFlag),
    /// User toggled the "add to /etc/fstab" checkbox in the mount dialog.
    ToggleFstabCheckbox,
    /// User clicked Browse — open the portal folder picker.
    BrowseMountPath,
    /// Portal folder picker returned (None = cancelled).
    BrowsePathPicked(Option<String>),
    /// Confirmed mount from the dialog.
    ConfirmMountDialog,
    /// Closed / cancelled the mount dialog.
    CloseMountDialog,
    /// Open the unmount confirmation dialog.
    OpenUnmountDialog(String, Vec<String>),
    /// User confirmed the unmount dialog.
    ConfirmUnmount,
    /// User cancelled the unmount dialog.
    CloseUnmountDialog,
    /// Open the inline format panel for a device.
    OpenFormatPanel(String),
    /// User picked a different filesystem in the format panel.
    FormatPanelFsChanged(FsType),
    /// User edited the label field in the format panel.
    FormatPanelLabelChanged(String),
    /// User cancelled the format panel.
    CloseFormatPanel,
    /// User confirmed — execute the format.
    ConfirmFormat,
    /// Open the create-partition panel for the unallocated region of a drive.
    OpenCreatePartitionPanel { drive_id: String, max_bytes: u64 },
    /// User edited the size field in the create-partition panel.
    CreatePartitionSizeChanged(String),
    /// User picked a different filesystem in the create-partition panel.
    CreatePartitionFsChanged(FsType),
    /// User edited the label field in the create-partition panel.
    CreatePartitionLabelChanged(String),
    /// User cancelled the create-partition panel.
    CloseCreatePartitionPanel,
    /// User confirmed — execute the partition creation.
    ConfirmCreatePartition,
    /// An async operation (mount / unmount / format) returned an error.
    OperationFailed(String),
    /// User dismissed the inline error.
    DismissError,
    /// Config changed externally (e.g. another process wrote to cosmic-config).
    ConfigUpdate(crate::config::Config),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::disks::{BlockDevice, FstabEntry, PartitionState};

    fn dummy_block_device() -> BlockDevice {
        BlockDevice {
            device: String::new(),
            uuid: String::new(),
            label: String::new(),
            fs_type: String::new(),
            size: 0,
            offset: 0,
            mount_points: vec![],
            drive_id: String::new(),
            state: PartitionState::Unmounted,
            part_uuid: String::new(),
            part_label: String::new(),
        }
    }

    fn partition_ctx(fstab_entry: Option<FstabEntry>) -> PartitionContext {
        PartitionContext {
            partition: dummy_block_device(),
            fstab_entry,
            format_panel: None,
            operation_in_progress: None,
            operation_error: None,
        }
    }

    fn dummy_fstab() -> FstabEntry {
        FstabEntry {
            spec: "UUID=abc-123".to_string(),
            mount_point: "/mnt/data".to_string(),
            options: "defaults".to_string(),
        }
    }

    #[test]
    fn no_selection_returns_false() {
        assert!(!SelectionState::None.has_fstab_entry());
    }

    #[test]
    fn partition_without_fstab_returns_false() {
        let state = SelectionState::Partition(partition_ctx(None));
        assert!(!state.has_fstab_entry());
    }

    #[test]
    fn partition_with_fstab_returns_true() {
        let state = SelectionState::Partition(partition_ctx(Some(dummy_fstab())));
        assert!(state.has_fstab_entry());
    }

    #[test]
    fn unallocated_returns_false() {
        let state = SelectionState::Unallocated(UnallocatedContext {
            drive_id: "test-drive".to_string(),
            create_panel: None,
            operation_in_progress: None,
            operation_error: None,
        });
        assert!(!state.has_fstab_entry());
    }

    fn mount_dialog(add_to_fstab: bool, already_in_fstab: bool) -> MountLocationDialog {
        MountLocationDialog {
            device: String::new(),
            path: String::new(),
            show_advanced: false,
            selected_flags: std::collections::HashSet::new(),
            add_to_fstab,
            already_in_fstab,
        }
    }

    #[test]
    fn effective_fstab_normal_write() {
        // user checked the box, not already in fstab → write
        assert!(mount_dialog(true, false).effective_add_to_fstab());
    }

    #[test]
    fn effective_fstab_suppressed_when_already_present() {
        // already in fstab → no write even if checkbox was somehow set true
        assert!(!mount_dialog(true, true).effective_add_to_fstab());
    }

    #[test]
    fn effective_fstab_unchecked_no_write() {
        // user left box unchecked → no write
        assert!(!mount_dialog(false, false).effective_add_to_fstab());
    }

    #[test]
    fn effective_fstab_unchecked_already_present() {
        // both false → no write
        assert!(!mount_dialog(false, true).effective_add_to_fstab());
    }
}
