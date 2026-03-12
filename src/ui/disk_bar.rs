// SPDX-License-Identifier: GPL-3.0

use crate::actions::disks::BlockDevice;
use crate::message::{FsType, Message};
use cosmic::iced::{Color, Point, Rectangle, Size as IcedSize};
use cosmic::iced::mouse;
use cosmic::widget::canvas;

/// Minimum rendered and clickable width (px) per partition segment.
/// Keeps tiny partitions (e.g. a 1 MB EFI stub on a 2 TB disk) visible and clickable.
pub(crate) const MIN_SEGMENT_PX: f32 = 8.0;

#[derive(Default, PartialEq, Clone, Copy)]
pub(crate) enum Hover {
    #[default]
    None,
    Partition(usize),
    Unallocated,
}

#[derive(Default)]
pub(crate) struct DiskBarState {
    pub(crate) hover: Hover,
}

pub(crate) struct DiskBar<'a> {
    pub(crate) drive_id: &'a str,
    pub(crate) drive_size: u64,
    pub(crate) partitions: &'a [BlockDevice],
    pub(crate) selected_device: Option<&'a str>,
    pub(crate) selected_unallocated: bool,
}

impl<'a> DiskBar<'a> {
    /// Compute screen (x, w) for every partition in one pass.
    ///
    /// Uses the byte `offset` for natural placement, but a cumulative `min_x`
    /// ensures an inflated-to-MIN_SEGMENT_PX tiny partition is never covered by
    /// the next one.  Both `draw()` and `hit_test()` call this so visuals and
    /// click targets always agree.
    pub(crate) fn screen_rects(&self, bar_width: f32) -> Vec<(f32, f32)> {
        if self.drive_size == 0 || bar_width == 0.0 {
            return vec![(0.0, 0.0); self.partitions.len()];
        }
        let total = self.drive_size as f32;
        let mut rects = Vec::with_capacity(self.partitions.len());
        let mut min_x = 0.0_f32;

        for partition in self.partitions {
            if partition.size == 0 {
                rects.push((0.0, 0.0));
                continue;
            }
            let natural_x = partition.offset as f32 / total * bar_width;
            let natural_w = partition.size as f32 / total * bar_width;

            let x = natural_x.max(min_x);
            let w = natural_w.max(MIN_SEGMENT_PX);

            rects.push((x, w));
            min_x = x + w;
        }
        rects
    }

    pub(crate) fn hit_test(&self, x: f32, bar_width: f32) -> Hover {
        for (i, &(px, pw)) in self.screen_rects(bar_width).iter().enumerate() {
            if pw == 0.0 {
                continue;
            }
            if x >= px && x < px + pw {
                return Hover::Partition(i);
            }
        }
        Hover::Unallocated
    }
}

impl<'a> canvas::Program<Message, cosmic::Theme> for DiskBar<'a> {
    type State = DiskBarState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let new_hover = cursor
                    .position_in(bounds)
                    .map(|pos| self.hit_test(pos.x, bounds.width))
                    .unwrap_or(Hover::None);
                if new_hover != state.hover {
                    state.hover = new_hover;
                    return Some(canvas::Action::request_redraw());
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                match state.hover {
                    Hover::Partition(idx) => {
                        if let Some(partition) = self.partitions.get(idx) {
                            return Some(
                                canvas::Action::publish(Message::SelectPartition(
                                    partition.clone(),
                                ))
                                .and_capture(),
                            );
                        }
                    }
                    Hover::Unallocated => {
                        return Some(
                            canvas::Action::publish(Message::SelectUnallocated(
                                self.drive_id.to_string(),
                            ))
                            .and_capture(),
                        );
                    }
                    Hover::None => {}
                }
            }
            _ => {}
        }
        None
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.hover != Hover::None {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<cosmic::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        if self.drive_size == 0 || bounds.width == 0.0 {
            return vec![frame.into_geometry()];
        }

        let height = bounds.height;
        let is_unalloc_hovered = state.hover == Hover::Unallocated;

        // Fill entire bar as the unallocated background; partitions paint on top.
        let bg = if self.selected_unallocated {
            Color::from_rgba(0.65, 0.65, 0.70, 0.9)
        } else if is_unalloc_hovered {
            Color::from_rgba(0.55, 0.55, 0.60, 0.6)
        } else {
            Color::from_rgba(0.4, 0.4, 0.4, 0.3)
        };
        frame.fill_rectangle(Point::new(0.0, 0.0), IcedSize::new(bounds.width, height), bg);

        let rects = self.screen_rects(bounds.width);
        for (i, (partition, &(x, w))) in self.partitions.iter().zip(rects.iter()).enumerate() {
            if w == 0.0 {
                continue;
            }

            let is_hovered = state.hover == Hover::Partition(i);
            let is_selected = self.selected_device == Some(partition.device.as_str());

            let base = fs_color(&partition.fs_type);
            let color = if is_selected {
                Color::from_rgba(
                    (base.r * 1.3).min(1.0),
                    (base.g * 1.3).min(1.0),
                    (base.b * 1.3).min(1.0),
                    1.0,
                )
            } else if is_hovered {
                Color::from_rgba(
                    (base.r + 0.15).min(1.0),
                    (base.g + 0.15).min(1.0),
                    (base.b + 0.15).min(1.0),
                    base.a,
                )
            } else {
                base
            };

            frame.fill_rectangle(Point::new(x, 0.0), IcedSize::new(w, height), color);

            if is_selected {
                let path =
                    canvas::Path::rectangle(Point::new(x, 0.0), IcedSize::new(w, height));
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_color(Color::WHITE)
                        .with_width(2.0),
                );
            }
        }

        vec![frame.into_geometry()]
    }
}

fn fs_color(fs: &str) -> Color {
    // Known FsType variants use the enum's canonical colour; special-cased
    // filesystems that have no FsType variant (swap, zfs, iso9660) keep their
    // own entries here so they don't fall through to the grey default.
    if let Ok(t) = fs.parse::<FsType>() {
        return match t {
            FsType::Ext4  => Color::from_rgb(0.22, 0.53, 0.92),
            FsType::Btrfs => Color::from_rgb(0.18, 0.75, 0.50),
            FsType::Xfs   => Color::from_rgb(0.82, 0.44, 0.10),
            FsType::Vfat  => Color::from_rgb(0.92, 0.76, 0.10),
            FsType::Exfat | FsType::Ntfs => Color::from_rgb(0.00, 0.44, 0.76),
        };
    }
    match fs {
        "swap"    => Color::from_rgb(0.86, 0.24, 0.24),
        "zfs"     => Color::from_rgb(0.55, 0.24, 0.86),
        "iso9660" => Color::from_rgb(0.70, 0.70, 0.30),
        _         => Color::from_rgb(0.50, 0.50, 0.55),
    }
}
