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
