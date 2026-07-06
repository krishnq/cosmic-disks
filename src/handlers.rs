// SPDX-License-Identifier: GPL-3.0

use bytesize::ByteSize;
use cosmic::cosmic_config::CosmicConfigEntry;
use cosmic::prelude::*;
use std::collections::HashSet;

use crate::actions::disks;
use crate::app::AppModel;
use crate::fl;
use crate::message::{
    ActiveDialog, CreatePartitionPanel, FormatPanel, LoadState, Message, MountFlag,
    MountLocationDialog, OperationKind, PartitionContext, SelectionState, UnallocatedContext,
};

impl AppModel {
    pub(crate) fn handle(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::LaunchUrl(url) => {
                // best-effort; failure is non-fatal (browser may not be installed)
                let _ = open::that_detached(&url);
            }

            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                }
            }

            Message::SelectPartition(partition) => {
                // Clone only the fields the async closure needs; move `partition` itself.
                let uuid = partition.uuid.clone();
                let label = partition.label.clone();
                let device = partition.device.clone();
                let part_uuid = partition.part_uuid.clone();
                let part_label = partition.part_label.clone();

                self.selection_state = SelectionState::Partition(Box::new(PartitionContext {
                    partition,
                    fstab_entry: None,
                    format_panel: None,
                    operation_in_progress: None,
                    operation_error: None,
                }));

                return cosmic::task::future(async move {
                    let entry =
                        disks::lookup_fstab(&uuid, &label, &device, &part_uuid, &part_label).await;
                    cosmic::Action::App(Message::FstabLoaded(entry))
                });
            }

            Message::FstabLoaded(entry) => {
                if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                    ctx.fstab_entry = entry;
                }
            }

            Message::SelectUnallocated(drive_id) => {
                self.selection_state = SelectionState::Unallocated(UnallocatedContext {
                    drive_id,
                    create_panel: None,
                    operation_in_progress: None,
                    operation_error: None,
                });
            }

            Message::OpenMountDialog {
                device,
                prefill_path,
            } => {
                let already_in_fstab = self.selection_state.has_fstab_entry();
                self.active_dialog = ActiveDialog::Mount(MountLocationDialog {
                    device,
                    path: prefill_path.unwrap_or_default(),
                    show_advanced: false,
                    selected_flags: HashSet::new(),
                    add_to_fstab: false,
                    already_in_fstab,
                });
                if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                    ctx.operation_error = None;
                }
            }

            Message::MountDialogPathChanged(new_path) => {
                if let ActiveDialog::Mount(ref mut d) = self.active_dialog {
                    d.path = new_path;
                }
            }

            Message::ToggleMountDialogAdvanced => {
                if let ActiveDialog::Mount(ref mut d) = self.active_dialog {
                    d.show_advanced = !d.show_advanced;
                }
            }

            Message::ToggleDialogMountFlag(flag) => {
                if let ActiveDialog::Mount(ref mut d) = self.active_dialog {
                    if !d.selected_flags.remove(&flag) {
                        d.selected_flags.insert(flag);
                    }
                }
            }

            Message::ToggleFstabCheckbox => {
                if let ActiveDialog::Mount(ref mut d) = self.active_dialog {
                    if !d.already_in_fstab {
                        d.add_to_fstab = !d.add_to_fstab;
                    }
                }
            }

            Message::BrowseMountPath => {
                return cosmic::task::future(async {
                    let path = crate::actions::portal::pick_folder().await;
                    cosmic::Action::App(Message::BrowsePathPicked(path))
                });
            }

            Message::BrowsePathPicked(path) => {
                if let (ActiveDialog::Mount(ref mut d), Some(p)) = (&mut self.active_dialog, path) {
                    d.path = p;
                }
            }

            Message::ConfirmMountDialog => {
                if let ActiveDialog::Mount(d) =
                    std::mem::replace(&mut self.active_dialog, ActiveDialog::None)
                {
                    let mount_path = d.path.trim().to_string();
                    let add_to_fstab = d.effective_add_to_fstab();
                    let options = MountFlag::ALL
                        .iter()
                        .filter(|f| d.selected_flags.contains(f))
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    let device = d.device;

                    if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                        ctx.operation_in_progress = Some(OperationKind::Mounting(device.clone()));
                    }

                    return cosmic::task::future(async move {
                        let result = if mount_path.is_empty() {
                            crate::actions::volumes::mount_default(&device, &options).await
                        } else {
                            crate::actions::volumes::mount_at_path(
                                &device,
                                &mount_path,
                                &options,
                                add_to_fstab,
                            )
                            .await
                        };
                        match result {
                            Ok(_) => cosmic::Action::App(Message::RefreshDisks),
                            Err(e) => cosmic::Action::App(Message::OperationFailed(e.to_string())),
                        }
                    });
                }
            }

            Message::CloseMountDialog => {
                self.active_dialog = ActiveDialog::None;
            }

            Message::OpenUnmountDialog(device, mount_points) => {
                self.active_dialog = ActiveDialog::Unmount {
                    device,
                    mount_points,
                };
                if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                    ctx.operation_error = None;
                }
            }

            Message::ConfirmUnmount => {
                if let ActiveDialog::Unmount { device, .. } =
                    std::mem::replace(&mut self.active_dialog, ActiveDialog::None)
                {
                    if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                        ctx.operation_error = None;
                        ctx.operation_in_progress = Some(OperationKind::Unmounting(device.clone()));
                    }

                    return cosmic::task::future(async move {
                        match crate::actions::volumes::unmount(&device).await {
                            Ok(_) => cosmic::Action::App(Message::RefreshDisks),
                            Err(e) => cosmic::Action::App(Message::OperationFailed(e.to_string())),
                        }
                    });
                }
            }

            Message::CloseUnmountDialog => {
                self.active_dialog = ActiveDialog::None;
            }

            Message::OpenFormatPanel(device) => {
                if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                    ctx.format_panel = Some(FormatPanel {
                        device,
                        fs_type: self.config.default_fs_type,
                        label: String::new(),
                    });
                    ctx.operation_error = None;
                }
            }

            Message::FormatPanelFsChanged(fs) => {
                if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                    if let Some(ref mut fp) = ctx.format_panel {
                        fp.fs_type = fs;
                    }
                }
            }

            Message::FormatPanelLabelChanged(s) => {
                if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                    if let Some(ref mut fp) = ctx.format_panel {
                        fp.label = s;
                    }
                }
            }

            Message::CloseFormatPanel => {
                if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                    ctx.format_panel = None;
                }
            }

            Message::ConfirmFormat => {
                let fp = match self.selection_state {
                    SelectionState::Partition(ref mut ctx) => ctx.format_panel.take(),
                    _ => None,
                };
                if let Some(fp) = fp {
                    let FormatPanel {
                        device,
                        fs_type,
                        label,
                    } = fp;
                    let fs_type_str = fs_type.as_str().to_string();

                    if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                        ctx.operation_error = None;
                        ctx.operation_in_progress = Some(OperationKind::Formatting(device.clone()));
                    }

                    self.config.default_fs_type = fs_type;
                    if let Some(ref handler) = self.config_handler {
                        if let Err(e) = self.config.write_entry(handler) {
                            log::warn!("failed to save config: {e}");
                        }
                    }

                    return cosmic::task::future(async move {
                        match crate::actions::volumes::format(&device, &fs_type_str, &label).await {
                            Ok(_) => cosmic::Action::App(Message::RefreshDisks),
                            Err(e) => cosmic::Action::App(Message::OperationFailed(e.to_string())),
                        }
                    });
                }
            }

            Message::OpenCreatePartitionPanel {
                drive_id,
                max_bytes,
            } => {
                let block_device = match &self.load_state {
                    LoadState::Ready(drives) => drives
                        .iter()
                        .find(|d| d.id == drive_id)
                        .map(|d| d.block_device.clone())
                        .unwrap_or_default(),
                    _ => String::new(),
                };

                if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                    ctx.create_panel = Some(CreatePartitionPanel {
                        block_device,
                        size_str: ByteSize::b(max_bytes).to_string(),
                        drive_id: ctx.drive_id.clone(),
                        max_bytes,
                        fs_type: self.config.default_fs_type,
                        label: String::new(),
                    });
                    ctx.operation_error = None;
                }
            }

            Message::CreatePartitionSizeChanged(s) => {
                if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                    if let Some(ref mut p) = ctx.create_panel {
                        p.size_str = s;
                    }
                }
            }

            Message::CreatePartitionFsChanged(fs) => {
                if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                    if let Some(ref mut p) = ctx.create_panel {
                        p.fs_type = fs;
                    }
                }
            }

            Message::CreatePartitionLabelChanged(s) => {
                if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                    if let Some(ref mut p) = ctx.create_panel {
                        p.label = s;
                    }
                }
            }

            Message::CloseCreatePartitionPanel => {
                if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                    ctx.create_panel = None;
                }
            }

            Message::ConfirmCreatePartition => {
                let panel = match self.selection_state {
                    SelectionState::Unallocated(ref mut ctx) => ctx.create_panel.take(),
                    _ => None,
                };
                if let Some(panel) = panel {
                    let parsed = panel.size_str.trim().parse::<ByteSize>().map(|b| b.0).ok();
                    let size_bytes = match parsed {
                        None => {
                            let bad_input = panel.size_str.clone();
                            if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                                ctx.operation_error =
                                    Some(fl!("error-invalid-size", input = bad_input));
                                ctx.create_panel = Some(panel);
                            }
                            return Task::none();
                        }
                        Some(b) if b == 0 || b >= panel.max_bytes => 0u64,
                        Some(b) => b,
                    };

                    let CreatePartitionPanel {
                        drive_id,
                        fs_type,
                        label,
                        block_device,
                        ..
                    } = panel;
                    let fs_type_str = fs_type.as_str().to_string();

                    if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                        ctx.operation_error = None;
                        ctx.operation_in_progress =
                            Some(OperationKind::CreatingPartition { drive_id });
                    }

                    self.config.default_fs_type = fs_type;
                    if let Some(ref handler) = self.config_handler {
                        if let Err(e) = self.config.write_entry(handler) {
                            log::warn!("failed to save config: {e}");
                        }
                    }

                    return cosmic::task::future(async move {
                        match crate::actions::volumes::create_partition(
                            &block_device,
                            size_bytes,
                            &fs_type_str,
                            &label,
                        )
                        .await
                        {
                            Ok(_) => cosmic::Action::App(Message::RefreshDisks),
                            Err(e) => cosmic::Action::App(Message::OperationFailed(e.to_string())),
                        }
                    });
                }
            }

            Message::OperationFailed(e) => {
                // The error is shown in exactly one place: inside the selected
                // card when a selection exists, otherwise in the top-level banner.
                match &mut self.selection_state {
                    SelectionState::Partition(ctx) => {
                        ctx.operation_in_progress = None;
                        ctx.operation_error = Some(e);
                    }
                    SelectionState::Unallocated(ctx) => {
                        ctx.operation_in_progress = None;
                        ctx.operation_error = Some(e);
                    }
                    SelectionState::None => self.operation_error = Some(e),
                }
                // Epoch-tag the auto-dismiss so a stale timer from an earlier
                // error cannot dismiss a newer one.
                self.error_epoch += 1;
                let epoch = self.error_epoch;
                return cosmic::task::future(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                    cosmic::Action::App(Message::AutoDismissError(epoch))
                });
            }

            Message::AutoDismissError(epoch) => {
                if epoch == self.error_epoch {
                    self.clear_errors();
                }
            }

            Message::DismissError => {
                self.clear_errors();
            }

            Message::ConfigUpdate(config) => {
                self.config = config;
            }

            Message::RefreshDisks => {
                // Keep the current view, selection, and any open dialog while the
                // scan runs; DisksScanned reconciles them with the fresh results.
                return cosmic::task::future(async {
                    cosmic::Action::App(match disks::scan_drives().await {
                        Ok(drives) => Message::DisksScanned(Ok(drives)),
                        Err(e) => Message::DisksScanned(Err(e.to_string())),
                    })
                });
            }

            Message::DisksScanned(result) => {
                let drives = match result {
                    Ok(drives) => drives,
                    Err(e) => {
                        self.load_state = LoadState::Error(e);
                        return Task::none();
                    }
                };

                let followup = self.recompute_selection(&drives);
                self.recompute_dialog(&drives);
                self.load_state = LoadState::Ready(drives);
                return followup;
            }
        }

        Task::none()
    }

    /// Re-point the current selection at the freshly scanned data, or clear it
    /// if the selected partition / drive no longer exists.  Returns a follow-up
    /// task to re-check `/etc/fstab` for a still-selected partition (a mount may
    /// have just written an entry).
    fn recompute_selection(&mut self, drives: &[disks::Drive]) -> Task<cosmic::Action<Message>> {
        let mut clear = false;
        let mut followup = Task::none();

        match &mut self.selection_state {
            SelectionState::Partition(ctx) => {
                let fresh = drives
                    .iter()
                    .flat_map(|d| &d.partitions)
                    .find(|p| p.device == ctx.partition.device);
                match fresh {
                    Some(p) => {
                        ctx.partition = p.clone();
                        ctx.operation_in_progress = None;

                        let uuid = p.uuid.clone();
                        let label = p.label.clone();
                        let device = p.device.clone();
                        let part_uuid = p.part_uuid.clone();
                        let part_label = p.part_label.clone();
                        followup = cosmic::task::future(async move {
                            let entry = disks::lookup_fstab(
                                &uuid,
                                &label,
                                &device,
                                &part_uuid,
                                &part_label,
                            )
                            .await;
                            cosmic::Action::App(Message::FstabLoaded(entry))
                        });
                    }
                    None => clear = true,
                }
            }
            SelectionState::Unallocated(ctx) => {
                if drives.iter().any(|d| d.id == ctx.drive_id) {
                    ctx.operation_in_progress = None;
                } else {
                    clear = true;
                }
            }
            SelectionState::None => {}
        }

        if clear {
            self.selection_state = SelectionState::None;
        }
        followup
    }

    /// Close an open dialog if the device it refers to disappeared from the scan.
    fn recompute_dialog(&mut self, drives: &[disks::Drive]) {
        let device = match &self.active_dialog {
            ActiveDialog::Mount(d) => d.device.as_str(),
            ActiveDialog::Unmount { device, .. } => device.as_str(),
            ActiveDialog::None => return,
        };
        let still_exists = drives
            .iter()
            .flat_map(|d| &d.partitions)
            .any(|p| p.device == device);
        if !still_exists {
            self.active_dialog = ActiveDialog::None;
        }
    }

    fn clear_errors(&mut self) {
        self.operation_error = None;
        match &mut self.selection_state {
            SelectionState::Partition(ctx) => ctx.operation_error = None,
            SelectionState::Unallocated(ctx) => ctx.operation_error = None,
            SelectionState::None => {}
        }
    }
}
