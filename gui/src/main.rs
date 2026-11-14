#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod book;
mod theme;
mod wordlists;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, ensure};
use furigana4epub_core::{Options, download_dictionary, download_jlpt_wordlists};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, WindowExt as _,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    radio::RadioGroup,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use theme::ThemePreference;
use wordlists::Wordlists;

#[derive(Default)]
enum Status {
    #[default]
    Idle,
    UpdatingDictionary,
    DictionaryUpdated,
    ChoosingBook,
    ChoosingWordlists,
    Converting,
    ChoosingDestination,
    Saving,
    Saved(PathBuf),
    Failed(String),
}

impl Status {
    fn busy(&self) -> bool {
        matches!(
            self,
            Self::UpdatingDictionary
                | Self::ChoosingBook
                | Self::ChoosingWordlists
                | Self::Converting
                | Self::ChoosingDestination
                | Self::Saving
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Identifier {
    Keep,
    Generate,
    Custom,
}

impl Identifier {
    const MODES: [Self; 3] = [Self::Keep, Self::Generate, Self::Custom];
}

struct Converter {
    theme_preference: ThemePreference,
    input: Option<PathBuf>,
    override_existing: bool,
    highlight: bool,
    color: Entity<InputState>,
    identifier: Identifier,
    custom_identifier: Entity<InputState>,
    wordlists: Wordlists,
    output: Option<Arc<Vec<u8>>>,
    status: Status,
}

impl Converter {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme_preference = ThemePreference::load();
        theme_preference.apply(window, cx);
        cx.observe_window_appearance(window, |view, window, cx| {
            if view.theme_preference == ThemePreference::System {
                view.theme_preference.apply(window, cx);
            }
        })
        .detach();

        let mut converter = Self {
            theme_preference,
            input: None,
            override_existing: false,
            highlight: false,
            color: cx.new(|cx| InputState::new(window, cx).placeholder("#c2410c")),
            identifier: Identifier::Keep,
            custom_identifier: cx
                .new(|cx| InputState::new(window, cx).placeholder("urn:example:my-book")),
            wordlists: Wordlists::default(),
            output: None,
            status: Status::Idle,
        };
        match wordlists::read_shared() {
            Ok(shared) => converter.wordlists.replace_shared(shared),
            Err(error) => converter.fail(error.context("Could not load shared wordlists")),
        }
        converter
    }

    fn update_dictionary(&mut self, cx: &mut Context<Self>) {
        if self.status.busy() {
            return;
        }
        self.status = Status::UpdatingDictionary;
        cx.notify();

        let update = cx.background_executor().spawn(async {
            download_dictionary()?;
            download_jlpt_wordlists()
                .context("The dictionary was updated. Could not update the JLPT wordlists.")?;
            wordlists::read_shared().context("Could not reload shared wordlists")
        });
        cx.spawn(async move |this, cx| {
            let result = update.await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(shared) => {
                        view.wordlists.replace_shared(shared);
                        view.output = None;
                        view.status = Status::DictionaryUpdated;
                    }
                    Err(error) => view.fail(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_book(&mut self, cx: &mut Context<Self>) {
        if self.status.busy() {
            return;
        }
        self.status = Status::ChoosingBook;
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose an EPUB 3 book".into()),
        });
        cx.notify();

        cx.spawn(async move |this, cx| {
            let selected = async {
                let paths = selection
                    .await
                    .context("The open dialog closed unexpectedly")??;
                let path = paths.and_then(|paths| paths.into_iter().next());
                if let Some(path) = &path {
                    ensure!(
                        book::is_epub(path),
                        "Choose a file with the .epub extension."
                    );
                }
                Ok::<_, anyhow::Error>(path)
            }
            .await;

            let _ = this.update(cx, |view, cx| {
                view.status = Status::Idle;
                match selected {
                    Ok(Some(path)) => {
                        view.input = Some(path);
                        view.output = None;
                    }
                    Ok(None) => {}
                    Err(error) => view.fail(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_wordlists(&mut self, cx: &mut Context<Self>) {
        if self.status.busy() {
            return;
        }
        self.status = Status::ChoosingWordlists;
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Choose wordlist files".into()),
        });
        cx.notify();

        cx.spawn(async move |this, cx| {
            let selected = selection
                .await
                .context("The open dialog closed unexpectedly")
                .flatten();
            let _ = this.update(cx, |view, cx| {
                view.status = Status::Idle;
                match selected {
                    Ok(Some(paths)) => {
                        for path in paths {
                            if view.wordlists.add(path) {
                                view.output = None;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(error) => view.fail(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn convert(&mut self, cx: &mut Context<Self>) {
        if self.status.busy() {
            return;
        }
        let Some(input) = self.input.clone() else {
            return;
        };
        let override_existing = self.override_existing;
        let highlight_injected = self.highlight;
        let color = self.color.read(cx).value();
        let identifier = self.identifier;
        let custom_identifier = self.custom_identifier.read(cx).value();
        let ignore_wordlists = self.wordlists.ignored_paths();

        if identifier == Identifier::Custom && custom_identifier.trim().is_empty() {
            self.status = Status::Failed("Enter a custom EPUB identifier.".into());
            cx.notify();
            return;
        }

        self.status = Status::Converting;
        self.output = None;
        cx.notify();

        let conversion = cx.background_executor().spawn(async move {
            let color = color.trim();
            book::convert(
                &input,
                Options {
                    override_existing,
                    highlight_injected,
                    highlight_color: (highlight_injected && !color.is_empty()).then_some(color),
                    new_identifier: identifier == Identifier::Generate,
                    identifier: (identifier == Identifier::Custom)
                        .then_some(custom_identifier.trim()),
                    ignore_wordlists: &ignore_wordlists,
                },
            )
        });
        cx.spawn(async move |this, cx| {
            let result = conversion.await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(bytes) => {
                        view.output = Some(Arc::new(bytes));
                        view.status = Status::Idle;
                        view.choose_destination(cx);
                    }
                    Err(error) => view.fail(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_destination(&mut self, cx: &mut Context<Self>) {
        if self.status.busy() {
            return;
        }
        let (Some(input), Some(bytes)) = (&self.input, self.output.clone()) else {
            return;
        };
        let suggested = input.with_extension("furigana.epub");
        let name = suggested
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("book.furigana.epub");
        let directory = input.parent().unwrap_or(Path::new("."));
        let destination = cx.prompt_for_new_path(directory, Some(name));
        self.status = Status::ChoosingDestination;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = async {
                let Some(mut path) = destination
                    .await
                    .context("The save dialog closed unexpectedly")??
                else {
                    return Ok(None);
                };
                if path.extension().is_none() {
                    path.set_extension("epub");
                }
                this.update(cx, |view, cx| {
                    view.status = Status::Saving;
                    cx.notify();
                })?;
                cx.background_executor()
                    .spawn(async move {
                        book::save(&path, &bytes)?;
                        Ok::<_, anyhow::Error>(Some(path))
                    })
                    .await
            }
            .await;

            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(Some(path)) => {
                        view.output = None;
                        view.status = Status::Saved(path);
                    }
                    Ok(None) => view.status = Status::Idle,
                    Err(error) => view.fail(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn fail(&mut self, error: anyhow::Error) {
        self.status = Status::Failed(format!("{error:#}"));
    }

    fn set_theme(
        &mut self,
        preference: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.theme_preference = preference;
        preference.apply(window, cx);
        if let Err(error) = preference.save() {
            window.push_notification(
                Notification::error(format!("Could not save the theme preference. {error}")),
                cx,
            );
        }
        cx.notify();
    }

    fn theme_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let current = self.theme_preference;
        let view = cx.weak_entity();
        Button::new("theme")
            .outline()
            .label("Theme")
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let mut menu = menu;
                for choice in ThemePreference::CHOICES {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(choice.label())
                            .checked(choice == current)
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |view, cx| {
                                    view.set_theme(choice, window, cx);
                                });
                            }),
                    );
                }
                menu
            })
    }

    fn book_picker(&self, panel: Div, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (icon_background, icon_foreground) = if theme.mode.is_dark() {
            (rgb(0x38324b), rgb(0xd3c4ff))
        } else {
            (rgb(0xede7fb), rgb(0x594290))
        };
        let input_name = self.input.as_deref().and_then(Path::file_name).map_or_else(
            || "No EPUB selected".into(),
            |name| name.to_string_lossy().into_owned(),
        );

        panel
            .flex_row()
            .items_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_10()
                    .flex_shrink_0()
                    .rounded_lg()
                    .bg(icon_background)
                    .child(Icon::new(assets::IconName::BookOpen).text_color(icon_foreground)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Source book"),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(input_name),
                    ),
            )
            .child(
                Button::new("choose-book")
                    .outline()
                    .label("Choose EPUB…")
                    .disabled(self.status.busy())
                    .on_click(cx.listener(|view, _, _, cx| view.choose_book(cx))),
            )
    }

    fn conversion_options(&self, panel: Div, cx: &Context<Self>) -> impl IntoElement {
        let busy = self.status.busy();
        panel
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Conversion options"),
            )
            .child(
                Checkbox::new("replace-ruby")
                    .label("Replace existing furigana when readable")
                    .checked(self.override_existing)
                    .disabled(busy)
                    .on_change(cx.listener(|view, checked, _, cx| {
                        view.override_existing = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("highlight")
                    .label("Highlight added furigana")
                    .checked(self.highlight)
                    .disabled(busy)
                    .on_change(cx.listener(|view, checked, _, cx| {
                        view.highlight = *checked;
                        cx.notify();
                    })),
            )
            .when(self.highlight, |panel| {
                panel.child(
                    h_flex()
                        .gap_3()
                        .pl_6()
                        .child(div().text_sm().child("Highlight color"))
                        .child(
                            div().w(px(120.)).child(
                                Input::new(&self.color)
                                    .aria_label("Highlight color (CSS hex)")
                                    .disabled(busy),
                            ),
                        ),
                )
            })
            .child(
                v_flex()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("EPUB identifier"),
                    )
                    .child(
                        RadioGroup::horizontal("identifier")
                            .children(["Keep original", "Generate UUID", "Use custom identifier"])
                            .selected_index(
                                Identifier::MODES
                                    .iter()
                                    .position(|mode| *mode == self.identifier),
                            )
                            .disabled(busy)
                            .on_change(cx.listener(|view, index, _, cx| {
                                view.identifier = Identifier::MODES[*index];
                                cx.notify();
                            })),
                    )
                    .when(self.identifier == Identifier::Custom, |section| {
                        section.child(
                            Input::new(&self.custom_identifier)
                                .aria_label("Custom EPUB identifier")
                                .disabled(busy),
                        )
                    }),
            )
    }

    fn wordlist_options(&self, panel: Div, cx: &Context<Self>) -> impl IntoElement {
        let busy = self.status.busy();
        panel
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Ignored wordlists"),
                    )
                    .child(
                        Button::new("add-wordlists")
                            .outline()
                            .label("Add wordlist files…")
                            .disabled(busy)
                            .on_click(cx.listener(|view, _, _, cx| view.choose_wordlists(cx))),
                    ),
            )
            .when(self.wordlists.is_empty(), |panel| {
                panel.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No wordlists available"),
                )
            })
            .when(!self.wordlists.is_empty(), |panel| {
                panel.child(
                    v_flex()
                        .w_full()
                        .max_h(px(144.))
                        .gap_2()
                        .p_1()
                        .overflow_y_scrollbar()
                        .children(self.wordlists.iter().map(|(path, wordlist)| {
                            let label = wordlist.label(path);
                            let path_id: Arc<Path> = Arc::from(path.as_path());
                            let tooltip = path.display().to_string();
                            Checkbox::new(path_id.clone())
                                .w_full()
                                .checked(wordlist.ignored)
                                .disabled(busy)
                                .accessibility_label(label.clone())
                                .tooltip(tooltip)
                                .child(div().truncate().child(label))
                                .on_change(cx.listener(move |view, checked, _, cx| {
                                    view.wordlists.set_ignored(&path_id, *checked);
                                    view.output = None;
                                    cx.notify();
                                }))
                        })),
                )
            })
    }

    fn footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let busy = self.status.busy();
        let has_output = self.output.is_some();
        let (message, color) = match &self.status {
            Status::Idle if has_output => {
                ("Converted EPUB ready to save.".into(), theme.foreground)
            }
            Status::Idle => ("".into(), theme.muted_foreground),
            Status::UpdatingDictionary => (
                "Downloading the Sudachi dictionary and JLPT wordlists…".into(),
                theme.muted_foreground,
            ),
            Status::DictionaryUpdated => (
                "Dictionary and JLPT wordlists updated.".into(),
                theme.foreground,
            ),
            Status::ChoosingBook => ("Waiting for an EPUB…".into(), theme.muted_foreground),
            Status::ChoosingWordlists => ("Waiting for wordlists…".into(), theme.muted_foreground),
            Status::Converting => ("Adding furigana…".into(), theme.muted_foreground),
            Status::ChoosingDestination => {
                ("Choose where to save the EPUB.".into(), theme.foreground)
            }
            Status::Saving => ("Saving EPUB…".into(), theme.muted_foreground),
            Status::Saved(path) => (format!("Saved to {}", path.display()), theme.foreground),
            Status::Failed(error) => (error.clone(), theme.danger),
        };

        h_flex()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(color)
                    .child(message),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .child(
                        Button::new("convert")
                            .when(has_output, |button| button.outline())
                            .when(!has_output, |button| button.primary())
                            .label("Convert EPUB")
                            .loading(matches!(self.status, Status::Converting))
                            .disabled(self.input.is_none() || busy)
                            .on_click(cx.listener(|view, _, _, cx| view.convert(cx))),
                    )
                    .when(has_output, |row| {
                        row.child(
                            Button::new("save")
                                .primary()
                                .label("Save EPUB…")
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.choose_destination(cx)),
                                ),
                        )
                    }),
            )
    }
}

impl Render for Converter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, panel) = if theme.mode.is_dark() {
            (rgb(0x1b1b20), rgb(0x25252d))
        } else {
            (rgb(0xf6f5fa), rgb(0xffffff))
        };
        let card = || {
            v_flex()
                .gap_3()
                .p_4()
                .rounded_xl()
                .bg(panel)
                .border_1()
                .border_color(theme.border)
        };

        div()
            .id("converter")
            .size_full()
            .overflow_y_scroll()
            .bg(background)
            .text_color(theme.foreground)
            .child(
                v_flex()
                    .mx_auto()
                    .w_full()
                    .max_w(px(680.))
                    .px_6()
                    .py_6()
                    .gap_5()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_3xl()
                                            .font_weight(FontWeight::BOLD)
                                            .child("Add furigana"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.muted_foreground)
                                            .child("Convert a Japanese EPUB 3 book."),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .flex_shrink_0()
                                    .gap_2()
                                    .child(self.theme_menu(cx))
                                    .child(
                                        Button::new("update-dictionary")
                                            .outline()
                                            .label("Update Dictionary")
                                            .loading(matches!(
                                                self.status,
                                                Status::UpdatingDictionary
                                            ))
                                            .disabled(self.status.busy())
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.update_dictionary(cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(self.book_picker(card(), cx))
                    .child(self.conversion_options(card(), cx))
                    .child(self.wordlist_options(card(), cx))
                    .child(self.footer(cx)),
            )
    }
}

fn main() {
    application()
        .with_assets(assets::Assets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            init(cx);
            open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(600.), px(715.)), cx)),
                    window_min_size: Some(size(px(460.), px(460.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Furigana for EPUB".into()),
                        ..Default::default()
                    }),
                    app_id: Some("furigana4epub".into()),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Converter::new(window, cx)),
            )
            .expect("Could not open the converter window");
            cx.activate(true);
        });
}
