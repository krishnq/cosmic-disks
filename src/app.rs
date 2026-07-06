// SPDX-License-Identifier: GPL-3.0

use std::any::TypeId;
use std::collections::HashMap;

use cosmic::app::context_drawer;
use cosmic::cosmic_config::CosmicConfigEntry;
use cosmic::iced::alignment::Horizontal;
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::prelude::*;
use cosmic::widget::{self, about::About, menu};

use crate::config::{Config, CONFIG_VERSION};
use crate::fl;
use crate::message::{ActiveDialog, ContextPage, LoadState, Message, MountFlag, SelectionState};
use crate::ui::disk_card::disk_card;

const REPOSITORY: &str = "https://github.com/krishnqs/cosmic-disks";
const APP_ICON: &[u8] = include_bytes!("../resources/icons/hicolor/scalable/apps/icon.svg");

pub struct AppModel {
    pub(crate) core: cosmic::Core,
    pub(crate) context_page: ContextPage,
    about: About,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    pub load_state: LoadState,
    pub selection_state: SelectionState,
    pub active_dialog: ActiveDialog,
    pub config: Config,
    pub config_handler: Option<cosmic::cosmic_config::Config>,
    pub operation_error: Option<String>,
    /// Incremented on every new error so a stale auto-dismiss timer is ignored.
    pub error_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MenuAction {
    About,
    Refresh,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
            MenuAction::Refresh => Message::RefreshDisks,
        }
    }
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "dev.krishnqs.CosmicDisks";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let about = About::default()
            .name(fl!("app-title"))
            .icon(widget::icon::from_svg_bytes(APP_ICON))
            .version(env!("CARGO_PKG_VERSION"))
            .links([(fl!("repository"), REPOSITORY)])
            .license(env!("CARGO_PKG_LICENSE"));

        let (config_handler, config) =
            match cosmic::cosmic_config::Config::new(Self::APP_ID, CONFIG_VERSION) {
                Ok(handler) => {
                    let config = Config::get_entry(&handler).unwrap_or_default();
                    (Some(handler), config)
                }
                Err(e) => {
                    log::warn!("failed to open config: {e}");
                    (None, Config::default())
                }
            };

        let app = AppModel {
            core,
            context_page: ContextPage::default(),
            about,
            key_binds: HashMap::new(),
            load_state: LoadState::Scanning,
            selection_state: SelectionState::None,
            active_dialog: ActiveDialog::None,
            config,
            config_handler,
            operation_error: None,
            error_epoch: 0,
        };

        let command = cosmic::task::future(async {
            cosmic::Action::App(match crate::actions::disks::scan_drives().await {
                Ok(drives) => Message::DisksScanned(Ok(drives)),
                Err(e) => Message::DisksScanned(Err(e.to_string())),
            })
        });

        (app, command)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let udisks = crate::actions::udisks_watch::subscription();

        let config = cosmic::cosmic_config::config_subscription(
            TypeId::of::<Config>(),
            Self::APP_ID.into(),
            CONFIG_VERSION,
        )
        .map(|update: cosmic::cosmic_config::Update<Config>| {
            if !update.errors.is_empty() {
                log::warn!("config update errors: {:?}", update.errors);
            }
            Message::ConfigUpdate(update.config)
        });

        Subscription::batch([udisks, config])
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        let menu_bar = menu::bar(vec![menu::Tree::with_children(
            menu::root(fl!("view")).apply(Element::from),
            menu::items(
                &self.key_binds,
                vec![
                    menu::Item::Button(fl!("refresh"), None, MenuAction::Refresh),
                    menu::Item::Divider,
                    menu::Item::Button(fl!("about"), None, MenuAction::About),
                ],
            ),
        )]);

        vec![menu_bar.into()]
    }

    fn header_center(&self) -> Vec<Element<'_, Self::Message>> {
        vec![widget::text::heading(fl!("app-title")).into()]
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_string()),
                Message::ToggleContextPage(ContextPage::About),
            ),
        })
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        let spacing = cosmic::theme::spacing();

        match &self.active_dialog {
            ActiveDialog::None => None,

            ActiveDialog::Mount(d) => {
                let path_row = widget::row()
                    .push(
                        widget::text_input(fl!("mount-path-placeholder"), d.path.as_str())
                            .on_input(Message::MountDialogPathChanged)
                            .width(Length::Fill),
                    )
                    .push(
                        widget::button::standard(fl!("browse")).on_press(Message::BrowseMountPath),
                    )
                    .spacing(spacing.space_s)
                    .align_y(cosmic::iced::Alignment::Center);

                let adv_label = if d.show_advanced {
                    fl!("hide-advanced")
                } else {
                    fl!("advanced-options")
                };

                let adv_toggle = widget::button::standard(adv_label)
                    .on_press(Message::ToggleMountDialogAdvanced);

                let fstab_checkbox = if d.already_in_fstab {
                    widget::checkbox(true).label(fl!("mount-already-in-fstab"))
                } else {
                    widget::checkbox(d.add_to_fstab)
                        .label(fl!("mount-add-to-fstab"))
                        .on_toggle(|_| Message::ToggleFstabCheckbox)
                };

                let mut controls = widget::column()
                    .push(path_row)
                    .push(fstab_checkbox)
                    .push(adv_toggle)
                    .spacing(spacing.space_s);

                if d.show_advanced {
                    let mut flags_row = widget::row().spacing(spacing.space_xs);
                    for &flag in MountFlag::ALL.iter() {
                        let active = d.selected_flags.contains(&flag);
                        let btn = if active {
                            widget::button::suggested(flag.label())
                        } else {
                            widget::button::standard(flag.label())
                        };
                        flags_row =
                            flags_row.push(btn.on_press(Message::ToggleDialogMountFlag(flag)));
                    }
                    controls = controls.push(flags_row);
                }

                Some(
                    widget::dialog()
                        .title(fl!("mount-location-title"))
                        .control(controls)
                        .primary_action(
                            widget::button::suggested(fl!("mount"))
                                .on_press(Message::ConfirmMountDialog),
                        )
                        .secondary_action(
                            widget::button::standard(fl!("cancel"))
                                .on_press(Message::CloseMountDialog),
                        )
                        .into(),
                )
            }

            ActiveDialog::Unmount {
                device,
                mount_points,
            } => {
                let device = device.as_str();
                let body = if mount_points.is_empty() {
                    fl!("unmount-confirm-body-no-path", device = device)
                } else {
                    fl!(
                        "unmount-confirm-body",
                        device = device,
                        path = mount_points.join(", ")
                    )
                };

                Some(
                    widget::dialog()
                        .title(fl!("unmount-confirm-title", device = device))
                        .control(widget::text::body(body))
                        .primary_action(
                            widget::button::destructive(fl!("unmount"))
                                .on_press(Message::ConfirmUnmount),
                        )
                        .secondary_action(
                            widget::button::standard(fl!("cancel"))
                                .on_press(Message::CloseUnmountDialog),
                        )
                        .into(),
                )
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        match &self.load_state {
            LoadState::Scanning => self.view_scanning(),
            LoadState::Error(_) => self.view_error(),
            LoadState::Ready(drives) if drives.is_empty() => self.view_no_drives(),
            LoadState::Ready(_) => self.view_drives(),
        }
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        self.handle(message)
    }
}

impl AppModel {
    fn view_scanning(&self) -> Element<'_, Message> {
        widget::column()
            .push(widget::text::title3(fl!("scanning")))
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_error(&self) -> Element<'_, Message> {
        let LoadState::Error(ref error) = self.load_state else {
            return self.view_scanning();
        };
        let spacing = cosmic::theme::spacing();
        widget::column()
            .push(widget::text::title3(fl!("error")))
            .push(widget::text::body(error.as_str()))
            .push(widget::button::standard(fl!("refresh")).on_press(Message::RefreshDisks))
            .spacing(spacing.space_m)
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .into()
    }

    fn view_no_drives(&self) -> Element<'_, Message> {
        widget::column()
            .push(widget::text::title3(fl!("no-drives")))
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_drives(&self) -> Element<'_, Message> {
        let LoadState::Ready(ref drives) = self.load_state else {
            return self.view_scanning();
        };
        let spacing = cosmic::theme::spacing();
        let mut outer = widget::column();

        if let Some(ref err) = self.operation_error {
            outer = outer.push(
                widget::row()
                    .push(widget::text::body(err.as_str()).width(Length::Fill))
                    .push(widget::button::standard(fl!("dismiss")).on_press(Message::DismissError))
                    .spacing(spacing.space_s)
                    .align_y(Alignment::Center)
                    .apply(widget::container)
                    .padding([spacing.space_s, spacing.space_m])
                    .class(cosmic::theme::Container::Card),
            );
        }

        let mut list = widget::column().spacing(spacing.space_xs);
        for drive in drives {
            list = list.push(disk_card(drive, &self.selection_state));
        }
        list = list.push(
            widget::button::standard(fl!("refresh"))
                .on_press(Message::RefreshDisks)
                .apply(widget::container)
                .width(Length::Fill)
                .align_x(Horizontal::Center),
        );

        outer
            .push(
                widget::scrollable(list.padding(spacing.space_m))
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
