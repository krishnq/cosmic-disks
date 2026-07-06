// SPDX-License-Identifier: GPL-3.0

use bytesize::ByteSize;
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::widget::{self, canvas, icon};

use crate::actions::disks::{BlockDevice, Drive, FstabEntry, PartitionState};
use crate::fl;
use crate::message::{
    CreatePartitionPanel, FormatPanel, FsType, Message, OperationKind, SelectionState,
};
use crate::ui::disk_bar::DiskBar;

pub fn disk_card<'a>(
    drive: &'a Drive,
    selection: &'a SelectionState,
) -> cosmic::Element<'a, Message> {
    let spacing = cosmic::theme::spacing();

    let disk_icon = if drive.removable {
        "drive-removable-media-symbolic"
    } else {
        "drive-harddisk-symbolic"
    };

    let header = widget::row()
        .push(icon::from_name(disk_icon).size(24))
        .push(widget::text::title4(drive.display_name()).width(Length::Fill))
        .spacing(spacing.space_s)
        .align_y(Alignment::Center)
        .padding([spacing.space_s, spacing.space_m]);

    let mut card = widget::column().push(header).spacing(spacing.space_xxs);

    // Resolve selection fields relevant to this specific drive.
    let (
        selected_partition,
        selected_unallocated,
        fstab_entry,
        operation_error,
        format_panel,
        create_partition_panel,
        operation_in_progress,
    ) = match selection {
        SelectionState::Partition(p) if p.partition.drive_id == drive.id => (
            Some(&p.partition),
            false,
            p.fstab_entry.as_ref(),
            p.operation_error.as_deref(),
            p.format_panel.as_ref(),
            None,
            p.operation_in_progress.as_ref(),
        ),
        SelectionState::Unallocated(u) if u.drive_id == drive.id => (
            None,
            true,
            None,
            u.operation_error.as_deref(),
            None,
            u.create_panel.as_ref(),
            u.operation_in_progress.as_ref(),
        ),
        _ => (None, false, None, None, None, None, None),
    };

    if drive.size > 0 {
        let selected_dev = selected_partition.map(|p| p.device.as_str());

        let bar = canvas(DiskBar {
            drive_id: &drive.id,
            drive_size: drive.size,
            partitions: &drive.partitions,
            selected_device: selected_dev,
            selected_unallocated,
        })
        .width(Length::Fill)
        .height(cosmic::iced::Pixels(40.0));

        card = card.push(widget::container(bar).padding([
            0,
            spacing.space_m,
            spacing.space_s,
            spacing.space_m,
        ]));
    }

    if let Some(p) = selected_partition {
        let op_label = operation_in_progress.and_then(|op| match op {
            OperationKind::Mounting(dev) if dev == &p.device => {
                Some(fl!("op-mounting", device = dev.clone()))
            }
            OperationKind::Unmounting(dev) if dev == &p.device => {
                Some(fl!("op-unmounting", device = dev.clone()))
            }
            OperationKind::Formatting(dev) if dev == &p.device => {
                Some(fl!("op-formatting", device = dev.clone()))
            }
            _ => None,
        });

        card = card.push(partition_details(
            p,
            fstab_entry,
            operation_error,
            format_panel,
            op_label,
            &spacing,
        ));
    } else if selected_unallocated {
        let used: u64 = drive.partitions.iter().map(|p| p.size).sum();
        let op_label = operation_in_progress.and_then(|op| match op {
            OperationKind::CreatingPartition { drive_id: did } if did == &drive.id => {
                Some(fl!("op-creating-partition"))
            }
            _ => None,
        });

        card = card.push(unallocated_details(
            &drive.id,
            drive.size.saturating_sub(used),
            create_partition_panel,
            op_label,
            &spacing,
        ));
    }

    card.apply(widget::container)
        .class(cosmic::theme::Container::Card)
        .into()
}

// ---------------------------------------------------------------------------
// Detail panels
// ---------------------------------------------------------------------------

fn partition_details<'a>(
    p: &'a BlockDevice,
    fstab_entry: Option<&'a FstabEntry>,
    operation_error: Option<&'a str>,
    format_panel: Option<&'a FormatPanel>,
    op_label: Option<String>,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    let mounts = match p.state {
        PartitionState::Mounted => p.mount_points.join("\n"),
        PartitionState::Unmounted => fl!("not-mounted"),
        PartitionState::Unformatted => fl!("unformatted"),
    };
    let fs = if p.fs_type.is_empty() {
        fl!("unknown-fs")
    } else {
        p.fs_type.clone()
    };

    let mut details = widget::column()
        .push(detail_row(fl!("field-device"), p.device.clone(), spacing))
        .push(detail_row(fl!("field-size"), p.display_size(), spacing))
        .push(detail_row(fl!("field-filesystem"), fs, spacing))
        .push(detail_row(fl!("field-label"), p.label.clone(), spacing))
        .push(detail_row(fl!("field-mounted-at"), mounts, spacing));

    if let Some(fstab) = fstab_entry {
        details = details
            .push(detail_row(
                fl!("field-fstab-mount"),
                fstab.mount_point.clone(),
                spacing,
            ))
            .push(detail_row(
                fl!("field-fstab-options"),
                fstab.options.clone(),
                spacing,
            ));
    }

    let details = details.spacing(spacing.space_s).padding([
        spacing.space_s,
        spacing.space_m,
        0,
        spacing.space_m,
    ]);

    let actions_area = if let Some(label) = op_label {
        busy_row(label, spacing)
    } else if let Some(fp) = format_panel.filter(|fp| fp.device == p.device) {
        format_panel_widget(fp, spacing)
    } else {
        widget::container(action_buttons(p, fstab_entry, spacing))
            .padding([spacing.space_s, spacing.space_m])
            .into()
    };

    let mut col = widget::column().push(details).push(actions_area).spacing(0);

    if let Some(err) = operation_error {
        col = col.push(error_banner(err, spacing));
    }

    col.into()
}

fn action_buttons<'a>(
    p: &'a BlockDevice,
    fstab_entry: Option<&'a FstabEntry>,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    let row = widget::row().spacing(spacing.space_s);

    match p.state {
        PartitionState::Mounted => row.push(widget::button::destructive(fl!("unmount")).on_press(
            Message::OpenUnmountDialog(p.device.clone(), p.mount_points.clone()),
        )),
        PartitionState::Unmounted => {
            let row = if let Some(fstab) = fstab_entry {
                row.push(
                    widget::button::suggested(fl!(
                        "mount-on-path",
                        path = fstab.mount_point.clone()
                    ))
                    .on_press(Message::OpenMountDialog {
                        device: p.device.clone(),
                        prefill_path: Some(fstab.mount_point.clone()),
                    }),
                )
            } else {
                row
            };
            row.push(
                widget::button::standard(fl!("mount-new-location")).on_press(
                    Message::OpenMountDialog {
                        device: p.device.clone(),
                        prefill_path: None,
                    },
                ),
            )
            .push(
                widget::button::standard(fl!("format"))
                    .on_press(Message::OpenFormatPanel(p.device.clone())),
            )
        }
        PartitionState::Unformatted => row.push(
            widget::button::standard(fl!("format"))
                .on_press(Message::OpenFormatPanel(p.device.clone())),
        ),
    }
    .into()
}

/// Shows an animated icon + operation label while a D-Bus call is in-flight.
fn busy_row<'a>(
    label: String,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    widget::row()
        .push(icon::from_name("process-working-symbolic").size(16))
        .push(widget::text::body(label))
        .spacing(spacing.space_s)
        .align_y(Alignment::Center)
        .apply(widget::container)
        .padding([spacing.space_s, spacing.space_m])
        .into()
}

fn error_banner<'a>(
    message: &'a str,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    widget::row()
        .push(widget::text::body(message).width(Length::Fill))
        .push(widget::button::standard(fl!("dismiss")).on_press(Message::DismissError))
        .spacing(spacing.space_s)
        .align_y(Alignment::Center)
        .apply(widget::container)
        .padding([spacing.space_s, spacing.space_m])
        .into()
}

fn unallocated_details<'a>(
    drive_id: &'a str,
    free_bytes: u64,
    create_panel: Option<&'a CreatePartitionPanel>,
    op_label: Option<String>,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    let info = widget::column()
        .push(detail_row(
            fl!("field-type"),
            fl!("field-unallocated"),
            spacing,
        ))
        .push(detail_row(
            fl!("field-size"),
            ByteSize::b(free_bytes).to_string(),
            spacing,
        ))
        .spacing(spacing.space_s)
        .padding([spacing.space_s, spacing.space_m, 0, spacing.space_m]);

    let actions: cosmic::Element<'_, Message> = if let Some(label) = op_label {
        busy_row(label, spacing)
    } else if let Some(p) = create_panel.filter(|p| p.drive_id == drive_id) {
        create_partition_widget(p, spacing)
    } else {
        widget::container(
            widget::row()
                .push(widget::button::suggested(fl!("create-partition")).on_press(
                    Message::OpenCreatePartitionPanel {
                        drive_id: drive_id.to_string(),
                        max_bytes: free_bytes,
                    },
                ))
                .spacing(spacing.space_s),
        )
        .padding([spacing.space_s, spacing.space_m])
        .into()
    };

    widget::column().push(info).push(actions).into()
}

fn create_partition_widget<'a>(
    panel: &'a CreatePartitionPanel,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    let size_input = widget::text_input(fl!("create-size-placeholder"), panel.size_str.as_str())
        .on_input(Message::CreatePartitionSizeChanged)
        .width(Length::Fill);

    let mut fs_row = widget::row().spacing(spacing.space_xs);
    for &fs in FsType::ALL.iter() {
        let btn = if panel.fs_type == fs {
            widget::button::suggested(fs.as_str())
        } else {
            widget::button::standard(fs.as_str())
        };
        fs_row = fs_row.push(btn.on_press(Message::CreatePartitionFsChanged(fs)));
    }

    let label_input = widget::text_input(fl!("format-label-placeholder"), panel.label.as_str())
        .on_input(Message::CreatePartitionLabelChanged)
        .width(Length::Fill);

    let buttons = widget::row()
        .push(widget::button::standard(fl!("cancel")).on_press(Message::CloseCreatePartitionPanel))
        .push(
            widget::button::suggested(fl!("create-partition-confirm"))
                .on_press(Message::ConfirmCreatePartition),
        )
        .spacing(spacing.space_s);

    widget::column()
        .push(size_input)
        .push(fs_row)
        .push(label_input)
        .push(buttons)
        .spacing(spacing.space_s)
        .padding([spacing.space_s, spacing.space_m])
        .into()
}

fn detail_row<'a>(
    label: String,
    value: String,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    widget::row()
        .push(widget::text::body(label).width(Length::Fixed(110.0)))
        .push(widget::text::body(value).width(Length::Fill))
        .spacing(spacing.space_s)
        .align_y(Alignment::Center)
        .into()
}

fn format_panel_widget<'a>(
    fp: &'a FormatPanel,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> cosmic::Element<'a, Message> {
    let warning = widget::text::body(fl!("format-warning", device = fp.device.as_str()));

    let mut fs_row = widget::row().spacing(spacing.space_xs);
    for &fs in FsType::ALL.iter() {
        let btn = if fp.fs_type == fs {
            widget::button::suggested(fs.as_str())
        } else {
            widget::button::standard(fs.as_str())
        };
        fs_row = fs_row.push(btn.on_press(Message::FormatPanelFsChanged(fs)));
    }

    let label_input = widget::text_input(fl!("format-label-placeholder"), fp.label.as_str())
        .on_input(Message::FormatPanelLabelChanged)
        .width(Length::Fill);

    let buttons = widget::row()
        .push(widget::button::standard(fl!("cancel")).on_press(Message::CloseFormatPanel))
        .push(widget::button::destructive(fl!("format-confirm")).on_press(Message::ConfirmFormat))
        .spacing(spacing.space_s);

    widget::column()
        .push(warning)
        .push(fs_row)
        .push(label_input)
        .push(buttons)
        .spacing(spacing.space_s)
        .padding([spacing.space_s, spacing.space_m])
        .into()
}
