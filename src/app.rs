//! Application state and behaviour.

use crate::catalog::{self, Details, Face, Family};
use crate::palette::{Palette, Rng};

use iced::advanced::widget::operate;
use iced::advanced::widget::operation::focusable::unfocus;
use iced::keyboard::key::Named;
use iced::keyboard::{self, Key, Modifiers};
use iced::mouse::ScrollDelta;
use iced::widget::operation;
use iced::widget::scrollable::{AbsoluteOffset, RelativeOffset};
use iced::{Event, Size, Subscription, Task, Theme, event, window};

use std::path::PathBuf;

/// Height of one row in the family list.
pub const ROW_HEIGHT: f32 = 34.0;
/// Height of everything above the family list in the sidebar.
pub const SIDEBAR_HEAD: f32 = 100.0;
pub const GLYPHS_PER_PAGE: usize = 160;

pub const FILTER_INPUT: &str = "filter";
pub const SAMPLE_INPUT: &str = "sample";
pub const SPECIMEN: &str = "specimen";

/// Rows kept visible above and below the cursor while moving through the list.
const SCROLL_OFF: usize = 3;
const ZOOM_STEPS: &[f32] = &[0.5, 0.67, 0.8, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing in the family filter.
    Filter,
    /// Typing the sample text.
    Insert,
}

#[derive(Debug, Clone)]
pub enum Message {
    Key {
        key: Key,
        modified: Key,
        modifiers: Modifiers,
        captured: bool,
    },
    Shuffle,
    /// A family was clicked; the index is a position in the filtered list.
    Select(usize),
    SelectFace(usize),
    FilterChanged(String),
    FilterSubmitted,
    SampleChanged(String),
    SampleSubmitted,
    ListScrolled(ScrollDelta),
    BodyResized(Size),
    Zoom(i8),
    GlyphPage(isize),
    /// Jump to a Unicode block of the glyph grid, by index.
    GlyphBlock(usize),
    ToggleHelp,
}

pub struct App {
    /// Browsing installed fonts, as opposed to viewing a single file.
    pub browse: bool,
    pub families: Vec<Family>,
    /// Indices into `families` that pass the current filter.
    pub visible: Vec<usize>,
    /// Position of the selected family within `visible`.
    pub cursor: usize,
    /// Position of the first family row on screen.
    pub top: usize,
    pub filter: String,
    pub monospace_only: bool,
    pub mode: Mode,
    pub palette: Palette,
    pub theme: Theme,
    pub sample: String,
    pub scale: f32,
    /// Index of the first character shown in the glyph grid.
    pub glyph_start: usize,
    pub help: bool,
    /// While browsing: whether the keyboard is "inside" the specimen, so that
    /// motions scroll it instead of moving through the family list.
    pub inside: bool,
    /// A transient message for the status bar.
    pub flash: Option<String>,
    pub error: Option<String>,
    pub file: Option<PathBuf>,
    /// A count typed before a motion (`12j`).
    pub count: Option<usize>,
    /// The first key of a two-key motion (`g`, `z`).
    pub pending: Option<char>,
    /// Size of the area between the window top and the status bar.
    body: Size,
    /// The face whose glyph page is on display, to reset paging on change.
    shown: Option<(usize, usize)>,
    wheel: f32,
    rng: Rng,
}

impl App {
    pub fn boot(file: Option<PathBuf>) -> Self {
        let mut rng = Rng::from_clock();
        let palette = Palette::random(&mut rng, None);

        let (families, error) = match &file {
            None => (catalog::system(), None),
            Some(path) => match catalog::from_file(path) {
                Ok(family) => (vec![family], None),
                Err(error) => (Vec::new(), Some(error)),
            },
        };

        let mut app = Self {
            browse: file.is_none(),
            families,
            visible: Vec::new(),
            cursor: 0,
            top: 0,
            filter: String::new(),
            monospace_only: false,
            mode: Mode::Normal,
            theme: theme(&palette),
            palette,
            sample: String::new(),
            scale: 1.0,
            glyph_start: 0,
            help: false,
            inside: false,
            flash: None,
            error,
            file,
            count: None,
            pending: None,
            body: Size::new(1280.0, 800.0),
            shown: None,
            wheel: 0.0,
            rng,
        };

        app.refilter();
        app
    }

    pub fn title(&self) -> String {
        match self.family() {
            Some(family) if !self.browse || !self.visible.is_empty() => {
                format!("{} \u{2014} Swipeface", family.face().family)
            }
            _ => "Swipeface".to_owned(),
        }
    }

    pub fn theme(&self) -> Theme {
        self.theme.clone()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        event::listen_with(on_event)
    }

    /// The selected family.
    pub fn family(&self) -> Option<&Family> {
        self.visible
            .get(self.cursor)
            .map(|&index| &self.families[index])
    }

    /// The face on display.
    pub fn face(&self) -> Option<&Face> {
        self.family().map(Family::face)
    }

    /// How many family rows fit on screen.
    pub fn rows(&self) -> usize {
        (((self.body.height - SIDEBAR_HEAD) / ROW_HEIGHT).floor() as usize).max(1)
    }

    pub fn list_height(&self) -> f32 {
        (self.body.height - SIDEBAR_HEAD).max(ROW_HEIGHT)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Key {
                key,
                modified,
                modifiers,
                captured,
            } => return self.key(key, modified, modifiers, captured),
            Message::Shuffle => self.shuffle(),
            Message::Select(position) => {
                if position < self.visible.len() {
                    self.inside = false;
                    self.cursor = position;
                    self.sync(0);
                }
            }
            Message::SelectFace(index) => {
                if let Some(&family) = self.visible.get(self.cursor)
                    && index < self.families[family].faces.len()
                {
                    self.families[family].current = index;
                    self.sync(0);
                }
            }
            Message::FilterChanged(filter) => {
                self.filter = filter;
                self.mode = Mode::Filter;
                self.refilter();
            }
            Message::FilterSubmitted | Message::SampleSubmitted => {
                self.mode = Mode::Normal;
                return operate(unfocus());
            }
            Message::SampleChanged(sample) => {
                self.sample = sample;
                self.mode = Mode::Insert;
            }
            Message::ListScrolled(delta) => {
                self.wheel += match delta {
                    ScrollDelta::Lines { y, .. } => -y * 3.0,
                    ScrollDelta::Pixels { y, .. } => -y / ROW_HEIGHT,
                };
                let rows = self.wheel.trunc();
                self.wheel -= rows;
                self.scroll_list(rows as isize);
            }
            Message::BodyResized(size) => {
                self.body = size;
                self.sync(0);
            }
            Message::Zoom(direction) => self.zoom(direction),
            Message::GlyphPage(delta) => self.turn_glyph_page(delta),
            Message::GlyphBlock(index) => {
                if let Some(start) = self
                    .details()
                    .and_then(|details| details.blocks.get(index))
                    .map(|block| block.start)
                {
                    self.glyph_start = start;
                }
            }
            Message::ToggleHelp => self.help = !self.help,
        }

        Task::none()
    }

    fn shuffle(&mut self) {
        self.palette = Palette::random(&mut self.rng, Some(self.palette.hue));
        self.theme = theme(&self.palette);
    }

    /// Recomputes the filtered list, keeping the selection when it survives.
    fn refilter(&mut self) {
        let selected = self.visible.get(self.cursor).copied();
        let needle = self.filter.to_lowercase();
        let terms: Vec<&str> = needle.split_whitespace().collect();

        self.visible = (0..self.families.len())
            .filter(|&index| {
                let family = &self.families[index];
                (!self.monospace_only || family.monospaced)
                    && terms.iter().all(|term| family.key.contains(term))
            })
            .collect();

        self.cursor = selected
            .and_then(|selected| self.visible.iter().position(|&index| index == selected))
            .unwrap_or(0);

        self.sync(SCROLL_OFF);
    }

    /// Brings the view in line with the cursor: scrolls the list so that the
    /// cursor is on screen and loads whatever the specimen needs.
    fn sync(&mut self, scroll_off: usize) {
        let rows = self.rows();
        let total = self.visible.len();

        self.cursor = self.cursor.min(total.saturating_sub(1));

        let margin = scroll_off.min(rows.saturating_sub(1) / 2);
        if self.cursor < self.top + margin {
            self.top = self.cursor.saturating_sub(margin);
        } else if self.cursor + margin >= self.top + rows {
            self.top = self.cursor + margin + 1 - rows;
        }
        self.top = self.top.min(total.saturating_sub(rows));

        let Some(&index) = self.visible.get(self.cursor) else {
            self.shown = None;
            return;
        };

        let family = &mut self.families[index];
        catalog::with_database(|db| family.prepare(db));

        let shown = Some((index, family.current));
        if self.shown != shown {
            self.shown = shown;
            self.glyph_start = 0;
        }
    }

    fn scroll_list(&mut self, rows: isize) {
        let limit = self.visible.len().saturating_sub(self.rows());
        self.top = self.top.saturating_add_signed(rows).min(limit);
    }

    fn move_cursor(&mut self, delta: isize) {
        let last = self.visible.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
        self.sync(SCROLL_OFF);
    }

    fn jump(&mut self, position: usize) {
        self.cursor = position;
        self.sync(SCROLL_OFF);
    }

    fn cycle_face(&mut self, delta: isize) {
        let Some(&index) = self.visible.get(self.cursor) else {
            return;
        };

        let family = &mut self.families[index];
        let count = family.faces.len() as isize;
        family.current = (family.current as isize + delta).rem_euclid(count) as usize;

        self.sync(SCROLL_OFF);
    }

    fn zoom(&mut self, direction: i8) {
        let nearest = ZOOM_STEPS
            .iter()
            .position(|step| *step >= self.scale - 0.001)
            .unwrap_or(ZOOM_STEPS.len() - 1);

        self.scale = match direction {
            0 => 1.0,
            d if d > 0 => ZOOM_STEPS[(nearest + 1).min(ZOOM_STEPS.len() - 1)],
            _ => ZOOM_STEPS[nearest.saturating_sub(1)],
        };
    }

    /// Details of the face on display.
    pub fn details(&self) -> Option<&Details> {
        self.face().and_then(|face| face.details.as_deref())
    }

    /// `[` and `]`: page through the glyph grid.
    fn turn_glyph_page(&mut self, delta: isize) {
        let total = self.details().map_or(0, |details| details.chars.len());
        let step = delta.saturating_mul(GLYPHS_PER_PAGE as isize);

        match self.glyph_start.checked_add_signed(step) {
            Some(start) if start < total => self.glyph_start = start,
            Some(_) => {}
            None => self.glyph_start = 0,
        }
    }

    /// `{` and `}`: jump between the Unicode blocks of the glyph grid.
    fn jump_glyph_block(&mut self, mut delta: isize) {
        let Some(details) = self.details() else {
            return;
        };
        let Some(current) = details
            .blocks
            .iter()
            .rposition(|block| block.start <= self.glyph_start)
        else {
            return;
        };

        // Like `{` in vim: from the middle of a block, first go to its start.
        if delta < 0 && self.glyph_start > details.blocks[current].start {
            delta += 1;
        }

        let target = current
            .saturating_add_signed(delta)
            .min(details.blocks.len() - 1);
        let block = &details.blocks[target];
        let flash = format!("{} \u{2014} {} glyphs", block.name, block.len);

        self.glyph_start = block.start;
        self.flash = Some(flash);
    }

    fn scroll_specimen(&self, pixels: f32) -> Task<Message> {
        operation::scroll_by(SPECIMEN, AbsoluteOffset { x: 0.0, y: pixels })
    }

    fn escape(&mut self) -> Task<Message> {
        self.count = None;
        self.pending = None;
        self.flash = None;

        if self.help {
            self.help = false;
            return Task::none();
        }

        match self.mode {
            Mode::Insert => {}
            Mode::Filter => {
                self.filter.clear();
                self.refilter();
            }
            // Back out of the specimen first; the filter survives.
            Mode::Normal if self.inside => self.inside = false,
            Mode::Normal => {
                if !self.filter.is_empty() || self.monospace_only {
                    self.filter.clear();
                    self.monospace_only = false;
                    self.refilter();
                }
            }
        }

        self.mode = Mode::Normal;
        operate(unfocus())
    }

    fn key(&mut self, key: Key, modified: Key, modifiers: Modifiers, captured: bool) -> Task<Message> {
        if key == Key::Named(Named::Escape) {
            return self.escape();
        }

        // A focused text field has already consumed this key.
        if captured {
            return Task::none();
        }

        if self.help {
            self.help = false;
            return Task::none();
        }

        self.flash = None;

        let typing = self.mode != Mode::Normal;
        let page = self.rows() as isize;
        let view = self.body.height;

        if let Key::Named(named) = &key {
            self.pending = None;
            let count = self.count.take().unwrap_or(1) as isize;

            return match named {
                Named::ArrowDown => self.vertical(count),
                Named::ArrowUp => self.vertical(-count),
                Named::PageDown => self.page(page, view * 0.9),
                Named::PageUp => self.page(-page, -view * 0.9),
                _ if typing => Task::none(),
                Named::ArrowLeft => self.horizontal(-count),
                Named::ArrowRight => self.horizontal(count),
                Named::Home => self.first(),
                Named::End => self.last(),
                Named::Enter if self.browse && !self.visible.is_empty() => {
                    self.inside = !self.inside;
                    Task::none()
                }
                Named::Backspace => {
                    self.inside = false;
                    Task::none()
                }
                Named::Space => {
                    self.shuffle();
                    Task::none()
                }
                _ => Task::none(),
            };
        }

        let Key::Character(text) = &modified else {
            return Task::none();
        };
        let Some(c) = text.chars().next() else {
            return Task::none();
        };

        if modifiers.control() {
            self.pending = None;
            let count = self.count.take().unwrap_or(1) as isize;

            return match c.to_ascii_lowercase() {
                'n' => self.vertical(count),
                'p' => self.vertical(-count),
                'd' => self.page(page / 2, view / 2.0),
                'u' => self.page(-page / 2, -view / 2.0),
                'f' => self.page(page, view * 0.9),
                'b' => self.page(-page, -view * 0.9),
                'e' => self.scroll_specimen(60.0 * count as f32),
                'y' => self.scroll_specimen(-60.0 * count as f32),
                _ => Task::none(),
            };
        }

        if typing || modifiers.alt() || modifiers.logo() {
            return Task::none();
        }

        if let Some(digit) = c.to_digit(10)
            && (digit != 0 || self.count.is_some())
            && self.pending.is_none()
        {
            let count = self.count.unwrap_or(0) * 10 + digit as usize;
            self.count = Some(count.min(99_999));
            return Task::none();
        }

        let given = self.count.take();
        let count = given.unwrap_or(1) as isize;

        if let Some(pending) = self.pending.take() {
            return match (pending, c) {
                ('g', 'g') => match given {
                    Some(line) => self.goto(line),
                    None => self.first(),
                },
                ('z', 'z') => self.place_cursor(self.rows() / 2),
                ('z', 't') => self.place_cursor(0),
                ('z', 'b') => self.place_cursor(self.rows().saturating_sub(1)),
                _ => Task::none(),
            };
        }

        match c {
            'j' => return self.vertical(count),
            'k' => return self.vertical(-count),
            'h' => return self.horizontal(-count),
            'l' => return self.horizontal(count),
            'J' => return self.scroll_specimen(120.0 * count as f32),
            'K' => return self.scroll_specimen(-120.0 * count as f32),
            'G' => {
                return match given {
                    Some(line) => self.goto(line),
                    None => self.last(),
                };
            }
            'g' | 'z' => {
                self.pending = Some(c);
                self.count = given;
            }
            'H' => self.jump(self.top),
            'M' => self.jump(self.top + self.rows().min(self.visible.len()) / 2),
            'L' => self.jump(self.top + self.rows() - 1),
            '/' if self.browse => {
                self.inside = false;
                self.mode = Mode::Filter;
                return operation::focus(FILTER_INPUT);
            }
            'i' => {
                self.mode = Mode::Insert;
                return operation::focus(SAMPLE_INPUT);
            }
            'm' if self.browse => {
                self.monospace_only = !self.monospace_only;
                self.refilter();
            }
            's' => self.shuffle(),
            '+' | '=' => self.zoom(1),
            '-' | '_' => self.zoom(-1),
            '0' => self.zoom(0),
            ']' => self.turn_glyph_page(count),
            '[' => self.turn_glyph_page(-count),
            '}' => self.jump_glyph_block(count),
            '{' => self.jump_glyph_block(-count),
            '?' => self.help = true,
            'y' => {
                if let Some(face) = self.face() {
                    let name = face.family.clone();
                    self.flash = Some(format!("yanked \u{201C}{name}\u{201D}"));
                    return iced::clipboard::write(name);
                }
            }
            'q' => return iced::exit(),
            _ => {}
        }

        Task::none()
    }

    /// Whether motions scroll the specimen rather than move through the list:
    /// always when viewing a file, after `Enter` when browsing.
    fn scrolling(&self) -> bool {
        !self.browse || self.inside
    }

    /// `j`/`k`: the next family in the list, or scrolling inside the specimen.
    fn vertical(&mut self, count: isize) -> Task<Message> {
        if !self.scrolling() {
            self.move_cursor(count);
            Task::none()
        } else {
            self.scroll_specimen(60.0 * count as f32)
        }
    }

    /// `h`/`l`: the next style of the family.
    fn horizontal(&mut self, count: isize) -> Task<Message> {
        self.cycle_face(count);
        Task::none()
    }

    fn page(&mut self, rows: isize, pixels: f32) -> Task<Message> {
        if !self.scrolling() {
            self.scroll_list(rows);
            self.move_cursor(rows);
            Task::none()
        } else {
            self.scroll_specimen(pixels)
        }
    }

    fn first(&mut self) -> Task<Message> {
        if !self.scrolling() {
            self.jump(0);
            Task::none()
        } else {
            operation::snap_to(SPECIMEN, RelativeOffset::START)
        }
    }

    fn last(&mut self) -> Task<Message> {
        if !self.scrolling() {
            self.jump(usize::MAX);
            Task::none()
        } else {
            operation::snap_to_end(SPECIMEN)
        }
    }

    /// `{n}G`: jump to the n-th family.
    fn goto(&mut self, line: usize) -> Task<Message> {
        if !self.scrolling() {
            self.jump(line.saturating_sub(1));
        }
        Task::none()
    }

    /// `zz`, `zt`, `zb`: scroll so that the cursor sits on the given row.
    fn place_cursor(&mut self, row: usize) -> Task<Message> {
        let limit = self.visible.len().saturating_sub(self.rows());
        self.top = self.cursor.saturating_sub(row).min(limit);
        Task::none()
    }
}

fn theme(palette: &Palette) -> Theme {
    Theme::custom(
        "Swipeface",
        iced::theme::Palette {
            background: palette.bg,
            text: palette.text,
            primary: palette.accent,
            success: palette.accent2,
            warning: palette.accent3,
            danger: palette.accent3,
        },
    )
}

fn on_event(event: Event, status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            modifiers,
            ..
        }) => Some(Message::Key {
            key,
            modified: modified_key,
            modifiers,
            captured: status == event::Status::Captured,
        }),
        _ => None,
    }
}
