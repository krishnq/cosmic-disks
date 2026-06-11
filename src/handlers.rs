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
    pub fn handle(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::LaunchUrl(url) => {
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
                let uuid = partition.uuid.clone();
                let label = partition.label.clone();
                let device = partition.device.clone();
                let part_uuid = partition.part_uuid.clone();
                let part_label = partition.part_label.clone();
                self.selection_state = SelectionState::Partition(PartitionContext {
                    partition,
                    fstab_entry: None,
                    format_panel: None,
                    operation_in_progress: None,
                    operation_error: None,
                });

                return cosmic::task::future(async move {
                    let entry =
                        disks::lookup_fstab(&uuid, &label, &device, &part_uuid, &part_label)
                            .await;
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

            Message::OpenMountDialog { device, prefill_path } => {
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
                if let (ActiveDialog::Mount(ref mut d), Some(p)) =
                    (&mut self.active_dialog, path)
                {
                    d.path = p;
                }
            }

            Message::ConfirmMountDialog => {
                if let ActiveDialog::Mount(d) =
                    std::mem::replace(&mut self.active_dialog, ActiveDialog::None)
                {
                    let device = d.device.clone();
                    let mount_path = d.path.trim().to_string();
                    let add_to_fstab = d.effective_add_to_fstab();
                    let options = MountFlag::ALL
                        .iter()
                        .filter(|f| d.selected_flags.contains(f))
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join(",");

                    if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                        ctx.operation_in_progress =
                            Some(OperationKind::Mounting(device.clone()));
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
                            Err(e) => {
                                cosmic::Action::App(Message::OperationFailed(e.to_string()))
                            }
                        }
                    });
                }
            }

            Message::CloseMountDialog => {
                self.active_dialog = ActiveDialog::None;
            }

            Message::OpenUnmountDialog(device, mount_points) => {
                self.active_dialog = ActiveDialog::Unmount { device, mount_points };
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
                        ctx.operation_in_progress =
                            Some(OperationKind::Unmounting(device.clone()));
                    }

                    return cosmic::task::future(async move {
                        match crate::actions::volumes::unmount(&device).await {
                            Ok(_) => cosmic::Action::App(Message::RefreshDisks),
                            Err(e) => {
                                cosmic::Action::App(Message::OperationFailed(e.to_string()))
                            }
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
                    let device = fp.device.clone();
                    let fs_type = fp.fs_type.as_str().to_string();
                    let label = fp.label.clone();

                    if let SelectionState::Partition(ref mut ctx) = self.selection_state {
                        ctx.operation_error = None;
                        ctx.operation_in_progress =
                            Some(OperationKind::Formatting(device.clone()));
                    }

                    self.config.default_fs_type = fp.fs_type;
                    if let Some(ref handler) = self.config_handler {
                        if let Err(e) = self.config.write_entry(handler) {
                            log::warn!("failed to save config: {e}");
                        }
                    }

                    return cosmic::task::future(async move {
                        match crate::actions::volumes::format(&device, &fs_type, &label).await {
                            Ok(_) => cosmic::Action::App(Message::RefreshDisks),
                            Err(e) => {
                                cosmic::Action::App(Message::OperationFailed(e.to_string()))
                            }
                        }
                    });
                }
            }

            Message::OpenCreatePartitionPanel { drive_id, max_bytes } => {
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
                            if let SelectionState::Unallocated(ref mut ctx) =
                                self.selection_state
                            {
                                ctx.operation_error =
                                    Some(fl!("error-invalid-size", input = bad_input));
                                ctx.create_panel = Some(panel);
                            }
                            return Task::none();
                        }
                        Some(b) if b == 0 || b >= panel.max_bytes => 0u64,
                        Some(b) => b,
                    };

                    let fs_type = panel.fs_type.as_str().to_string();
                    let label = panel.label.clone();
                    let block_device = panel.block_device.clone();

                    if let SelectionState::Unallocated(ref mut ctx) = self.selection_state {
                        ctx.operation_error = None;
                        ctx.operation_in_progress = Some(OperationKind::CreatingPartition {
                            drive_id: panel.drive_id.clone(),
                        });
                    }

                    self.config.default_fs_type = panel.fs_type;
                    if let Some(ref handler) = self.config_handler {
                        if let Err(e) = self.config.write_entry(handler) {
                            log::warn!("failed to save config: {e}");
                        }
                    }

                    return cosmic::task::future(async move {
                        match crate::actions::volumes::create_partition(
                            &block_device,
                            size_bytes,
                            &fs_type,
                            &label,
                        )
                        .await
                        {
                            Ok(_) => cosmic::Action::App(Message::RefreshDisks),
                            Err(e) => {
                                cosmic::Action::App(Message::OperationFailed(e.to_string()))
                            }
                        }
                    });
                }
            }

            Message::OperationFailed(e) => {
                self.operation_error = Some(e.clone());
                match &mut self.selection_state {
                    SelectionState::Partition(ctx) => {
                        ctx.operation_in_progress = None;
                        ctx.operation_error = Some(e);
                    }
                    SelectionState::Unallocated(ctx) => {
                        ctx.operation_in_progress = None;
                        ctx.operation_error = Some(e);
                    }
                    SelectionState::None => {}
                }
                return cosmic::task::future(async {
                    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                    cosmic::Action::App(Message::DismissError)
                });
            }

            Message::DismissError => {
                self.operation_error = None;
                match &mut self.selection_state {
                    SelectionState::Partition(ctx) => ctx.operation_error = None,
                    SelectionState::Unallocated(ctx) => ctx.operation_error = None,
                    SelectionState::None => {}
                }
            }

            Message::ConfigUpdate(config) => {
                self.config = config;
            }

            Message::RefreshDisks => {
                self.load_state = LoadState::Scanning;
                self.selection_state = SelectionState::None;
                self.active_dialog = ActiveDialog::None;

                return cosmic::task::future(async {
                    cosmic::Action::App(match disks::scan_drives().await {
                        Ok(drives) => Message::DisksScanned(Ok(drives)),
                        Err(e) => Message::DisksScanned(Err(e.to_string())),
                    })
                });
            }

            Message::DisksScanned(result) => {
                self.load_state = match result {
                    Ok(drives) => LoadState::Ready(drives),
                    Err(e) => LoadState::Error(e),
                };
            }
        }

        Task::none()
    }
}
