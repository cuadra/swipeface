//! Font discovery and metadata.
//!
//! The renderer's own font database is the single source of truth: installed
//! fonts are enumerated from it, and every face we want to display gets an
//! *alias* entry with a unique family name. Rendering through the alias pins
//! the text to exactly that face, side-stepping family/weight/style matching
//! (which cannot tell apart faces that differ only by name, optical size or a
//! non-standard weight). For variable fonts, one alias is registered per
//! weight instance, since the text engine drives the `wght` axis from the
//! requested weight.

use iced::advanced::graphics::text::cosmic_text::fontdb;
use iced::advanced::graphics::text::font_system;
use iced::font::{self, Font};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub type Database = fontdb::Database;

/// Runs `f` with exclusive access to the renderer's font database.
pub fn with_database<T>(f: impl FnOnce(&mut Database) -> T) -> T {
    let mut guard = font_system()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    f(guard.raw().db_mut())
}

/// Where the bytes of a face live.
#[derive(Clone)]
pub enum Data {
    File(PathBuf),
    Memory { bytes: Arc<Vec<u8>>, path: PathBuf },
}

impl Data {
    pub fn path(&self) -> &Path {
        match self {
            Data::File(path) | Data::Memory { path, .. } => path,
        }
    }

    fn source(&self) -> fontdb::Source {
        match self {
            Data::File(path) => fontdb::Source::File(path.clone()),
            Data::Memory { bytes, .. } => fontdb::Source::Binary(bytes.clone()),
        }
    }

    fn size(&self) -> u64 {
        match self {
            Data::File(path) => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            Data::Memory { bytes, .. } => bytes.len() as u64,
        }
    }

    /// Runs `f` over the raw font data. Files are memory-mapped, so parsing a
    /// few tables out of a 30 MB collection only touches the pages it needs.
    fn with<T>(&self, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
        match self {
            Data::Memory { bytes, .. } => Some(f(bytes)),
            Data::File(path) => {
                let file = std::fs::File::open(path).ok()?;
                // SAFETY: the mapping is read-only and dropped before returning.
                // A font file being rewritten underneath us is not a case worth
                // defending against in a viewer.
                let map = unsafe { memmap2::Mmap::map(&file).ok()? };
                Some(f(&map))
            }
        }
    }
}

#[derive(Clone)]
pub struct Face {
    /// Family name, as shown to the user.
    pub family: String,
    /// Style name, as shown to the user ("Bold Italic").
    pub style: String,
    /// A font that renders exactly this face.
    pub font: Font,
    pub weight: u16,
    pub italic: bool,
    /// Width class, 1 (ultra condensed) to 9 (ultra expanded).
    pub stretch: u16,
    pub monospaced: bool,
    pub post_script_name: String,
    pub data: Data,
    pub index: u32,
    pub details: Option<Arc<Details>>,
}

impl Face {
    fn from_info(db: &mut Database, info: &fontdb::FaceInfo, data: Data, family: String) -> Self {
        let italic = info.style != fontdb::Style::Normal;
        let stretch = info.stretch.to_number();

        Self {
            family,
            style: style_label(info.weight.0, italic, stretch),
            font: alias(
                db,
                info.source.clone(),
                info.index,
                &info.post_script_name,
                info.monospaced,
                400,
            ),
            weight: info.weight.0,
            italic,
            stretch,
            monospaced: info.monospaced,
            post_script_name: info.post_script_name.clone(),
            data,
            index: info.index,
            details: None,
        }
    }
}

impl Face {
    /// Whether the Latin letters of this face are actually Latin letters,
    /// judging by the names of the glyphs they map to.
    fn spells_latin(&self) -> bool {
        self.data
            .with(|bytes| {
                let Ok(font) = ttf_parser::Face::parse(bytes, self.index) else {
                    return false;
                };

                ['A', 'a', 'g'].into_iter().all(|letter| {
                    let Some(glyph) = font.glyph_index(letter) else {
                        return false;
                    };

                    match font.glyph_name(glyph) {
                        // Plenty of fonts ship without glyph names.
                        None => true,
                        Some(name) => {
                            name.len() == 1 && name.starts_with(letter)
                                || ["uni", "u0", "cid", "gid", "glyph"]
                                    .iter()
                                    .any(|prefix| name.starts_with(prefix))
                        }
                    }
                })
            })
            .unwrap_or(false)
    }
}

pub struct Family {
    pub name: String,
    /// Lowercased name, for sorting and filtering.
    pub key: String,
    pub faces: Vec<Face>,
    /// Index of the face currently on display.
    pub current: usize,
    /// Index of the most "regular" face, which represents the family in lists.
    pub regular: usize,
    pub monospaced: bool,
    /// Whether the faces belong to more than one family (font collections).
    pub mixed: bool,
    /// Whether the family can spell its own name. Symbol fonts put ornaments
    /// or Greek on the Latin letters, which makes for unreadable labels.
    pub legible: bool,
    inspected: bool,
}

impl Family {
    fn new(name: String) -> Self {
        Self {
            key: name.to_lowercase(),
            name,
            faces: Vec::new(),
            current: 0,
            regular: 0,
            monospaced: false,
            mixed: false,
            legible: true,
            inspected: false,
        }
    }

    pub fn face(&self) -> &Face {
        &self.faces[self.current]
    }

    /// Label of a face within this family.
    pub fn label(&self, face: &Face) -> String {
        if self.mixed {
            format!("{} {}", face.family, face.style)
        } else {
            face.style.clone()
        }
    }

    fn finish(&mut self) {
        let mut seen = BTreeSet::new();
        self.faces.retain(|face| {
            seen.insert((
                face.family.clone(),
                face.post_script_name.clone(),
                face.weight,
                face.italic,
                face.stretch,
            ))
        });

        self.order();
        self.monospaced = self.faces.iter().any(|face| face.monospaced);
        self.mixed = self
            .faces
            .iter()
            .any(|face| face.family != self.faces[0].family);
        self.legible = self.face().spells_latin();
    }

    /// Sorts the faces and puts the most "regular" one on display.
    fn order(&mut self) {
        self.faces.sort_by(|a, b| {
            (&a.family, a.stretch, a.weight, a.italic)
                .cmp(&(&b.family, b.stretch, b.weight, b.italic))
        });

        self.current = self
            .faces
            .iter()
            .enumerate()
            .min_by_key(|(_, face)| {
                (
                    face.stretch.abs_diff(5),
                    face.italic,
                    face.weight.abs_diff(400),
                )
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        self.regular = self.current;
    }

    /// Makes the family ready for display: reads real style names, expands
    /// variable fonts into weight instances, and loads the details of the
    /// current face. Everything is cached, so this is cheap to call again.
    pub fn prepare(&mut self, db: &mut Database) {
        if !self.inspected {
            self.inspect(db);
        }

        let face = &mut self.faces[self.current];
        if face.details.is_none() {
            face.details = Some(Arc::new(Details::load(face)));
        }
    }

    fn inspect(&mut self, db: &mut Database) {
        self.inspected = true;

        let mut faces = Vec::with_capacity(self.faces.len());

        for mut face in std::mem::take(&mut self.faces) {
            let probe = face
                .data
                .with(|bytes| Probe::read(bytes, face.index))
                .flatten()
                .unwrap_or_default();

            let instances: Vec<u16> = match probe.weight_axis {
                Some((min, max)) => (1..=9u16)
                    .map(|step| step * 100)
                    .filter(|weight| min <= f32::from(*weight) && f32::from(*weight) <= max)
                    .collect(),
                None => Vec::new(),
            };

            if instances.len() > 1 {
                for weight in instances {
                    faces.push(Face {
                        style: style_label(weight, face.italic, face.stretch),
                        font: alias(
                            db,
                            face.data.source(),
                            face.index,
                            &face.post_script_name,
                            face.monospaced,
                            weight,
                        ),
                        weight,
                        details: None,
                        ..face.clone()
                    });
                }
            } else {
                if let Some(name) = probe.subfamily {
                    face.style = name;
                }
                faces.push(face);
            }
        }

        self.faces = faces;
        self.order();
    }
}

/// Registers an alias for a face under a unique family name and returns the
/// font that selects it.
fn alias(
    db: &mut Database,
    source: fontdb::Source,
    index: u32,
    post_script_name: &str,
    monospaced: bool,
    weight: u16,
) -> Font {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    // Font families are `&'static str` in iced. Aliases live for the whole
    // session and there are only as many as there are faces, so leak them.
    let name: &'static str = Box::leak(
        format!("swipeface alias {}", NEXT.fetch_add(1, Ordering::Relaxed)).into_boxed_str(),
    );

    db.push_face_info(fontdb::FaceInfo {
        id: fontdb::ID::dummy(),
        source,
        index,
        families: vec![(name.to_owned(), fontdb::Language::English_UnitedStates)],
        post_script_name: post_script_name.to_owned(),
        style: fontdb::Style::Normal,
        weight: fontdb::Weight(weight),
        stretch: fontdb::Stretch::Normal,
        monospaced,
    });

    Font {
        family: font::Family::Name(name),
        weight: match weight {
            0..=149 => font::Weight::Thin,
            150..=249 => font::Weight::ExtraLight,
            250..=349 => font::Weight::Light,
            350..=449 => font::Weight::Normal,
            450..=549 => font::Weight::Medium,
            550..=649 => font::Weight::Semibold,
            650..=749 => font::Weight::Bold,
            750..=849 => font::Weight::ExtraBold,
            _ => font::Weight::Black,
        },
        stretch: font::Stretch::Normal,
        style: font::Style::Normal,
    }
}

fn weight_name(weight: u16) -> &'static str {
    match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    }
}

fn style_label(weight: u16, italic: bool, stretch: u16) -> String {
    let mut parts: Vec<&str> = Vec::new();

    match stretch {
        1 => parts.push("UltraCondensed"),
        2 => parts.push("ExtraCondensed"),
        3 => parts.push("Condensed"),
        4 => parts.push("SemiCondensed"),
        6 => parts.push("SemiExpanded"),
        7 => parts.push("Expanded"),
        8 => parts.push("ExtraExpanded"),
        9 => parts.push("UltraExpanded"),
        _ => {}
    }

    let regular = (350..=449).contains(&weight);
    if !regular || (!italic && parts.is_empty()) {
        parts.push(weight_name(weight));
    }
    if italic {
        parts.push("Italic");
    }

    parts.join(" ")
}

/// Every installed font, grouped by family and sorted by name.
pub fn system() -> Vec<Family> {
    with_database(|db| {
        let infos: Vec<fontdb::FaceInfo> = db.faces().cloned().collect();
        let mut families: BTreeMap<String, Family> = BTreeMap::new();

        for info in &infos {
            let path = match &info.source {
                fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => path.clone(),
                // Fonts embedded in the toolkit are not "installed".
                fontdb::Source::Binary(_) => continue,
            };

            let Some(name) = family_name(info) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }

            let face = Face::from_info(db, info, Data::File(path), name.clone());

            families
                .entry(name.to_lowercase())
                .or_insert_with(|| Family::new(name))
                .faces
                .push(face);
        }

        let mut families: Vec<Family> = families.into_values().collect();
        families.iter_mut().for_each(Family::finish);
        families
    })
}

/// Loads a single font file (or collection) as one browsable group.
pub fn from_file(path: &Path) -> Result<Family, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("Could not read the file: {error}"))?;

    if bytes.starts_with(b"wOFF") || bytes.starts_with(b"wOF2") {
        return Err(
            "This is a compressed web font (WOFF). Only TrueType and OpenType \
             fonts and collections are supported."
                .to_owned(),
        );
    }

    ttf_parser::Face::parse(&bytes, 0)
        .map_err(|error| format!("This does not look like a font file ({error})."))?;

    let bytes = Arc::new(bytes);
    let data = Data::Memory {
        bytes: bytes.clone(),
        path: path.to_path_buf(),
    };

    with_database(|db| {
        let ids = db.load_font_source(fontdb::Source::Binary(bytes.clone()));
        let infos: Vec<fontdb::FaceInfo> =
            ids.iter().filter_map(|id| db.face(*id).cloned()).collect();

        let fallback = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_owned());

        let mut family = Family::new(fallback.clone());

        for info in &infos {
            let name = family_name(info).unwrap_or_else(|| fallback.clone());
            family
                .faces
                .push(Face::from_info(db, info, data.clone(), name));
        }

        if family.faces.is_empty() {
            return Err("No usable faces were found in this file.".to_owned());
        }

        family.finish();
        if !family.mixed {
            family.name = family.faces[0].family.clone();
            family.key = family.name.to_lowercase();
        }

        Ok(family)
    })
}

fn family_name(info: &fontdb::FaceInfo) -> Option<String> {
    info.families
        .iter()
        .find(|(_, language)| *language == fontdb::Language::English_UnitedStates)
        .or_else(|| info.families.first())
        .map(|(name, _)| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// The little we need to know about every face of a family up front.
#[derive(Default)]
struct Probe {
    subfamily: Option<String>,
    /// Range of the `wght` axis, for variable fonts.
    weight_axis: Option<(f32, f32)>,
}

impl Probe {
    fn read(bytes: &[u8], index: u32) -> Option<Self> {
        let face = ttf_parser::Face::parse(bytes, index).ok()?;

        Some(Self {
            subfamily: name(&face, ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY)
                .or_else(|| name(&face, ttf_parser::name_id::SUBFAMILY)),
            weight_axis: face
                .variation_axes()
                .into_iter()
                .find(|axis| axis.tag == ttf_parser::Tag::from_bytes(b"wght"))
                .map(|axis| (axis.min_value, axis.max_value)),
        })
    }
}

pub struct Axis {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// A run of consecutive entries of [`Details::chars`] in one Unicode block.
pub struct Block {
    pub name: &'static str,
    /// Index of the first character of the run.
    pub start: usize,
    pub len: usize,
}

const PRIVATE_USE: &str = "Icons \u{00B7} Private Use";

/// A coarse map of Unicode: first code point, last code point, name.
/// Sorted, without overlaps; anything in between is "Other".
const BLOCKS: &[(u32, u32, &str)] = &[
    (0x0000, 0x007F, "Basic Latin"),
    (0x0080, 0x00FF, "Latin-1"),
    (0x0100, 0x024F, "Latin Extended"),
    (0x0250, 0x02FF, "IPA & Modifiers"),
    (0x0300, 0x036F, "Combining Marks"),
    (0x0370, 0x03FF, "Greek"),
    (0x0400, 0x052F, "Cyrillic"),
    (0x0530, 0x058F, "Armenian"),
    (0x0590, 0x05FF, "Hebrew"),
    (0x0600, 0x077F, "Arabic"),
    (0x0900, 0x0DFF, "Indic"),
    (0x0E00, 0x0E7F, "Thai"),
    (0x0E80, 0x0FFF, "Lao & Tibetan"),
    (0x10A0, 0x10FF, "Georgian"),
    (0x1100, 0x11FF, "Hangul Jamo"),
    (0x1E00, 0x1EFF, "Latin Additional"),
    (0x1F00, 0x1FFF, "Greek Extended"),
    (0x2000, 0x206F, "Punctuation"),
    (0x2070, 0x209F, "Super & Subscripts"),
    (0x20A0, 0x20CF, "Currency"),
    (0x20D0, 0x214F, "Letterlike"),
    (0x2150, 0x218F, "Number Forms"),
    (0x2190, 0x21FF, "Arrows"),
    (0x2200, 0x22FF, "Math Operators"),
    (0x2300, 0x23FF, "Technical"),
    (0x2400, 0x245F, "Control Pictures"),
    (0x2460, 0x24FF, "Enclosed"),
    (0x2500, 0x257F, "Box Drawing"),
    (0x2580, 0x259F, "Block Elements"),
    (0x25A0, 0x25FF, "Geometric Shapes"),
    (0x2600, 0x26FF, "Symbols"),
    (0x2700, 0x27BF, "Dingbats"),
    (0x27C0, 0x27FF, "Math & Arrows"),
    (0x2800, 0x28FF, "Braille"),
    (0x2900, 0x2BFF, "Math & Arrows"),
    (0x2E80, 0x2FFF, "CJK Radicals"),
    (0x3000, 0x303F, "CJK Punctuation"),
    (0x3040, 0x30FF, "Kana"),
    (0x3100, 0x33FF, "CJK Letters & Enclosed"),
    (0x3400, 0x4DBF, "CJK Extension A"),
    (0x4E00, 0x9FFF, "CJK Ideographs"),
    (0xAC00, 0xD7AF, "Hangul"),
    (0xE000, 0xF8FF, PRIVATE_USE),
    (0xF900, 0xFAFF, "CJK Compatibility"),
    (0xFB00, 0xFB4F, "Ligatures & Forms"),
    (0xFB50, 0xFDFF, "Arabic Forms"),
    (0xFE70, 0xFEFF, "Arabic Forms"),
    (0xFF00, 0xFFEF, "Half & Full Width"),
    (0x1D400, 0x1D7FF, "Math Alphanumerics"),
    (0x1F000, 0x1F2FF, "Tiles & Cards"),
    (0x1F300, 0x1FAFF, "Emoji & Pictographs"),
    (0x1FB00, 0x1FBFF, "Legacy Computing"),
    (0x20000, 0x3FFFF, "CJK Extensions"),
    (0xF0000, 0x10FFFF, PRIVATE_USE),
];

fn block_name(c: char) -> &'static str {
    let code = c as u32;
    let index = BLOCKS.partition_point(|(first, _, _)| *first <= code);

    match index.checked_sub(1).map(|index| BLOCKS[index]) {
        Some((_, last, name)) if code <= last => name,
        _ => "Other",
    }
}

/// Groups sorted characters into runs of the same block.
fn blocks(chars: &[char]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();

    for (index, &c) in chars.iter().enumerate() {
        let name = block_name(c);

        match blocks.last_mut() {
            Some(block) if block.name == name => block.len += 1,
            _ => blocks.push(Block {
                name,
                start: index,
                len: 1,
            }),
        }
    }

    blocks
}

/// Everything the specimen shows about a face beyond its name.
pub struct Details {
    /// Label/value pairs for the details table.
    pub info: Vec<(&'static str, String)>,
    pub glyphs: u16,
    pub outline: &'static str,
    pub axes: Vec<Axis>,
    /// OpenType layout features, sorted.
    pub features: Vec<String>,
    /// Every printable character the face maps, sorted.
    pub chars: Vec<char>,
    /// Runs of `chars` that share a Unicode block, in order.
    pub blocks: Vec<Block>,
    /// An even sample of the private-use characters, which is where icon
    /// fonts (Nerd Fonts, Font Awesome) keep their icons.
    pub icons: Vec<char>,
    pub icon_count: usize,
    latin: bool,
    script_sample: Option<&'static str>,
}

/// How many icons the specimen previews.
const ICON_SAMPLE: usize = 72;

const PANGRAM: &str = "Sphinx of black quartz, judge my vow";

const PARAGRAPH: &str = "Typography is the craft of endowing human language with a durable \
    visual form, and thus with an independent existence. Its heartwood is calligraphy \u{2014} the \
    dance, on a tiny stage, of the living, speaking hand \u{2014} and its roots reach into living \
    soil, though its branches may be hung each year with new machines. So long as the root lives, \
    typography remains a source of true delight, true knowledge, true surprise.";

/// A probe character and a sample sentence for scripts other than Latin.
const SCRIPT_SAMPLES: &[(char, &str)] = &[
    ('ж', "Съешь же ещё этих мягких французских булок, да выпей чаю"),
    ('λ', "Ξεσκεπάζω την ψυχοφθόρα βδελυγμία"),
    ('א', "דג סקרן שט בים מאוכזב ולפתע מצא חברה"),
    ('ب', "نص حكيم له سر قاطع وذو شأن عظيم مكتوب على ثوب أخضر"),
    ('क', "ऋषियों को सताने वाले दुष्ट राक्षसों के राजा रावण का सर्वनाश"),
    ('ก', "เป็นมนุษย์สุดประเสริฐเลิศคุณค่า กว่าบรรดาฝูงสัตว์เดรัจฉาน"),
    ('あ', "いろはにほへと ちりぬるを わかよたれそ つねならむ"),
    ('가', "키스의 고유조건은 입술끼리 만나야 하고 특별한 기술은 필요치 않다"),
    ('永', "永和九年，歲在癸丑，暮春之初，會于會稽山陰之蘭亭"),
];

impl Details {
    fn load(face: &Face) -> Self {
        face.data
            .with(|bytes| Self::parse(bytes, face))
            .flatten()
            .unwrap_or_else(|| Self {
                info: vec![("File", face.data.path().display().to_string())],
                glyphs: 0,
                outline: "Unknown",
                axes: Vec::new(),
                features: Vec::new(),
                chars: Vec::new(),
                blocks: Vec::new(),
                icons: Vec::new(),
                icon_count: 0,
                latin: true,
                script_sample: None,
            })
    }

    fn parse(bytes: &[u8], face: &Face) -> Option<Self> {
        use ttf_parser::name_id;

        let font = ttf_parser::Face::parse(bytes, face.index).ok()?;
        let tables = font.tables();

        let mut chars = BTreeSet::new();
        if let Some(cmap) = tables.cmap {
            for subtable in cmap.subtables {
                if !subtable.is_unicode() {
                    continue;
                }
                subtable.codepoints(|codepoint| {
                    // Some fonts map a single glyph to all of Unicode.
                    if chars.len() >= 200_000 {
                        return;
                    }
                    if let Some(c) = char::from_u32(codepoint)
                        && !c.is_control()
                        && !c.is_whitespace()
                        && subtable.glyph_index(codepoint).is_some_and(|glyph| glyph.0 != 0)
                    {
                        chars.insert(c);
                    }
                });
            }
        }
        let chars: Vec<char> = chars.into_iter().collect();
        let has = |c: char| chars.binary_search(&c).is_ok();

        let latin = ('a'..='z').chain('A'..='Z').all(has);
        let script_sample = SCRIPT_SAMPLES
            .iter()
            .find(|(probe, _)| has(*probe))
            .map(|(_, sample)| *sample);

        let mut features = BTreeSet::new();
        for table in [tables.gsub, tables.gpos].into_iter().flatten() {
            for feature in table.features {
                features.insert(feature.tag.to_string());
            }
        }

        let outline = match (tables.glyf.is_some(), tables.cff.is_some(), tables.cff2.is_some()) {
            (true, _, _) => "TrueType",
            (_, true, _) => "OpenType CFF",
            (_, _, true) => "OpenType CFF2",
            _ if tables.sbix.is_some() || tables.cbdt.is_some() => "Bitmap",
            _ if tables.svg.is_some() => "SVG",
            _ => "OpenType",
        };

        let mut info = Vec::new();
        let mut push = |label: &'static str, value: Option<String>| {
            if let Some(value) = value {
                info.push((label, value));
            }
        };

        push("Full name", name(&font, name_id::FULL_NAME));
        push("PostScript", name(&font, name_id::POST_SCRIPT_NAME));
        push("Version", name(&font, name_id::VERSION));
        push("Designer", name(&font, name_id::DESIGNER));
        push("Foundry", name(&font, name_id::MANUFACTURER));
        push(
            "License",
            name(&font, name_id::LICENSE).map(|text| truncate(&text, 320)),
        );
        push(
            "Copyright",
            name(&font, name_id::COPYRIGHT_NOTICE).map(|text| truncate(&text, 320)),
        );
        push("Units per em", Some(font.units_per_em().to_string()));
        push("File", Some(face.data.path().display().to_string()));
        push("Size", Some(file_size(face.data.size())));

        let private: Vec<char> = chars
            .iter()
            .copied()
            .filter(|c| block_name(*c) == PRIVATE_USE)
            .collect();
        let icons: Vec<char> = (0..ICON_SAMPLE.min(private.len()))
            .map(|step| private[step * private.len() / ICON_SAMPLE.min(private.len())])
            .collect();

        Some(Self {
            info,
            glyphs: font.number_of_glyphs(),
            outline,
            axes: font
                .variation_axes()
                .into_iter()
                .map(|axis| Axis {
                    tag: axis.tag.to_string(),
                    min: axis.min_value,
                    default: axis.def_value,
                    max: axis.max_value,
                })
                .collect(),
            features: features.into_iter().collect(),
            blocks: blocks(&chars),
            icons,
            icon_count: private.len(),
            chars,
            latin,
            script_sample,
        })
    }

    /// A handful of mapped characters, for fonts with no script we know of.
    fn first_chars(&self, count: usize) -> String {
        self.chars.iter().take(count).collect()
    }

    /// One line of text that shows the face off.
    pub fn pangram(&self) -> String {
        if self.latin {
            PANGRAM.to_owned()
        } else if let Some(sample) = self.script_sample {
            sample.to_owned()
        } else {
            self.first_chars(28)
        }
    }

    /// Two characters for the oversized hero.
    pub fn hero(&self) -> String {
        if self.latin {
            "Aa".to_owned()
        } else {
            self.pangram()
                .chars()
                .filter(|c| !c.is_whitespace())
                .take(2)
                .collect()
        }
    }

    /// The character set overview next to the hero.
    pub fn alphabet(&self) -> Vec<String> {
        if self.latin {
            let mut lines = vec![
                "ABCDEFGHIJKLM".to_owned(),
                "NOPQRSTUVWXYZ".to_owned(),
                "abcdefghijklm".to_owned(),
                "nopqrstuvwxyz".to_owned(),
            ];
            let extras: String = "0123456789&@#$%!?"
                .chars()
                .filter(|c| self.chars.binary_search(c).is_ok())
                .collect();
            if !extras.is_empty() {
                lines.push(extras);
            }
            lines
        } else {
            self.chars
                .chunks(13)
                .take(5)
                .map(|chunk| chunk.iter().collect())
                .collect()
        }
    }

    /// Running text.
    pub fn paragraph(&self) -> String {
        if self.latin {
            PARAGRAPH.to_owned()
        } else if let Some(sample) = self.script_sample {
            [sample; 4].join(" ")
        } else {
            self.first_chars(240)
        }
    }
}

/// The best rendition of a `name` table entry: English if there is one.
fn name(face: &ttf_parser::Face<'_>, id: u16) -> Option<String> {
    let names = face.names();
    let mut best: Option<(u8, String)> = None;

    for entry in (0..names.len()).filter_map(|index| names.get(index)) {
        if entry.name_id != id {
            continue;
        }

        let (rank, value) = if entry.is_unicode() {
            let Some(value) = entry.to_string() else {
                continue;
            };
            let english = entry.language() == ttf_parser::Language::English_UnitedStates
                || entry.platform_id == ttf_parser::PlatformId::Unicode;
            (if english { 0 } else { 1 }, value)
        } else if entry.platform_id == ttf_parser::PlatformId::Macintosh && entry.encoding_id == 0
        {
            // Mac Roman; close enough to Latin-1 for names.
            (2, entry.name.iter().map(|&byte| byte as char).collect())
        } else {
            continue;
        };

        if best.as_ref().is_none_or(|(best_rank, _)| rank < *best_rank) {
            best = Some((rank, value));
        }
    }

    best.map(|(_, value)| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|value| !value.is_empty())
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let mut cut: String = text.chars().take(limit).collect();
        cut.push('\u{2026}');
        cut
    }
}

fn file_size(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / f64::from(1 << 20))
    } else {
        format!("{:.0} kB", bytes as f64 / 1024.0)
    }
}
