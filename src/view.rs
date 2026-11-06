//! Everything on screen.

use crate::app::{
    App, FILTER_INPUT, GLYPHS_PER_PAGE, Message, Mode, ROW_HEIGHT, SAMPLE_INPUT, SIDEBAR_HEAD,
    SPECIMEN,
};
use crate::catalog::{Details, Face, Family};
use crate::palette::{Palette, alpha, mix};

use iced::border::{self, Border};
use iced::font::{self, Font};
use iced::widget::text::{LineHeight, Shaping, Wrapping};
use iced::widget::{
    Column, Row, button, center, column, container, mouse_area, opaque, row, scrollable, sensor,
    space, stack, text, text_input,
};
use iced::{Center, Color, Element, Fill, FillPortion, Padding, Theme};

const SIDEBAR_WIDTH: f32 = 300.0;
const STATUS_HEIGHT: f32 = 28.0;
const TOOLBAR_HEIGHT: f32 = 60.0;

const MONO: Font = Font::MONOSPACE;
const MONO_BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::MONOSPACE
};

/// Sizes of the waterfall, in pixels at 100% zoom.
const WATERFALL: &[f32] = &[96.0, 72.0, 56.0, 44.0, 34.0, 26.0, 20.0, 16.0, 13.0, 11.0];

pub fn view(app: &App) -> Element<'_, Message> {
    let p = app.palette;

    let content: Element<'_, Message> = if let Some(error) = &app.error {
        problem(app, "This file could not be opened", error)
    } else if app.browse {
        row![
            sidebar(app),
            // The divider lights up while the keyboard is inside the specimen.
            rule_vertical(if app.inside {
                p.accent
            } else {
                alpha(p.border, 0.6)
            }),
            specimen_pane(app)
        ]
        .into()
    } else {
        specimen_pane(app)
    };

    let body = sensor(container(content).width(Fill).height(Fill))
        .on_show(Message::BodyResized)
        .on_resize(Message::BodyResized);

    let page = container(column![body, status_bar(app)])
        .width(Fill)
        .height(Fill)
        .style(move |_| container::Style {
            background: Some(p.bg.into()),
            text_color: Some(p.text),
            ..container::Style::default()
        });

    if app.help {
        stack![page, help(app)].into()
    } else {
        page.into()
    }
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

fn sidebar(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let total = app.visible.len();
    let rows = app.rows();

    let filtered = !app.filter.is_empty() || app.monospace_only;
    let count = if filtered {
        format!("{total} of {}", app.families.len())
    } else {
        total.to_string()
    };

    let mut title = row![label("Fonts", p.muted)].spacing(8).align_y(Center);
    if app.monospace_only {
        title = title.push(tag("mono", p.accent3, p));
    }

    let header = container(
        row![title, space::horizontal(), label(count, p.faint)].align_y(Center),
    )
    .height(48)
    .padding(Padding::ZERO.left(20).right(20))
    .align_y(Center);

    let filter = container(
        text_input("/  filter", &app.filter)
            .id(FILTER_INPUT)
            .on_input(Message::FilterChanged)
            .on_submit(Message::FilterSubmitted)
            .size(13)
            .padding([8, 12])
            .style(input_style(p, p.bg)),
    )
    .height(SIDEBAR_HEAD - 48.0)
    .padding(Padding::ZERO.left(14).right(14));

    let list = Column::with_children(
        (app.top..(app.top + rows).min(total)).map(|position| family_row(app, position)),
    )
    .padding(Padding::ZERO.left(8).right(2));

    let list = mouse_area(
        row![
            container(list).width(Fill).height(Fill).clip(true),
            list_scrollbar(app)
        ]
        .height(Fill),
    )
    .on_scroll(Message::ListScrolled);

    container(column![header, filter, list])
        .width(SIDEBAR_WIDTH)
        .height(Fill)
        .style(fill(p.surface))
        .into()
}

fn family_row(app: &App, position: usize) -> Element<'_, Message> {
    let p = app.palette;
    let family = &app.families[app.visible[position]];
    let selected = position == app.cursor;

    let marker = container(space())
        .width(3)
        .height(16)
        .style(rounded(
            match (selected, app.inside) {
                (true, false) => p.accent,
                (true, true) => p.faint,
                _ => Color::TRANSPARENT,
            },
            2.0,
        ));

    let name = text(&family.name)
        .font(if family.legible {
            family.faces[family.regular].font
        } else {
            Font::DEFAULT
        })
        .size(15)
        .shaping(Shaping::Advanced)
        .wrapping(Wrapping::None)
        .color(if selected {
            p.text
        } else if app.inside {
            // The list recedes while the keyboard is inside the specimen.
            p.faint
        } else {
            mix(p.muted, p.text, 0.4)
        });

    let styles = text(family.faces.len().to_string())
        .size(10)
        .font(MONO)
        .color(if selected { p.accent } else { p.faint });

    button(
        // The row must fill the button's height, or it sits at the top of
        // the highlight instead of in its middle.
        row![marker, container(name).width(Fill).clip(true), styles]
            .spacing(10)
            .height(Fill)
            .align_y(Center),
    )
    .width(Fill)
    .height(ROW_HEIGHT)
    .padding(Padding::ZERO.left(6).right(12))
    .on_press(Message::Select(position))
    .style(move |_, status| button::Style {
        background: match status {
            _ if selected => Some(p.raised.into()),
            button::Status::Hovered | button::Status::Pressed => {
                Some(alpha(p.raised, 0.55).into())
            }
            _ => None,
        },
        text_color: p.text,
        border: border::rounded(7),
        ..button::Style::default()
    })
    .into()
}

/// A position indicator for the (virtualised) family list.
fn list_scrollbar(app: &App) -> Element<'_, Message> {
    let p = app.palette;
    let total = app.visible.len();
    let rows = app.rows();
    let track = app.list_height() - 8.0;

    let (offset, length) = if total > rows {
        let length = (track * rows as f32 / total as f32).max(28.0);
        let progress = app.top as f32 / (total - rows) as f32;
        (progress * (track - length), length)
    } else {
        (0.0, 0.0)
    };

    container(column![
        space().height(offset),
        container(space())
            .width(3)
            .height(length)
            .style(rounded(p.border, 2.0)),
    ])
    .width(8)
    .height(Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Specimen
// ---------------------------------------------------------------------------

fn specimen_pane(app: &App) -> Element<'_, Message> {
    let p = app.palette;

    let Some(family) = app.family() else {
        return column![
            toolbar(app),
            problem(app, "No fonts match", "Press Esc to clear the filter."),
        ]
        .into();
    };

    let specimen = scrollable(specimen(app, family))
        .id(SPECIMEN)
        .width(Fill)
        .height(Fill)
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::new()
                .width(5)
                .scroller_width(5)
                .margin(4),
        ))
        .style(move |theme, status| {
            let mut style = scrollable::default(theme, status);
            let active = matches!(
                status,
                scrollable::Status::Hovered {
                    is_vertical_scrollbar_hovered: true,
                    ..
                } | scrollable::Status::Dragged {
                    is_vertical_scrollbar_dragged: true,
                    ..
                }
            );

            style.container = container::Style::default();
            style.gap = None;
            style.vertical_rail.background = None;
            style.vertical_rail.border = Border::default();
            style.vertical_rail.scroller.background =
                if active { p.muted } else { p.border }.into();
            style.vertical_rail.scroller.border = border::rounded(3);
            style
        });

    column![toolbar(app), specimen].into()
}

/// The fixed strip above the specimen: sample text, zoom and colours.
fn toolbar(app: &App) -> Element<'_, Message> {
    let p = app.palette;

    let sample = text_input("Type to preview your own text  (i)", &app.sample)
        .id(SAMPLE_INPUT)
        .on_input(Message::SampleChanged)
        .on_submit(Message::SampleSubmitted)
        .size(13)
        .padding([8, 12])
        .style(input_style(p, p.surface));

    let zoom = row![
        ghost_button("\u{2212}", p).on_press(Message::Zoom(-1)),
        button(
            text(format!("{:.0}%", app.scale * 100.0))
                .size(11)
                .font(MONO)
                .width(44)
                .center()
        )
        .padding([8, 0])
        .on_press(Message::Zoom(0))
        .style(ghost(p)),
        ghost_button("+", p).on_press(Message::Zoom(1)),
    ]
    .align_y(Center);

    let dot = |color: Color| container(space()).width(9).height(9).style(rounded(color, 5.0));

    let shuffle = button(
        row![
            row![dot(p.accent), dot(p.accent2), dot(p.accent3)].spacing(4),
            text("Shuffle").size(13),
        ]
        .spacing(10)
        .align_y(Center),
    )
    .padding([8, 16])
    .on_press(Message::Shuffle)
    .style(move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(if hovered { p.raised } else { p.surface }.into()),
            text_color: p.text,
            border: Border {
                color: if hovered { p.accent } else { p.border },
                width: 1.0,
                radius: 99.0.into(),
            },
            ..button::Style::default()
        }
    });

    container(row![sample, zoom, shuffle].spacing(14).align_y(Center))
        .height(TOOLBAR_HEIGHT)
        .padding(Padding::ZERO.left(40).right(28))
        .align_y(Center)
        .into()
}

fn specimen<'a>(app: &'a App, family: &'a Family) -> Element<'a, Message> {
    let p = app.palette;
    let face = family.face();
    let details = face.details.as_deref();
    let scale = app.scale;

    // Text in the face on display.
    let set = move |content: String, size: f32| {
        text(content)
            .font(face.font)
            .size(size)
            .shaping(Shaping::Advanced)
    };

    let custom = app.sample.trim();
    let pangram = if custom.is_empty() {
        details.map(Details::pangram).unwrap_or_default()
    } else {
        custom.to_owned()
    };

    let mut page = Column::new()
        .spacing(44)
        .padding(Padding::new(40.0).top(20).bottom(72));

    // -- Heading -----------------------------------------------------------
    let mut heading = column![
        set(face.family.clone(), 56.0)
            .font(if family.legible { face.font } else { Font::DEFAULT })
            .color(p.accent)
            .line_height(LineHeight::Relative(1.15)),
        facts(face, details, p),
    ]
    .spacing(10);

    if family.faces.len() > 1 {
        heading = heading.push(styles(family, p));
    }

    page = page.push(heading);

    // -- Hero --------------------------------------------------------------
    if let Some(details) = details {
        let lines = details.alphabet();
        let last = lines.len().saturating_sub(1);

        let alphabet = Column::with_children(lines.into_iter().enumerate().map(|(index, line)| {
            set(line, 30.0)
                .wrapping(Wrapping::None)
                .color(if index == last && index > 3 {
                    p.accent3
                } else {
                    p.text
                })
                .into()
        }))
        .spacing(6);

        page = page.push(
            row![
                set(details.hero(), 200.0)
                    .color(p.accent2)
                    .wrapping(Wrapping::None)
                    .line_height(LineHeight::Relative(1.05)),
                alphabet,
            ]
            .spacing(56)
            .align_y(Center)
            .wrap(),
        );
    }

    // -- Icons -------------------------------------------------------------
    // Icon fonts keep their icons in the private use areas, far behind the
    // letters; surface a sample so they are not buried in the glyph grid.
    // Text fonts also park spare parts there (bracket pieces, or Inter's 745
    // alternates), so only call them icons when they dominate the font or
    // there are thousands of them, as in a patched Nerd Font.
    if let Some(details) = details.filter(|details| {
        details.icon_count >= 2000 || details.icon_count * 2 >= details.chars.len().max(96)
    }) {
        let icons = details.icons.iter().map(|&c| {
            container(set(c.to_string(), 30.0).wrapping(Wrapping::None))
                .width(46)
                .height(46)
                .align_x(Center)
                .align_y(Center)
                .clip(true)
                .into()
        });

        let all = details
            .blocks
            .iter()
            .position(|block| block.name.starts_with("Icons"))
            .map(|index| {
                button(label(
                    format!("{} in the glyph grid", thousands(details.icon_count)),
                    p.muted,
                ))
                .padding([4, 8])
                .on_press(Message::GlyphBlock(index))
                .style(ghost(p))
                .into()
            });

        page = page.push(section(
            "Icons",
            all,
            p,
            Row::with_children(icons).spacing(4).wrap().vertical_spacing(4),
        ));
    }

    // -- Waterfall ---------------------------------------------------------
    let mut waterfall = Column::new().spacing(6);
    for size in WATERFALL {
        waterfall = waterfall.push(
            row![
                text(format!("{size:.0}"))
                    .size(10)
                    .font(MONO)
                    .color(p.accent3)
                    .width(34),
                container(
                    set(pangram.clone(), size * scale)
                        .wrapping(Wrapping::None)
                        .line_height(LineHeight::Relative(1.25)),
                )
                .width(Fill)
                .clip(true),
            ]
            .align_y(Center),
        );
    }
    page = page.push(section("Waterfall", None, p, waterfall));

    // -- Running text ------------------------------------------------------
    let paragraph = if custom.is_empty() {
        details.map(Details::paragraph).unwrap_or_default()
    } else {
        [custom; 12].join(" ")
    };

    page = page.push(section(
        "Text",
        None,
        p,
        row![
            set(paragraph.clone(), 20.0 * scale)
                .line_height(LineHeight::Relative(1.5))
                .width(FillPortion(3)),
            set(paragraph, 13.0 * scale)
                .line_height(LineHeight::Relative(1.6))
                .color(mix(p.muted, p.text, 0.5))
                .width(FillPortion(2)),
        ]
        .spacing(48),
    ));

    // -- Glyphs ------------------------------------------------------------
    if let Some(details) = details.filter(|details| !details.chars.is_empty()) {
        let total = details.chars.len();
        let start = app.glyph_start.min(total.saturating_sub(1));
        let end = (start + GLYPHS_PER_PAGE).min(total);
        let current = details
            .blocks
            .iter()
            .rposition(|block| block.start <= start);

        let cells = details.chars[start..end].iter().map(|&c| {
            container(
                column![
                    set(c.to_string(), 28.0)
                        .wrapping(Wrapping::None)
                        .line_height(LineHeight::Absolute(38.0.into())),
                    text(format!("{:04X}", c as u32))
                        .size(9)
                        .font(MONO)
                        .color(p.faint),
                ]
                .spacing(4)
                .align_x(Center),
            )
            .width(66)
            .height(76)
            .align_x(Center)
            .align_y(Center)
            .clip(true)
            .style(move |_| container::Style {
                background: Some(p.surface.into()),
                border: border::rounded(8),
                ..container::Style::default()
            })
            .into()
        });

        let pager: Element<'_, Message> = if total > GLYPHS_PER_PAGE {
            row![
                label(
                    format!(
                        "{}\u{2013}{} of {}",
                        thousands(start + 1),
                        thousands(end),
                        thousands(total)
                    ),
                    p.muted
                ),
                ghost_button("[", p).on_press(Message::GlyphPage(-1)),
                ghost_button("]", p).on_press(Message::GlyphPage(1)),
            ]
            .spacing(6)
            .align_y(Center)
            .into()
        } else {
            label(thousands(total), p.muted).into()
        };

        let grid = Row::with_children(cells).spacing(6).wrap().vertical_spacing(6);

        // Jump targets, one per Unicode block the face covers.
        let content: Element<'_, Message> = if details.blocks.len() > 1 {
            let chips = details.blocks.iter().enumerate().map(|(index, block)| {
                let selected = current == Some(index);
                let icons = block.name.starts_with("Icons");

                button(
                    row![
                        text(block.name).size(11),
                        text(thousands(block.len)).size(10).font(MONO).color(
                            if selected { p.on_accent } else { p.faint }
                        ),
                    ]
                    .spacing(7)
                    .align_y(Center),
                )
                .padding([4, 10])
                .on_press(Message::GlyphBlock(index))
                .style(move |_, status| {
                    let hovered =
                        matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: Some(
                            if selected {
                                p.accent
                            } else if hovered {
                                p.raised
                            } else {
                                p.surface
                            }
                            .into(),
                        ),
                        text_color: if selected {
                            p.on_accent
                        } else if icons {
                            p.accent2
                        } else if hovered {
                            p.text
                        } else {
                            p.muted
                        },
                        border: border::rounded(6),
                        ..button::Style::default()
                    }
                })
                .into()
            });

            column![
                Row::with_children(chips).spacing(5).wrap().vertical_spacing(5),
                grid
            ]
            .spacing(18)
            .into()
        } else {
            grid.into()
        };

        page = page.push(section("Glyphs", Some(pager), p, content));
    }

    // -- Details -----------------------------------------------------------
    if let Some(details) = details {
        let entry = |name: &'static str, value: Element<'a, Message>| {
            row![
                container(label(name, p.muted)).width(130).padding(Padding::ZERO.top(3)),
                value
            ]
        };

        let mut table = Column::new().spacing(10);

        for (name, value) in &details.info {
            table = table.push(entry(
                name,
                text(value)
                    .size(13)
                    .shaping(Shaping::Advanced)
                    .color(mix(p.muted, p.text, 0.7))
                    .width(Fill)
                    .into(),
            ));
        }

        if !details.axes.is_empty() {
            let axes = details.axes.iter().map(|axis| {
                tag(
                    format!(
                        "{} {}\u{2013}{} ({})",
                        axis.tag,
                        number(axis.min),
                        number(axis.max),
                        number(axis.default)
                    ),
                    p.accent2,
                    p,
                )
            });
            table = table.push(entry(
                "Axes",
                Row::with_children(axes)
                    .spacing(6)
                    .wrap()
                    .vertical_spacing(6)
                    .into(),
            ));
        }

        if !details.features.is_empty() {
            let features = details
                .features
                .iter()
                .map(|feature| tag(feature.clone(), p.muted, p));
            table = table.push(entry(
                "Features",
                Row::with_children(features)
                    .spacing(6)
                    .wrap()
                    .vertical_spacing(6)
                    .into(),
            ));
        }

        page = page.push(section("Details", None, p, table));
    }

    page.into()
}

/// The one-line summary under the family name.
fn facts<'a>(face: &'a Face, details: Option<&'a Details>, p: Palette) -> Element<'a, Message> {
    let mut facts: Vec<String> = vec![face.style.clone()];

    if let Some(details) = details {
        facts.push(details.outline.to_owned());
        if !details.axes.is_empty() {
            facts.push("Variable".to_owned());
        }
        if face.monospaced {
            facts.push("Monospaced".to_owned());
        }
        facts.push(format!("{} glyphs", thousands(usize::from(details.glyphs))));
        facts.push(format!("{} characters", thousands(details.chars.len())));
    }

    let mut line = Row::new().spacing(10).align_y(Center);
    for (index, fact) in facts.into_iter().enumerate() {
        if index > 0 {
            line = line.push(text("\u{00B7}").size(13).color(p.faint));
        }
        line = line.push(text(fact).size(13).color(if index == 0 {
            p.text
        } else {
            p.muted
        }));
    }

    line.wrap().into()
}

/// The styles of a family, as clickable pills.
fn styles<'a>(family: &'a Family, p: Palette) -> Element<'a, Message> {
    let pills = family.faces.iter().enumerate().map(|(index, face)| {
        let selected = index == family.current;

        button(text(family.label(face)).size(12))
            .padding([5, 12])
            .on_press(Message::SelectFace(index))
            .style(move |_, status| {
                let hovered =
                    matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(
                        if selected {
                            p.accent
                        } else if hovered {
                            p.raised
                        } else {
                            p.surface
                        }
                        .into(),
                    ),
                    text_color: if selected {
                        p.on_accent
                    } else if hovered {
                        p.text
                    } else {
                        p.muted
                    },
                    border: border::rounded(99),
                    ..button::Style::default()
                }
            })
            .into()
    });

    container(Row::with_children(pills).spacing(6).wrap().vertical_spacing(6))
        .padding(Padding::ZERO.top(8))
        .into()
}

/// A titled block of the specimen.
fn section<'a>(
    title: &'static str,
    aside: Option<Element<'a, Message>>,
    p: Palette,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut header = row![
        label(title, p.accent),
        container(space())
            .width(Fill)
            .height(1)
            .style(fill(alpha(p.border, 0.7))),
    ]
    .spacing(14)
    .align_y(Center);

    if let Some(aside) = aside {
        header = header.push(aside);
    }

    column![header, content.into()].spacing(22).into()
}

/// A centred message for when there is nothing to show.
fn problem<'a>(app: &'a App, title: &'a str, message: &'a str) -> Element<'a, Message> {
    let p = app.palette;

    let mut lines = column![
        text(title).size(22).color(p.accent),
        text(message).size(14).color(p.muted).center(),
    ]
    .spacing(10)
    .max_width(520)
    .align_x(Center);

    if let (Some(file), Some(_)) = (&app.file, &app.error) {
        lines = lines.push(text(file.display().to_string()).size(11).font(MONO).color(p.faint));
    }

    center(lines).into()
}

// ---------------------------------------------------------------------------
// Status bar & help
// ---------------------------------------------------------------------------

fn status_bar(app: &App) -> Element<'_, Message> {
    let p = app.palette;

    let (mode, color) = match app.mode {
        Mode::Normal if app.inside => ("VIEW", p.accent2),
        Mode::Normal => ("NORMAL", p.accent),
        Mode::Filter => ("FILTER", p.accent2),
        Mode::Insert => ("INSERT", p.accent3),
    };

    let mode = container(text(mode).size(10).font(MONO_BOLD).color(p.on_accent))
        .height(Fill)
        .padding(Padding::ZERO.left(12).right(12))
        .align_y(Center)
        .style(fill(color));

    let subject = if let Some(flash) = &app.flash {
        flash.clone()
    } else if let Some(file) = app.file.as_ref().filter(|_| !app.browse) {
        file.display().to_string()
    } else if let Some(face) = app.face() {
        format!("{} \u{2014} {}", face.family, face.style)
    } else {
        String::new()
    };

    let mut keys = String::new();
    if let Some(count) = app.count {
        keys.push_str(&count.to_string());
    }
    if let Some(pending) = app.pending {
        keys.push(pending);
    }

    let mut bar = row![
        mode,
        text(subject)
            .size(11)
            .font(MONO)
            .color(p.muted)
            .wrapping(Wrapping::None),
        space::horizontal(),
        text(keys).size(11).font(MONO_BOLD).color(p.accent2),
    ]
    .spacing(14)
    .align_y(Center);

    if app.browse && !app.visible.is_empty() {
        bar = bar.push(label(
            format!("{}/{}", app.cursor + 1, app.visible.len()),
            p.muted,
        ));
    }

    bar = bar
        .push(label(
            format!("{} {:.0}\u{00B0}", app.palette.harmony, app.palette.hue),
            p.accent3,
        ))
        .push(
            button(label("? help", p.faint))
                .padding(0)
                .on_press(Message::ToggleHelp)
                .style(|_, _| button::Style::default()),
        );

    container(bar.padding(Padding::ZERO.right(14)))
        .width(Fill)
        .height(STATUS_HEIGHT)
        .clip(true)
        .style(fill(p.surface))
        .into()
}

fn help(app: &App) -> Element<'_, Message> {
    let p = app.palette;

    let browse: &[(&str, &str)] = &[
        ("j  k", "next / previous family"),
        ("h  l", "previous / next style"),
        ("gg  G", "first / last family"),
        ("5j  12G", "motions take counts"),
        ("Ctrl-d  Ctrl-u", "half a page down / up"),
        ("Ctrl-f  Ctrl-b", "a page down / up"),
        ("H  M  L", "top / middle / bottom of the list"),
        ("zz  zt  zb", "centre / top / bottom the cursor"),
        ("/", "filter families (Enter keeps, Esc clears)"),
        ("m", "monospaced families only"),
        ("Enter", "step into the specimen: j k gg G Ctrl-d Ctrl-u scroll it"),
        ("Enter  Esc", "step back out to the family list"),
        ("J  K", "scroll the specimen without stepping in"),
    ];
    let single: &[(&str, &str)] = &[
        ("j  k", "scroll down / up"),
        ("h  l", "previous / next face in the file"),
        ("gg  G", "top / bottom"),
        ("Ctrl-d  Ctrl-u", "half a page down / up"),
        ("Ctrl-f  Ctrl-b", "a page down / up"),
    ];
    let common: &[(&str, &str)] = &[
        ("[  ]", "previous / next page of glyphs"),
        ("{  }", "previous / next block of glyphs (icons, arrows\u{2026})"),
        ("i", "edit the sample text"),
        ("+  -  0", "zoom in / out / reset"),
        ("s  Space", "shuffle the colour scheme"),
        ("y", "yank the family name"),
        ("?", "toggle this help"),
        ("q", "quit"),
    ];

    let motions = if app.browse { browse } else { single };

    let lines = motions.iter().chain(common).map(|(keys, action)| {
        row![
            text(*keys).size(12).font(MONO_BOLD).color(p.accent2).width(150),
            text(*action).size(13).color(p.text),
        ]
        .align_y(Center)
        .into()
    });

    let card = container(
        column![
            label("Keys", p.accent),
            Column::with_children(lines).spacing(9)
        ]
        .spacing(18),
    )
    .padding(28)
    .style(move |_| container::Style {
        background: Some(p.surface.into()),
        border: Border {
            color: p.border,
            width: 1.0,
            radius: 14.0.into(),
        },
        ..container::Style::default()
    });

    opaque(
        mouse_area(center(opaque(card)).style(fill(alpha(p.bg, 0.82))))
            .on_press(Message::ToggleHelp),
    )
}

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

/// Small, spaced-out, uppercase text for labels and counters.
fn label<'a>(content: impl Into<String>, color: Color) -> iced::widget::Text<'a> {
    let spaced: String = content.into().to_uppercase();
    text(spaced).size(10).font(MONO).color(color)
}

/// An outlined chip.
fn tag<'a>(content: impl Into<String>, color: Color, p: Palette) -> Element<'a, Message> {
    container(text(content.into()).size(11).font(MONO).color(color))
        .padding([3, 8])
        .style(move |_| container::Style {
            background: Some(p.surface.into()),
            border: Border {
                color: alpha(p.border, 0.8),
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn ghost_button<'a>(content: &'a str, p: Palette) -> iced::widget::Button<'a, Message> {
    button(text(content).size(13).font(MONO).width(18).center())
        .padding([6, 6])
        .style(ghost(p))
}

fn ghost(p: Palette) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then(|| p.raised.into()),
            text_color: if hovered { p.text } else { p.muted },
            border: border::rounded(7),
            ..button::Style::default()
        }
    }
}

fn input_style(
    p: Palette,
    background: Color,
) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, status| {
        let focused = matches!(status, text_input::Status::Focused { .. });
        let hovered = matches!(status, text_input::Status::Hovered);

        text_input::Style {
            background: background.into(),
            border: Border {
                color: if focused {
                    p.accent
                } else if hovered {
                    p.faint
                } else {
                    p.border
                },
                width: 1.0,
                radius: 8.0.into(),
            },
            icon: p.muted,
            placeholder: p.faint,
            value: p.text,
            selection: alpha(p.accent, 0.35),
        }
    }
}

fn fill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        ..container::Style::default()
    }
}

fn rounded(color: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border::rounded(radius),
        ..container::Style::default()
    }
}

fn rule_vertical<'a>(color: Color) -> Element<'a, Message> {
    container(space())
        .width(1)
        .height(Fill)
        .style(fill(color))
        .into()
}

/// `12345` as `12,345`.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// An axis value without a pointless fraction.
fn number(value: f32) -> String {
    if value.fract().abs() < 0.005 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}
