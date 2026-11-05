use std::collections::HashSet;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Embedded FST dictionary compiled by `build.rs` when `UNIPASTREN_BUILD_DICTS` is set.
static EMBEDDED_DICT_FST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/embedded_dict.fst"));

/// Conservative fallback pool size when a single word is not verified by a dictionary (top-1000 list -> ~10 bits).
pub const DEFAULT_UNVERIFIED_POOL_SIZE: usize = 1000;

/// Default maximum number of Unicode scalar values (chars) to analyze to prevent excessive CPU usage.
pub const DEFAULT_MAX_CHARS: usize = 100;

/// Approximate number of Unicode scripts in standard use (~150 -> ~7.23 bits of script choice).
pub const UNICODE_SCRIPT_COUNT: f64 = 150.0;

/// Entropy in bits assigned to a repeated single-character non-word literal/delimiter (e.g., repeated `/` or `-`).
pub const REPEATED_LITERAL_ENTROPY_BITS: f64 = 1.0;

/// Bounded maximum period length for tandem repeat scans.
const MAX_REPEAT_PERIOD: usize = 32;

/// Maximum length of a constant separator inserted between characters of a word or sequence (e.g. "b22a22t22...").
const MAX_INTERLEAVE_SEP_LEN: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnicodePool {
    AsciiLower,
    AsciiUpper,
    AsciiDigit,
    AsciiSymbol,
    LatinExtended,
    Cyrillic,
    Greek,
    Armenian,
    Hebrew,
    Arabic,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
    Thai,
    Lao,
    Tibetan,
    Myanmar,
    Georgian,
    Ethiopic,
    Khmer,
    Hangul,
    HiraganaKatakana,
    CjkIdeograph,
    EmojiAndSymbols,
    OtherScript(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptFamily {
    Latin,
    Cyrillic,
    Greek,
    Armenian,
    Hebrew,
    Arabic,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
    Thai,
    Lao,
    Tibetan,
    Myanmar,
    Georgian,
    Ethiopic,
    Khmer,
    Hangul,
    HiraganaKatakana,
    CjkIdeograph,
    Other(u32),
    NonWord,
}

impl UnicodePool {
    /// Classifies a UTF-8 char into its Unicode script/block and estimates pool size.
    #[inline]
    pub fn classify(c: char) -> Self {
        let cp = u32::from(c);
        // Fast-path ASCII characters to avoid table lookups
        if cp <= 0x007F {
            return match cp {
                0x0061..=0x007A => Self::AsciiLower,
                0x0041..=0x005A => Self::AsciiUpper,
                0x0030..=0x0039 => Self::AsciiDigit,
                0x0021..=0x002F | 0x003A..=0x0040 | 0x005B..=0x0060 | 0x007B..=0x007E => {
                    Self::AsciiSymbol
                }
                _ => Self::OtherScript(0),
            };
        }

        match c.script() {
            Script::Latin => Self::LatinExtended,
            Script::Greek => Self::Greek,
            Script::Cyrillic => Self::Cyrillic,
            Script::Armenian => Self::Armenian,
            Script::Hebrew => Self::Hebrew,
            Script::Arabic => Self::Arabic,
            Script::Devanagari => Self::Devanagari,
            Script::Bengali => Self::Bengali,
            Script::Gurmukhi => Self::Gurmukhi,
            Script::Gujarati => Self::Gujarati,
            Script::Oriya => Self::Oriya,
            Script::Tamil => Self::Tamil,
            Script::Telugu => Self::Telugu,
            Script::Kannada => Self::Kannada,
            Script::Malayalam => Self::Malayalam,
            Script::Sinhala => Self::Sinhala,
            Script::Thai => Self::Thai,
            Script::Lao => Self::Lao,
            Script::Tibetan => Self::Tibetan,
            Script::Myanmar => Self::Myanmar,
            Script::Georgian => Self::Georgian,
            Script::Hangul => Self::Hangul,
            Script::Ethiopic => Self::Ethiopic,
            Script::Khmer => Self::Khmer,
            Script::Hiragana | Script::Katakana => Self::HiraganaKatakana,
            Script::Han => Self::CjkIdeograph,
            Script::Common | Script::Inherited | Script::Unknown => match cp {
                0x0080..=0x024F => Self::LatinExtended,
                0x2600..=0x27BF | 0x1F1E6..=0x1F1FF | 0x1F300..=0x1FAFF => Self::EmojiAndSymbols,
                // Group any other Unicode block by its 256-codepoint plane bucket
                _ => Self::OtherScript(cp >> 8),
            },
            // Group any other Unicode script by its Script discriminant
            other => Self::OtherScript(other as u32),
        }
    }

    /// Maps a character to its natural-language script family so multi-script runs
    /// (e.g., CJK + Hebrew + Cyrillic + Hangul) are never merged into a single word token.
    #[inline]
    pub fn script_family(c: char) -> ScriptFamily {
        if !c.is_alphabetic() {
            return ScriptFamily::NonWord;
        }
        match Self::classify(c) {
            Self::AsciiLower | Self::AsciiUpper | Self::LatinExtended => ScriptFamily::Latin,
            Self::Cyrillic => ScriptFamily::Cyrillic,
            Self::Greek => ScriptFamily::Greek,
            Self::Armenian => ScriptFamily::Armenian,
            Self::Hebrew => ScriptFamily::Hebrew,
            Self::Arabic => ScriptFamily::Arabic,
            Self::Devanagari => ScriptFamily::Devanagari,
            Self::Bengali => ScriptFamily::Bengali,
            Self::Gurmukhi => ScriptFamily::Gurmukhi,
            Self::Gujarati => ScriptFamily::Gujarati,
            Self::Oriya => ScriptFamily::Oriya,
            Self::Tamil => ScriptFamily::Tamil,
            Self::Telugu => ScriptFamily::Telugu,
            Self::Kannada => ScriptFamily::Kannada,
            Self::Malayalam => ScriptFamily::Malayalam,
            Self::Sinhala => ScriptFamily::Sinhala,
            Self::Thai => ScriptFamily::Thai,
            Self::Lao => ScriptFamily::Lao,
            Self::Tibetan => ScriptFamily::Tibetan,
            Self::Myanmar => ScriptFamily::Myanmar,
            Self::Georgian => ScriptFamily::Georgian,
            Self::Ethiopic => ScriptFamily::Ethiopic,
            Self::Khmer => ScriptFamily::Khmer,
            Self::Hangul => ScriptFamily::Hangul,
            Self::HiraganaKatakana => ScriptFamily::HiraganaKatakana,
            Self::CjkIdeograph => ScriptFamily::CjkIdeograph,
            Self::OtherScript(plane) => ScriptFamily::Other(plane),
            Self::AsciiDigit | Self::AsciiSymbol | Self::EmojiAndSymbols => ScriptFamily::NonWord,
        }
    }

    /// Approximate effective alphabet size for a given Unicode script/category.
    #[inline]
    pub fn pool_size(&self) -> f64 {
        match self {
            Self::AsciiLower | Self::AsciiUpper => 26.0,
            Self::AsciiDigit => 10.0,
            Self::AsciiSymbol => 33.0,
            Self::LatinExtended | Self::Devanagari | Self::Bengali | Self::Sinhala => 64.0,
            Self::Greek => 48.0,
            Self::Cyrillic => 66.0,
            Self::Armenian => 76.0,
            Self::Hebrew => 27.0,
            Self::Arabic | Self::OtherScript(_) => 50.0,
            Self::Gurmukhi | Self::Gujarati | Self::Oriya => 56.0,
            Self::Tamil => 48.0,
            Self::Telugu | Self::Kannada => 60.0,
            Self::Malayalam | Self::Tibetan => 68.0,
            Self::Thai => 62.0,
            Self::Lao => 52.0,
            Self::Myanmar | Self::Khmer => 74.0,
            Self::Georgian => 46.0,
            Self::Ethiopic => 300.0,
            Self::HiraganaKatakana => 96.0,
            Self::CjkIdeograph => 3500.0,
            Self::Hangul => 2350.0,
            Self::EmojiAndSymbols => 800.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Zeroize, ZeroizeOnDrop)]
pub enum SegmentKind {
    /// Raw non-repeating characters
    Literal(String),
    /// Word verified against the configured Aho-Corasick dictionary (scores log2(dict_len) bits)
    DictionaryWord(String),
    /// Unverified single-script word token when no dictionary matches
    UnverifiedWord(String),
    /// Immediate periodic repetition, e.g., "666" (period "6" x 3) or "aieaie" (period "aie" x 2)
    TandemRepeat { unit: String, repeats: usize, full_match: String },
    /// Previously seen substring or repeated word earlier in the password
    PreviousRepeat(String),
    /// Monotonic Unicode sequence, e.g., "1234", "abcd", "абвг" (or interleaved "h.g.f.e.d.c.b.a")
    Sequence(String),
    /// Spatial keyboard sequence across rows/columns/diagonals (QWERTY, QWERTZ, AZERTY, Dvorak, Colemak, Numpad, ЙЦУКЕН, Nordic)
    KeyboardSequence(String),
}

#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct Segment {
    pub kind: SegmentKind,
    pub entropy_bits: f64,
}

/// Lightweight, allocation-free DP segment representation referencing character indices.
#[derive(Clone, Copy, Zeroize)]
enum RawSegmentKind {
    Literal(usize),
    DictionaryWord(usize, usize),
    UnverifiedWord(usize, usize),
    TandemRepeat { start: usize, period: usize, repeats: usize },
    PreviousRepeat(usize, usize),
    Sequence(usize, usize),
    KeyboardSequence(usize, usize),
}

#[derive(Clone, Copy, Zeroize)]
struct RawSegment {
    kind: RawSegmentKind,
    entropy_bits: f64,
}

impl RawSegment {
    #[inline]
    fn slice_to_string(chars: &[char], start: usize, end: usize) -> String {
        chars.get(start..end).map(|s| s.iter().collect()).unwrap_or_default()
    }

    fn into_segment(self, chars: &[char]) -> Segment {
        let kind = match self.kind {
            RawSegmentKind::Literal(idx) => {
                SegmentKind::Literal(chars.get(idx).map(char::to_string).unwrap_or_default())
            }
            RawSegmentKind::DictionaryWord(s, e) => {
                SegmentKind::DictionaryWord(Self::slice_to_string(chars, s, e))
            }
            RawSegmentKind::UnverifiedWord(s, e) => {
                SegmentKind::UnverifiedWord(Self::slice_to_string(chars, s, e))
            }
            RawSegmentKind::TandemRepeat { start, period, repeats } => SegmentKind::TandemRepeat {
                unit: Self::slice_to_string(chars, start, start + period),
                repeats,
                full_match: Self::slice_to_string(chars, start, start + period * repeats),
            },
            RawSegmentKind::PreviousRepeat(s, e) => {
                SegmentKind::PreviousRepeat(Self::slice_to_string(chars, s, e))
            }
            RawSegmentKind::Sequence(s, e) => {
                SegmentKind::Sequence(Self::slice_to_string(chars, s, e))
            }
            RawSegmentKind::KeyboardSequence(s, e) => {
                SegmentKind::KeyboardSequence(Self::slice_to_string(chars, s, e))
            }
        };
        Segment { kind, entropy_bits: self.entropy_bits }
    }
}

/// Linearized keyboard rows, diagonals, and numpad sequences across:
/// - QWERTY / Nordic (FI/SE/NO/DK) / US & Nordic Shifted symbols
/// - QWERTZ (DE/Central Europe)
/// - AZERTY (FR/BE)
/// - Dvorak
/// - Colemak
/// - Numpad (rows, columns, diagonals)
/// - Cyrillic JCUKEN (ЙЦУКЕН)
const KEYBOARD_STRIPS: &[&str] = &[
    // QWERTY / Nordic rows + Shifted symbol rows
    "`1234567890-=",
    "~!@#$%^&*()_+",
    "§1234567890+´",
    "!\"#¤%&/()=?",
    "qwertyuiop[]\\",
    "qwertyuiopå¨'",
    "asdfghjkl;'\\",
    "asdfghjklöä'",
    "asdfghjklæø'",
    "<zxcvbnm,./",
    // QWERTY diagonals / columns (top-to-bottom across number & letter rows)
    "1qaz",
    "2wsx",
    "3edc",
    "4rfv",
    "5tgb",
    "6yhn",
    "7ujm",
    "8ik,",
    "9ol.",
    "0p;/",
    "0pö-",
    "!qaz",
    "@wsx",
    "#edc",
    "$rfv",
    "%tgb",
    "^yhn",
    "&ujm",
    "*ik,",
    "(ol.",
    // QWERTZ specific rows & diagonals
    "qwertzuiopü+",
    "asdfghjklöä#",
    "<yxcvbnm,.-",
    "1qay",
    "2wsx",
    "6zhn",
    "7ujm",
    // AZERTY specific rows & diagonals
    "&é\"'(-è_çà)=",
    "azertyuiop^$",
    "qsdfghjklmù*",
    "<wxcvbn,;:!",
    "1aqw",
    "2zsx",
    "3edc",
    // Dvorak rows & columns
    "',.pyfgcrl/=",
    "aoeuidhtns-",
    ";qjkxbmwvz",
    // Colemak rows & columns
    "qwfpgjluy;[]",
    "arstdhneio'",
    "zxcvbkm,./",
    "1qaz",
    "2wrx",
    "3fsx",
    "4ptc",
    "5gdv",
    // Numpad (horizontal, vertical, diagonal)
    "789",
    "456",
    "123",
    "0.",
    "7410",
    "8520",
    "963.",
    "753",
    "951",
    "159",
    "357",
    // Cyrillic JCUKEN (ЙЦУКЕН) rows & columns
    "ё1234567890-=",
    "йцукенгшщзхъ\\",
    "фывапролджэ",
    "ячсмитьбю.",
    "1йфя",
    "2цыч",
    "3увс",
    "4кам",
    "5епи",
    "6нрт",
    "7гоь",
    "8шлб",
    "9щдю",
    "0зж.",
];

const NUM_KB_STRIPS: usize = KEYBOARD_STRIPS.len();
const MAX_KB_KEYS: usize = 128;
const INVALID_KEY_ID: u8 = 0xFF;

/// Precompiled O(1) bitset & table-driven spatial keyboard automaton.
struct CompiledKeyboardData {
    /// Fast O(1) lookup for ASCII bytes (0..128) -> KeyId (or INVALID_KEY_ID).
    ascii_to_id: [u8; 128],
    /// Non-ASCII codepoints (Latin-1 & Cyrillic) -> KeyId.
    non_ascii_cps: [u32; 64],
    non_ascii_ids: [u8; 64],
    non_ascii_len: usize,
    /// For each KeyId, a 128-bit mask of which strips in `KEYBOARD_STRIPS` contain that key.
    key_strip_mask: [u128; MAX_KB_KEYS],
    /// For each (strip, KeyId), a 16-bit mask of positions (0..16) where KeyId occurs in that strip.
    strip_key_pos_mask: [[u16; MAX_KB_KEYS]; NUM_KB_STRIPS],
    /// Pre-decoded KeyId array for each strip (padded with INVALID_KEY_ID).
    strip_keys: [[u8; 16]; NUM_KB_STRIPS],
    /// 2D QWERTY grid coordinates `(row, col)` for each KeyId, or `(-1, -1)` if not on the 2D grid.
    qwerty_coords: [(i8, i8); MAX_KB_KEYS],
}

#[allow(clippy::indexing_slicing)]
static KB_DATA: CompiledKeyboardData = {
    let mut data = CompiledKeyboardData {
        ascii_to_id: [INVALID_KEY_ID; 128],
        non_ascii_cps: [0; 64],
        non_ascii_ids: [INVALID_KEY_ID; 64],
        non_ascii_len: 0,
        key_strip_mask: [0; MAX_KB_KEYS],
        strip_key_pos_mask: [[0; MAX_KB_KEYS]; NUM_KB_STRIPS],
        strip_keys: [[INVALID_KEY_ID; 16]; NUM_KB_STRIPS],
        qwerty_coords: [(-1, -1); MAX_KB_KEYS],
    };

    let mut next_key_id: u8 = 0;

    // Helper macro to intern a Unicode codepoint into a dense KeyId at compile time
    macro_rules! intern_cp {
        ($cp:expr) => {{
            let cp: u32 = $cp;
            if cp < 128 {
                let existing = data.ascii_to_id[cp as usize];
                if existing != INVALID_KEY_ID {
                    existing
                } else {
                    let id = next_key_id;
                    next_key_id += 1;
                    data.ascii_to_id[cp as usize] = id;
                    id
                }
            } else {
                let mut found = INVALID_KEY_ID;
                let mut idx = 0;
                while idx < data.non_ascii_len {
                    if data.non_ascii_cps[idx] == cp {
                        found = data.non_ascii_ids[idx];
                        break;
                    }
                    idx += 1;
                }
                if found != INVALID_KEY_ID {
                    found
                } else {
                    let id = next_key_id;
                    next_key_id += 1;
                    data.non_ascii_cps[data.non_ascii_len] = cp;
                    data.non_ascii_ids[data.non_ascii_len] = id;
                    data.non_ascii_len += 1;
                    id
                }
            }
        }};
    }

    // 1. Compile all KEYBOARD_STRIPS into dense KeyIds and 128-bit strip membership masks
    let mut s_idx = 0;
    while s_idx < NUM_KB_STRIPS {
        let bytes = KEYBOARD_STRIPS[s_idx].as_bytes();
        let mut b_idx = 0;
        let mut pos = 0;
        while b_idx < bytes.len() {
            // Const UTF-8 decode
            let b0 = bytes[b_idx];
            let (cp, step) = if b0 < 0x80 {
                (b0 as u32, 1)
            } else if (b0 & 0xE0) == 0xC0 {
                let b1 = bytes[b_idx + 1];
                (((b0 as u32 & 0x1F) << 6) | (b1 as u32 & 0x3F), 2)
            } else if (b0 & 0xF0) == 0xE0 {
                let b1 = bytes[b_idx + 1];
                let b2 = bytes[b_idx + 2];
                (((b0 as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F), 3)
            } else {
                let b1 = bytes[b_idx + 1];
                let b2 = bytes[b_idx + 2];
                let b3 = bytes[b_idx + 3];
                (
                    ((b0 as u32 & 0x07) << 18)
                        | ((b1 as u32 & 0x3F) << 12)
                        | ((b2 as u32 & 0x3F) << 6)
                        | (b3 as u32 & 0x3F),
                    4,
                )
            };
            let key_id = intern_cp!(cp);
            data.strip_keys[s_idx][pos] = key_id;
            data.key_strip_mask[key_id as usize] |= 1u128 << s_idx;
            data.strip_key_pos_mask[s_idx][key_id as usize] |= 1u16 << pos;
            pos += 1;
            b_idx += step;
        }
        s_idx += 1;
    }

    // 2. Also ensure all valid 2D QWERTY / European letters are interned and populate 2D coordinates
    const EXTRA_2D_LETTERS: &[(char, i8, i8)] = &[
        ('q', 0, 0),
        ('w', 0, 1),
        ('e', 0, 2),
        ('r', 0, 3),
        ('t', 0, 4),
        ('y', 0, 5),
        ('u', 0, 6),
        ('i', 0, 7),
        ('o', 0, 8),
        ('p', 0, 9),
        ('å', 0, 10),
        ('a', 1, 0),
        ('s', 1, 1),
        ('d', 1, 2),
        ('f', 1, 3),
        ('g', 1, 4),
        ('h', 1, 5),
        ('j', 1, 6),
        ('k', 1, 7),
        ('l', 1, 8),
        ('ö', 1, 9),
        ('ø', 1, 9),
        ('ä', 1, 10),
        ('æ', 1, 10),
        ('z', 2, 0),
        ('x', 2, 1),
        ('c', 2, 2),
        ('v', 2, 3),
        ('b', 2, 4),
        ('n', 2, 5),
        ('m', 2, 6),
    ];

    let mut q_idx = 0;
    while q_idx < EXTRA_2D_LETTERS.len() {
        let (ch, r, c) = EXTRA_2D_LETTERS[q_idx];
        let id = intern_cp!(ch as u32);
        data.qwerty_coords[id as usize] = (r, c);
        q_idx += 1;
    }

    data
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Zeroize)]
pub enum StrengthRating {
    VeryWeak,      // < 36 bits
    Weak,          // 36..60 bits
    Reasonable,    // 60..80 bits
    Strong,        // 80..128 bits
    Cryptographic, // >= 128 bits (e.g., 8 verified words from 200k list = ~141 bits)
}

impl StrengthRating {
    /// Returns the human-readable textual representation of the strength rating.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::VeryWeak => "Very Weak",
            Self::Weak => "Weak",
            Self::Reasonable => "Reasonable",
            Self::Strong => "Strong",
            Self::Cryptographic => "Cryptographic",
        }
    }

    /// Parses a textual string into a `StrengthRating`, ignoring case and delimiters.
    #[inline]
    pub fn from_text(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

impl std::fmt::Display for StrengthRating {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for StrengthRating {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseStrengthRatingError;

impl std::fmt::Display for ParseStrengthRatingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid password strength rating")
    }
}

impl std::error::Error for ParseStrengthRatingError {}

impl std::str::FromStr for StrengthRating {
    type Err = ParseStrengthRatingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = [0u8; 16];
        let mut len = 0;

        for &b in s.as_bytes() {
            if !b.is_ascii_whitespace() && b != b'_' && b != b'-' {
                let slot = buf.get_mut(len).ok_or(ParseStrengthRatingError)?;
                *slot = b.to_ascii_lowercase();
                len += 1;
            }
        }

        let normalized = buf.get(..len).ok_or(ParseStrengthRatingError)?;
        match normalized {
            b"veryweak" => Ok(Self::VeryWeak),
            b"weak" => Ok(Self::Weak),
            b"reasonable" => Ok(Self::Reasonable),
            b"strong" => Ok(Self::Strong),
            b"cryptographic" => Ok(Self::Cryptographic),
            _ => Err(ParseStrengthRatingError),
        }
    }
}

#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct PasswordAnalysis {
    pub normalized_length: usize,
    pub truncated: bool,
    pub entropy_bits: f64,
    pub rating: StrengthRating,
    pub segments: Vec<Segment>,
    pub warnings: Vec<String>,
}

pub struct PasswordChecker {
    /// Compiled Aho-Corasick automaton over NFC-normalized lowercase dictionary words.
    automaton: Option<AhoCorasick>,
    /// Precompiled FST dictionary for zero-cost startup.
    fst_dict: Option<fst::raw::Fst<Vec<u8>>>,
    /// Borrowed static FST dictionary (embedded in binary).
    embedded_fst: Option<fst::raw::Fst<&'static [u8]>>,
    /// Actual number of unique words in the configured dictionary.
    dict_size: usize,
    /// Conservative pool size assumed for unverified words (default: 100 -> ~6.64 bits).
    unverified_pool_size: usize,
    /// Maximum number of NFC characters to analyze (default: 100).
    max_chars: usize,
    /// Whether to emit warning strings (which may quote repeated/sequential substrings).
    emit_warnings: bool,
    /// Whether to retain matched password substrings inside `PasswordAnalysis::segments`.
    include_segments: bool,
}

impl Default for PasswordChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl PasswordChecker {
    /// Creates a checker with no dictionary loaded.
    pub fn new() -> Self {
        Self {
            automaton: None,
            fst_dict: None,
            embedded_fst: None,
            dict_size: 0,
            unverified_pool_size: DEFAULT_UNVERIFIED_POOL_SIZE,
            max_chars: DEFAULT_MAX_CHARS,
            emit_warnings: true,
            include_segments: true,
        }
    }

    /// Returns the number of unique words in the currently loaded dictionary.
    pub fn dictionary_size(&self) -> usize {
        self.dict_size
    }

    /// Configures the maximum number of NFC characters to check (bounds CPU usage).
    pub fn with_max_chars(mut self, max_chars: usize) -> Self {
        self.max_chars = max_chars.max(1);
        self
    }

    /// Uses the dictionary embedded at compile time via `UNIPASTREN_BUILD_DICTS` (zero-cost instant load).
    /// If compiled without `UNIPASTREN_BUILD_DICTS`, `dictionary_size()` will be 0.
    pub fn with_embedded_dictionary(mut self) -> Self {
        self.automaton = None;
        self.fst_dict = None;
        if let Ok(raw_fst) = fst::raw::Fst::new(EMBEDDED_DICT_FST) {
            self.dict_size = raw_fst.len();
            if self.dict_size > 0 {
                self.embedded_fst = Some(raw_fst);
            } else {
                self.embedded_fst = None;
            }
        }
        self
    }

    /// Loads a precompiled FST byte vector (e.g. read from disk).
    pub fn with_fst_bytes(mut self, bytes: Vec<u8>) -> Result<Self, fst::Error> {
        let raw_fst = fst::raw::Fst::new(bytes)?;
        self.dict_size = raw_fst.len();
        self.automaton = None;
        self.embedded_fst = None;
        self.fst_dict = if self.dict_size > 0 { Some(raw_fst) } else { None };
        Ok(self)
    }

    /// Configures the fallback pool size for unverified words (default: 100).
    pub fn with_unverified_pool_size(mut self, pool_size: usize) -> Self {
        self.unverified_pool_size = pool_size.max(2);
        self
    }

    /// Enables or disables generating warnings (which can contain sensitive substrings from the password).
    pub fn with_warnings(mut self, emit_warnings: bool) -> Self {
        self.emit_warnings = emit_warnings;
        self
    }

    /// Enables or disables storing diagnostic segments in the returned `PasswordAnalysis`.
    /// Set both `with_warnings(false)` and `with_segments(false)` for zero password substring retention.
    pub fn with_segments(mut self, include_segments: bool) -> Self {
        self.include_segments = include_segments;
        self
    }

    /// Maps a character to its dense compiled `KeyId` (`0..120`, or `INVALID_KEY_ID` if not on any keyboard).
    #[inline(always)]
    fn kb_key_id(c: char) -> u8 {
        if c.is_ascii() {
            let b = c.to_ascii_lowercase() as usize;
            return KB_DATA.ascii_to_id.get(b).copied().unwrap_or(INVALID_KEY_ID);
        }
        let lower = u32::from(c.to_lowercase().next().unwrap_or(c));
        let len = KB_DATA.non_ascii_len;
        if let (Some(cps), Some(ids)) =
            (KB_DATA.non_ascii_cps.get(..len), KB_DATA.non_ascii_ids.get(..len))
        {
            for (&cp, &id) in cps.iter().zip(ids.iter()) {
                if cp == lower {
                    return id;
                }
            }
        }
        INVALID_KEY_ID
    }

    /// Ultra-fast spatial walk detector operating directly on pre-mapped `KeyId`s.
    /// Uses a 128-bit strip intersection (`AND` + `trailing_zeros`) and O(1) 2D coordinate tables.
    #[inline]
    fn keyboard_walk_len_ids(key_ids: &[u8]) -> usize {
        let (Some(&k0), Some(&k1)) = (key_ids.first(), key_ids.get(1)) else {
            return 0;
        };
        if k0 == INVALID_KEY_ID || k1 == INVALID_KEY_ID || k0 == k1 {
            return 0;
        }

        let k0_idx = usize::from(k0);
        let k1_idx = usize::from(k1);
        let (Some(&mask0), Some(&mask1)) =
            (KB_DATA.key_strip_mask.get(k0_idx), KB_DATA.key_strip_mask.get(k1_idx))
        else {
            return 0;
        };

        let mut best_len = 0usize;

        // 1. 128-bit Bitwise Strip Intersection:
        // Finds all strips containing both `k0` and `k1` in a single 128-bit AND instruction!
        let mut candidate_strips = mask0 & mask1;

        while candidate_strips != 0 {
            let s_idx = candidate_strips.trailing_zeros() as usize;
            candidate_strips &= candidate_strips - 1; // Clear lowest set bit

            let (Some(active_strip), Some(strip_pos_masks)) =
                (KB_DATA.strip_keys.get(s_idx), KB_DATA.strip_key_pos_mask.get(s_idx))
            else {
                continue;
            };
            let (Some(&pos0_mask), Some(&pos1_mask)) =
                (strip_pos_masks.get(k0_idx), strip_pos_masks.get(k1_idx))
            else {
                continue;
            };

            // Fast bitwise adjacency check: does k1 appear at pos0 - 1 or pos0 + 1?
            let adj_pos0 = pos0_mask & ((pos1_mask << 1) | (pos1_mask >> 1));
            if adj_pos0 == 0 {
                continue;
            }

            let mut p0_bits = adj_pos0;
            while p0_bits != 0 {
                let pos0 = p0_bits.trailing_zeros() as usize;
                p0_bits &= p0_bits - 1;

                for init_next in [pos0.wrapping_sub(1), pos0 + 1] {
                    if active_strip.get(init_next).copied() != Some(k1) {
                        continue;
                    }
                    let mut len = 2usize;
                    let mut cur_pos = init_next;
                    let mut prev_pos = pos0;

                    while let Some(&kn) = key_ids.get(len) {
                        if kn == INVALID_KEY_ID {
                            break;
                        }
                        let (f_pos, r_pos) = if cur_pos > prev_pos {
                            (cur_pos + 1, cur_pos.wrapping_sub(1))
                        } else {
                            (cur_pos.wrapping_sub(1), cur_pos + 1)
                        };

                        if active_strip.get(f_pos).copied() == Some(kn) {
                            prev_pos = cur_pos;
                            cur_pos = f_pos;
                            len += 1;
                        } else if active_strip.get(r_pos).copied() == Some(kn) {
                            prev_pos = cur_pos;
                            cur_pos = r_pos;
                            len += 1;
                        } else {
                            break;
                        }
                    }
                    if len > best_len {
                        best_len = len;
                    }
                }
            }
        }

        // 2. O(1) 2D QWERTY Coordinate Grid Lookup (supports 8-way walks, zigzags,
        // and multi-row carriage-return block walks like "asdzxcfghvbn" or "qweasdzxc")
        if let (Some(&(r0, c0)), Some(&(r1, c1))) =
            (KB_DATA.qwerty_coords.get(k0_idx), KB_DATA.qwerty_coords.get(k1_idx))
            && r0 >= 0
            && r1 >= 0
        {
            let dr = r1 - r0;
            let dc = c1 - c0;
            if dr.abs() <= 1 && dc.abs() <= 1 && (dr != 0 || dc != 0) {
                let mut len = 2usize;
                let mut prev_r = r0;
                let mut prev_c = c0;
                let mut curr_r = r1;
                let mut curr_c = c1;

                while let Some(&kn) = key_ids.get(len) {
                    let Some(&(next_r, next_c)) = KB_DATA.qwerty_coords.get(usize::from(kn)) else {
                        break;
                    };
                    if next_r < 0 {
                        break;
                    }
                    let step_dr = next_r - curr_r;
                    let step_dc = next_c - curr_c;
                    let prev_dc = curr_c - prev_c;

                    // Standard 8-neighbor King's step (no immediate horizontal reversal on same row)
                    let is_adjacent_step = step_dr.abs() <= 1
                        && step_dc.abs() <= 1
                        && (step_dr != 0 || step_dc != 0)
                        && (next_r != prev_r || step_dc * prev_dc >= 0);

                    if is_adjacent_step {
                        prev_r = curr_r;
                        prev_c = curr_c;
                        curr_r = next_r;
                        curr_c = next_c;
                        len += 1;
                        continue;
                    }

                    // Carriage-return / parallel row-block step (e.g. "asdzxcfghvbn" or "qweasdzxc"):
                    // After walking horizontally along a row (curr_r == prev_r, |prev_dc| == 1),
                    // jumping to an adjacent row (|step_dr| == 1, |step_dc| <= 3) is valid if the
                    // next key after `kn` continues walking horizontally in the same direction (`prev_dc`).
                    if curr_r == prev_r
                        && prev_dc.abs() == 1
                        && step_dr.abs() == 1
                        && step_dc.abs() <= 3
                        && let Some(&after_kn) = key_ids.get(len + 1)
                        && let Some(&(after_r, after_c)) =
                            KB_DATA.qwerty_coords.get(usize::from(after_kn))
                        && after_r == next_r
                        && (after_c - next_c) == prev_dc
                    {
                        prev_r = curr_r;
                        prev_c = curr_c;
                        curr_r = next_r;
                        curr_c = next_c;
                        len += 1;
                        continue;
                    }

                    break;
                }
                if len > best_len {
                    best_len = len;
                }
            }
        }
        best_len
    }

    /// Folds a character into its NFKD base lowercase characters with combining diacritics stripped.
    /// Falls back to standard lowercase if stripping combining marks would leave it empty.
    #[inline]
    fn fold_char(c: char) -> impl Iterator<Item = char> {
        let mut buf = ['\0'; 8];
        let mut len = 0usize;
        for d in c.nfkd().filter(|&d| !is_combining_mark(d)).flat_map(|d| d.to_lowercase()) {
            if let Some(slot) = buf.get_mut(len) {
                *slot = d;
                len += 1;
            }
        }
        if len == 0 {
            for d in c.to_lowercase() {
                if let Some(slot) = buf.get_mut(len) {
                    *slot = d;
                    len += 1;
                }
            }
        }
        buf.into_iter().take(len)
    }

    /// Loads custom UTF-8 dictionary words (any language), normalizes them to NFKD with diacritics
    /// stripped + lowercase, builds an Aho-Corasick automaton, and sets `dict_size` to the actual
    /// number of unique words.
    pub fn with_dictionary<I, S>(self, words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.with_dictionaries(std::iter::once(words))
    }

    /// Loads multiple sets of custom UTF-8 dictionary words (any language), normalizes them to NFKD
    /// with diacritics stripped + lowercase, builds an Aho-Corasick automaton, and sets `dict_size`
    /// to the actual total number of unique words across all provided dictionaries.
    pub fn with_dictionaries<D, I, S>(mut self, dictionaries: D) -> Self
    where
        D: IntoIterator<Item = I>,
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.fst_dict = None;
        self.embedded_fst = None;

        let mut unique_words = HashSet::new();
        for words in dictionaries {
            for w in words {
                // Normalize each dictionary entry to NFKD, strip combining diacritics, and lowercase
                let normalized: String =
                    w.as_ref().trim().nfc().flat_map(Self::fold_char).collect();
                if normalized.chars().count() >= 2 {
                    unique_words.insert(normalized);
                }
            }
        }

        self.dict_size = unique_words.len();
        if self.dict_size > 0 {
            let patterns: Vec<String> = unique_words.into_iter().collect();
            self.automaton =
                AhoCorasickBuilder::new().match_kind(MatchKind::Standard).build(&patterns).ok();
            if self.automaton.is_none() {
                self.dict_size = 0;
            }
        } else {
            self.automaton = None;
        }
        self
    }

    /// Walks a raw FST automaton over `bytes` starting at `start_byte` and invokes `on_match(end_byte)`
    /// every time a final state is reached. Runs in O(word_length) with zero allocations.
    #[inline]
    fn find_fst_prefixes<D: AsRef<[u8]>, F: FnMut(usize)>(
        fst: &fst::raw::Fst<D>,
        bytes: &[u8],
        start_byte: usize,
        mut on_match: F,
    ) {
        let mut node = fst.root();
        let Some(tail) = bytes.get(start_byte..) else {
            return;
        };
        for (offset, &b) in tail.iter().enumerate() {
            if let Some(trans_idx) = node.find_input(b) {
                let trans = node.transition(trans_idx);
                node = fst.node(trans.addr);
                if node.is_final() {
                    on_match(start_byte + offset + 1);
                }
            } else {
                break;
            }
        }
    }

    /// Checks if an exact string exists in the configured dictionary (FST or Aho-Corasick).
    #[inline]
    fn is_exact_dict_match(&self, s: &str) -> bool {
        if let Some(ref fst) = self.embedded_fst {
            return fst.contains_key(s.as_bytes());
        }
        if let Some(ref fst) = self.fst_dict {
            return fst.contains_key(s.as_bytes());
        }
        if let Some(ref ac) = self.automaton {
            for mat in ac.find_iter(s) {
                if mat.start() == 0 && mat.end() == s.len() {
                    return true;
                }
                if mat.start() > 0 {
                    break;
                }
            }
        }
        false
    }

    /// Entropy in bits for a verified word from the configured dictionary.
    #[inline]
    fn verified_word_entropy(&self) -> f64 {
        (self.dict_size.max(2) as f64).log2()
    }

    /// Base entropy in bits for an unverified word in the default Latin script (top-1000 -> 10 bits).
    #[inline]
    fn unverified_word_entropy(&self) -> f64 {
        (self.unverified_pool_size.max(2) as f64).log2()
    }

    /// Estimates single-character entropy based on its Unicode script pool.
    #[inline]
    fn char_entropy(c: char) -> f64 {
        if c.is_whitespace() {
            return REPEATED_LITERAL_ENTROPY_BITS; // Minimal entropy for space delimiters
        }
        UnicodePool::classify(c).pool_size().log2()
    }

    /// Raw baseline entropy of a slice of characters without heap allocations.
    fn raw_slice_entropy(slice: &[char]) -> f64 {
        let mut mask: u32 = 0;
        let mut total_pool: f64 = 0.0;
        let mut other_planes = [u32::MAX; 32];
        let mut other_len = 0usize;

        for &c in slice {
            if c.is_whitespace() {
                continue;
            }
            let pool = UnicodePool::classify(c);
            let bit: Option<u32> = match pool {
                UnicodePool::AsciiLower => Some(1 << 0),
                UnicodePool::AsciiUpper => Some(1 << 1),
                UnicodePool::AsciiDigit => Some(1 << 2),
                UnicodePool::AsciiSymbol => Some(1 << 3),
                UnicodePool::LatinExtended => Some(1 << 4),
                UnicodePool::Cyrillic => Some(1 << 5),
                UnicodePool::Greek => Some(1 << 6),
                UnicodePool::Armenian => Some(1 << 7),
                UnicodePool::Hebrew => Some(1 << 8),
                UnicodePool::Arabic => Some(1 << 9),
                UnicodePool::Devanagari => Some(1 << 10),
                UnicodePool::Bengali => Some(1 << 11),
                UnicodePool::Gurmukhi => Some(1 << 12),
                UnicodePool::Gujarati => Some(1 << 13),
                UnicodePool::Oriya => Some(1 << 14),
                UnicodePool::Tamil => Some(1 << 15),
                UnicodePool::Telugu => Some(1 << 16),
                UnicodePool::Kannada => Some(1 << 17),
                UnicodePool::Malayalam => Some(1 << 18),
                UnicodePool::Sinhala => Some(1 << 19),
                UnicodePool::Thai => Some(1 << 20),
                UnicodePool::Lao => Some(1 << 21),
                UnicodePool::Tibetan => Some(1 << 22),
                UnicodePool::Myanmar => Some(1 << 23),
                UnicodePool::Georgian => Some(1 << 24),
                UnicodePool::Ethiopic => Some(1 << 25),
                UnicodePool::Khmer => Some(1 << 26),
                UnicodePool::Hangul => Some(1 << 27),
                UnicodePool::HiraganaKatakana => Some(1 << 28),
                UnicodePool::CjkIdeograph => Some(1 << 29),
                UnicodePool::EmojiAndSymbols => Some(1 << 30),
                UnicodePool::OtherScript(plane) => {
                    if !other_planes.get(..other_len).is_some_and(|s| s.contains(&plane))
                        && let Some(slot) = other_planes.get_mut(other_len)
                    {
                        *slot = plane;
                        other_len += 1;
                        total_pool += pool.pool_size();
                    }
                    None
                }
            };
            if let Some(b) = bit
                && (mask & b) == 0
            {
                mask |= b;
                total_pool += pool.pool_size();
            }
        }
        (slice.len() as f64) * total_pool.max(2.0).log2()
    }

    /// Returns true if `unit` is a primitive period (not itself a repetition of a smaller divisor period).
    #[inline]
    fn is_primitive_period(unit: &[char]) -> bool {
        let p = unit.len();
        for d in 1..=(p / 2) {
            if p.is_multiple_of(d)
                && let Some(prefix) = unit.get(..d)
                && unit.chunks_exact(d).skip(1).all(|chunk| chunk == prefix)
            {
                return false;
            }
        }
        true
    }

    /// Returns true if a slice of characters has natural word casing
    /// (all lowercase, ALL UPPERCASE, or TitleCase), rather than random mixed-case ("yYwKkFIq...").
    #[inline]
    fn has_natural_word_casing(word: &[char]) -> bool {
        let has_lower = word.iter().any(|c| c.is_lowercase());
        let has_upper_after_first =
            word.get(1..).is_some_and(|rest| rest.iter().any(|c| c.is_uppercase()));
        !(has_lower && has_upper_after_first)
    }

    /// Prices a constant interleaved separator (e.g. "." in "h.g.f.e.d.c.b.a" or "22" in "b22a22t22...").
    /// If the separator itself is a single repeated character (like "22"), its internal repetition is discounted.
    fn separator_entropy(sep: &[char]) -> f64 {
        let Some(&first) = sep.first() else {
            return 0.0;
        };
        let first_bits = Self::char_entropy(first);
        if sep.len() == 1 {
            first_bits
        } else if sep.iter().all(|&c| c == first) {
            first_bits + (sep.len() as f64).log2()
        } else {
            Self::raw_slice_entropy(sep)
        }
    }

    /// Estimates the entropy of an unverified single-script alphabetic run.
    /// Single short words score conservatively (`unverified_bits`, ~6.64 bits for Latin), while long
    /// unspaced compound runs (e.g. "monkeysikapaviaaniachtungisland") scale with estimated word count
    /// and compound/multilingual word-choice diversity.
    fn unverified_run_entropy(&self, run: &[char], family: ScriptFamily) -> f64 {
        let run_len = run.len();
        let unverified_bits = self.unverified_word_entropy();
        let script_choice_bits = UNICODE_SCRIPT_COUNT.log2();
        let single_char_bits = run.first().copied().map_or(0.0, Self::char_entropy);

        let base_word_bits = if family == ScriptFamily::Latin {
            unverified_bits
        } else {
            (unverified_bits + script_choice_bits).max(single_char_bits + script_choice_bits)
        };

        // Single word (<= 9 chars in Latin/alphabetic scripts)
        if run_len <= 9 {
            return base_word_bits;
        }

        // Long compound word run without spaces (e.g. "monkeysikapaviaaniachtungisland"):
        // Estimate constituent words using ~6 chars/word average and add compound/multilingual
        // vocabulary scaling (each concatenated word beyond the first reflects a broader ~1000-word
        // vocabulary + language/combinatorial choice -> ~10.5 bits/word).
        let estimated_words = (run_len as f64 / 6.0).ceil();
        let compound_per_word_bits = base_word_bits + 4.0;
        estimated_words * compound_per_word_bits
    }

    /// Relaxes a DP state transition with a single bounds check.
    #[inline(always)]
    fn relax_dp(
        dp: &mut [(f64, usize, Option<RawSegment>)],
        target: usize,
        new_cost: f64,
        prev: usize,
        seg: RawSegment,
    ) {
        if let Some(cell) = dp.get_mut(target)
            && new_cost < cell.0
        {
            *cell = (new_cost, prev, Some(seg));
        }
    }

    /// Evaluates password strength and zeroizes the input `String` buffer before returning.
    pub fn analyze_and_zeroize(&self, password: &mut String) -> PasswordAnalysis {
        let result = self.analyze(password.as_str());
        password.zeroize();
        result
    }

    /// Evaluates password strength across all UTF-8 scripts using NFC, Aho-Corasick, and DP.
    /// All internal temporary buffers containing password characters or substrings are zeroized before return.
    pub fn analyze(&self, password: &str) -> PasswordAnalysis {
        // Lazily NFC-normalize and take at most max_chars + 1 to detect truncation in O(max_chars) time
        let mut nfc_iter = password.nfc();
        let mut chars: Vec<char> = Vec::with_capacity(self.max_chars.min(password.len()));
        for _ in 0..self.max_chars {
            if let Some(c) = nfc_iter.next() {
                chars.push(c);
            } else {
                break;
            }
        }
        let truncated = nfc_iter.next().is_some();

        let n = chars.len();
        if n == 0 {
            chars.zeroize();
            return PasswordAnalysis {
                normalized_length: 0,
                truncated: false,
                entropy_bits: 0.0,
                rating: StrengthRating::VeryWeak,
                segments: Vec::new(),
                warnings: if self.emit_warnings {
                    vec!["Password is empty.".to_string()]
                } else {
                    Vec::new()
                },
            };
        }

        // 1. Precompute kb_ids once alongside lower_chars and families:
        let mut lower_chars: Vec<char> =
            chars.iter().map(|&c| Self::fold_char(c).next().unwrap_or(c)).collect();
        let mut kb_ids: Vec<u8> = chars.iter().map(|&c| Self::kb_key_id(c)).collect();
        let families: Vec<ScriptFamily> =
            chars.iter().map(|&c| UnicodePool::script_family(c)).collect();
        let mut lower_str = String::with_capacity(n * 4);
        let mut byte_to_char = Vec::with_capacity(n * 4 + 1);
        for (char_idx, &ch) in chars.iter().enumerate() {
            let start_len = lower_str.len();
            for folded in Self::fold_char(ch) {
                lower_str.push(folded);
            }
            let end_len = lower_str.len();
            for _ in start_len..end_len {
                byte_to_char.push(char_idx);
            }
        }
        byte_to_char.push(n);

        // Precompute UAX #29 extended grapheme cluster continuations (for combined emoji / ZWJ sequences)
        let mut is_grapheme_continuation = vec![false; n];
        for (byte_idx, grapheme) in lower_str.grapheme_indices(true) {
            if let (Some(&start_c), Some(&end_c)) =
                (byte_to_char.get(byte_idx), byte_to_char.get(byte_idx + grapheme.len()))
                && start_c + 1 < end_c
                && let Some(cont_slice) =
                    is_grapheme_continuation.get_mut(start_c + 1..end_c.min(n))
            {
                cont_slice.fill(true);
            }
        }

        // Precompute UAX #29 word segment lengths in O(n) time before zeroizing lower_str
        let mut uax29_word_end = vec![0usize; n];
        for (byte_idx, word) in lower_str.unicode_word_indices() {
            if let (Some(&start_c), Some(&end_c)) =
                (byte_to_char.get(byte_idx), byte_to_char.get(byte_idx + word.len()))
                && let Some(word_slice) = uax29_word_end.get_mut(start_c..end_c.min(n))
            {
                word_slice.fill(end_c);
            }
        }

        // Precompute all dictionary matches using embedded FST, owned FST, or Aho-Corasick
        let mut dict_matches_at: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut covered_by_dict = vec![false; n];
        if self.embedded_fst.is_some() || self.fst_dict.is_some() {
            let lower_bytes = lower_str.as_bytes();
            let mut prev_char = usize::MAX;
            for (byte_idx, &start_c) in byte_to_char.iter().enumerate().take(lower_bytes.len()) {
                if start_c == prev_char {
                    continue;
                }
                prev_char = start_c;
                let record_match = |end_byte: usize| {
                    if let Some(&end_c) = byte_to_char.get(end_byte)
                        && start_c < end_c
                        && end_c <= n
                        && byte_to_char.get(end_byte.saturating_sub(1)) != Some(&end_c)
                    {
                        if let Some(matches) = dict_matches_at.get_mut(start_c) {
                            matches.push(end_c);
                        }
                        if let Some(cov_slice) = covered_by_dict.get_mut(start_c..end_c) {
                            cov_slice.fill(true);
                        }
                    }
                };
                if let Some(ref fst) = self.embedded_fst {
                    Self::find_fst_prefixes(fst, lower_bytes, byte_idx, record_match);
                } else if let Some(ref fst) = self.fst_dict {
                    Self::find_fst_prefixes(fst, lower_bytes, byte_idx, record_match);
                }
            }
        } else if let Some(ref ac) = self.automaton {
            for mat in ac.find_overlapping_iter(&lower_str) {
                if let (Some(&start_c), Some(&end_c)) =
                    (byte_to_char.get(mat.start()), byte_to_char.get(mat.end()))
                    && start_c < end_c
                    && end_c <= n
                {
                    if let Some(matches) = dict_matches_at.get_mut(start_c) {
                        matches.push(end_c);
                    }
                    if let Some(cov_slice) = covered_by_dict.get_mut(start_c..end_c) {
                        cov_slice.fill(true);
                    }
                }
            }
        }

        // Zeroize temporary lowercase string & byte map immediately after Aho-Corasick and UAX #29 matching
        lower_str.zeroize();
        byte_to_char.zeroize();

        // Count how many distinct alphabetic scripts are present in the password.
        // If >= 3 scripts are mixed (e.g., "雪山אלףбелый강물🌺kissa"), non-dictionary runs are not
        // treated as simple single-language top-1000 words.
        let mut distinct_scripts = HashSet::new();
        for &sf in &families {
            if sf != ScriptFamily::NonWord {
                distinct_scripts.insert(sf);
            }
        }
        let multi_script_mix = distinct_scripts.len() >= 3;

        // dp[i] holds (min_entropy_up_to_i, prev_index, RawSegment)
        let mut dp: Vec<(f64, usize, Option<RawSegment>)> = vec![(f64::INFINITY, 0, None); n + 1];
        if let Some(first_dp) = dp.first_mut() {
            *first_dp = (0.0, 0, None);
        }

        let verified_bits = self.verified_word_entropy();
        let unverified_bits = self.unverified_word_entropy();

        let mut core_chars = Vec::with_capacity(n);
        let mut core_lower = Vec::with_capacity(n);
        let mut core_kb_ids = Vec::with_capacity(n);
        let mut core_lower_str = String::with_capacity(n * 4);

        for i in 0..n {
            let Some(&(current_cost, _, _)) = dp.get(i) else {
                continue;
            };
            if current_cost.is_infinite() {
                continue;
            }

            let (Some(&ch_i), Some(&lower_i), Some(&kb_i), Some(&family)) =
                (chars.get(i), lower_chars.get(i), kb_ids.get(i), families.get(i))
            else {
                continue;
            };

            let is_continuation = is_grapheme_continuation.get(i).copied().unwrap_or(false);

            // 1. Baseline transition: single UTF-8 character / grapheme continuation.
            // Continuation codepoints inside a multi-scalar extended grapheme cluster (e.g. ZWJ emoji
            // or skin-tone modifiers) add 0 bits so combined emojis score as a single grapheme.
            // Repeated NonWord delimiters/symbols are discounted to REPEATED_LITERAL_ENTROPY_BITS.
            let is_repeated_nonword = family == ScriptFamily::NonWord
                && !ch_i.is_ascii_digit()
                && lower_chars.get(..i).is_some_and(|prefix| prefix.contains(&lower_i));
            let step_cost = if is_continuation {
                0.0
            } else if is_repeated_nonword {
                Self::char_entropy(ch_i).min(REPEATED_LITERAL_ENTROPY_BITS)
            } else {
                Self::char_entropy(ch_i)
            };
            Self::relax_dp(
                &mut dp,
                i + 1,
                current_cost + step_cost,
                i,
                RawSegment { kind: RawSegmentKind::Literal(i), entropy_bits: step_cost },
            );

            if is_continuation {
                continue;
            }

            // 2. Tandem / periodic repeat detection starting at i (e.g., "666", "aieaie", "👍🏽👍🏽👍🏽")
            let mut has_tandem_repeat = false;
            let max_period = ((n - i) / 2).min(MAX_REPEAT_PERIOD);
            for period in 1..=max_period {
                if is_grapheme_continuation.get(i + period).copied().unwrap_or(false) {
                    continue;
                }
                let Some(unit_lower) = lower_chars.get(i..i + period) else {
                    break;
                };
                // Skip non-primitive periods (e.g., if unit "aaaa" is already a repeat of "a")
                // so a single repeating unit isn't artificially grouped into a larger period.
                if period > 1 && !Self::is_primitive_period(unit_lower) {
                    continue;
                }

                let tail_matches = lower_chars.get(i + period..).map_or(0, |tail| {
                    tail.iter().zip(unit_lower.iter().cycle()).take_while(|(a, b)| a == b).count()
                });
                let match_len = period + tail_matches;

                let repeats = match_len / period;
                let clean_len = repeats * period;
                if is_grapheme_continuation.get(i + clean_len).copied().unwrap_or(false) {
                    continue;
                }

                let unit_graphemes = is_grapheme_continuation
                    .get(i..i + period)
                    .map_or(period, |s| s.iter().filter(|&&cont| !cont).count().max(1));

                // Require >= 3 repeats for single graphemes ("666", "👍🏽👍🏽👍🏽") so normal double
                // letters in words ("kissa") aren't misclassified, and >= 2 repeats for multi-grapheme units.
                if ((unit_graphemes == 1 && repeats >= 3) || (unit_graphemes >= 2 && repeats >= 2))
                    && let (Some(unit_slice), Some(unit_families)) =
                        (chars.get(i..i + period), families.get(i..i + period))
                {
                    has_tandem_repeat = true;

                    // Scale raw entropy by grapheme count so multi-codepoint emojis aren't overpriced,
                    // and cap alphabetic repeat units at unverified_bits.
                    let raw_base = Self::raw_slice_entropy(unit_slice)
                        * (unit_graphemes as f64 / period as f64);
                    let base_bits = if unit_graphemes >= 2
                        && unit_families.iter().all(|&f| f != ScriptFamily::NonWord)
                    {
                        raw_base.min(unverified_bits)
                    } else {
                        raw_base
                    };
                    let repeat_bits = base_bits + (repeats as f64).log2();

                    Self::relax_dp(
                        &mut dp,
                        i + clean_len,
                        current_cost + repeat_bits,
                        i,
                        RawSegment {
                            kind: RawSegmentKind::TandemRepeat { start: i, period, repeats },
                            entropy_bits: repeat_bits,
                        },
                    );
                }
            }

            // 3. Previously seen substring (LZ77 backreference for gapped repeats & repeated words)
            if i >= 2 && !ch_i.is_whitespace() {
                let right_non_ws = chars
                    .get(i..)
                    .map_or(0, |s| s.iter().take_while(|c| !c.is_whitespace()).count());
                let right_lower = lower_chars.get(i..i + right_non_ws).unwrap_or(&[]);

                let mut best_prev_len = 0;
                let max_possible = n - i;
                if let Some(window_up_to_i) = lower_chars.get(..=i) {
                    for prev_start in 0..i {
                        if let Some(candidate) = window_up_to_i.get(prev_start..)
                            && candidate.first() == Some(&lower_i)
                        {
                            let len = candidate
                                .iter()
                                .zip(right_lower.iter())
                                .take_while(|(a, b)| a == b)
                                .count();
                            if len > best_prev_len {
                                best_prev_len = len;
                                if best_prev_len == max_possible {
                                    break;
                                }
                            }
                        }
                    }
                }

                if best_prev_len >= 3 {
                    let ref_bits = (i as f64 + 1.0).log2();
                    Self::relax_dp(
                        &mut dp,
                        i + best_prev_len,
                        current_cost + ref_bits,
                        i,
                        RawSegment {
                            kind: RawSegmentKind::PreviousRepeat(i, i + best_prev_len),
                            entropy_bits: ref_bits,
                        },
                    );
                }
            }

            // 4. Monotonic Unicode codepoint sequences (e.g., "12345", "abcdef", "h.g.f.e.d.c.b.a")
            let mut has_sequence = false;
            if let Some(tail) = chars.get(i..)
                && let Some(&next_ch) = tail.get(1)
            {
                let delta = (next_ch as i64) - (ch_i as i64);
                if delta.abs() == 1 {
                    let seq_len = 1 + tail
                        .windows(2)
                        .take_while(|w| matches!(w, [a, b] if (*b as i64) - (*a as i64) == delta))
                        .count();
                    if seq_len >= 3 {
                        has_sequence = true;
                        let seq_bits = Self::char_entropy(ch_i) + (seq_len as f64).log2() + 1.0;
                        Self::relax_dp(
                            &mut dp,
                            i + seq_len,
                            current_cost + seq_bits,
                            i,
                            RawSegment {
                                kind: RawSegmentKind::Sequence(i, i + seq_len),
                                entropy_bits: seq_bits,
                            },
                        );
                    }
                }

                // 4b. Spatial keyboard walks (QWERTY, QWERTZ, AZERTY, Dvorak, Colemak, Numpad, ЙЦУКЕН, Nordic)
                // Require >= 4 keys (or >= 3 if not purely alphabetic, e.g., "!@#$" or "147") so 3-letter words
                // that happen to sit adjacent on a layout (like "ert", "sad", "was") aren't over-flagged.
                let kb_len = kb_ids.get(i..).map_or(0, Self::keyboard_walk_len_ids);
                let min_kb_len = if family == ScriptFamily::NonWord { 3 } else { 4 };
                if kb_len >= min_kb_len {
                    has_sequence = true;
                    // Starting key choice + log2(walk_len) + ~2 bits for layout/direction choice
                    let kb_bits = Self::char_entropy(ch_i) + (kb_len as f64).log2() + 2.0;
                    Self::relax_dp(
                        &mut dp,
                        i + kb_len,
                        current_cost + kb_bits,
                        i,
                        RawSegment {
                            kind: RawSegmentKind::KeyboardSequence(i, i + kb_len),
                            entropy_bits: kb_bits,
                        },
                    );
                }
            }

            // 5. Aho-Corasick verified dictionary matches (supports compound words without spaces)
            if let Some(ends) = dict_matches_at.get(i) {
                for &end in ends {
                    Self::relax_dp(
                        &mut dp,
                        end,
                        current_cost + verified_bits,
                        i,
                        RawSegment {
                            kind: RawSegmentKind::DictionaryWord(i, end),
                            entropy_bits: verified_bits,
                        },
                    );
                }
            }

            // 6. Unverified single-script word fallback:
            // Active when the password is not a multi-script collage (>= 3 distinct scripts).
            // Uses UAX #29 word boundaries intersected with single-script uncovered runs.
            let is_covered = covered_by_dict.get(i).copied().unwrap_or(false);
            if !multi_script_mix && family != ScriptFamily::NonWord && !is_covered {
                let is_script_start = i == 0
                    || families.get(i - 1).copied() != Some(family)
                    || covered_by_dict.get(i - 1).copied().unwrap_or(false)
                    || (family != ScriptFamily::CjkIdeograph
                        && uax29_word_end.get(i - 1) != uax29_word_end.get(i));
                if is_script_start && !has_tandem_repeat && !has_sequence {
                    let word_limit = if family == ScriptFamily::CjkIdeograph {
                        n
                    } else {
                        uax29_word_end.get(i).copied().unwrap_or(i).max(i)
                    };
                    let run_len = families
                        .get(i..word_limit)
                        .zip(covered_by_dict.get(i..word_limit))
                        .map_or(0, |(f_tail, c_tail)| {
                            f_tail
                                .iter()
                                .zip(c_tail.iter())
                                .take_while(|&(&f, &cov)| f == family && !cov)
                                .count()
                        });

                    let min_word_len = match family {
                        ScriptFamily::CjkIdeograph | ScriptFamily::Hangul => 2,
                        _ => 3,
                    };

                    if run_len >= min_word_len
                        && let Some(word_slice) = chars.get(i..i + run_len)
                        && Self::has_natural_word_casing(word_slice)
                    {
                        let cost = self.unverified_run_entropy(word_slice, family);
                        Self::relax_dp(
                            &mut dp,
                            i + run_len,
                            current_cost + cost,
                            i,
                            RawSegment {
                                kind: RawSegmentKind::UnverifiedWord(i, i + run_len),
                                entropy_bits: cost,
                            },
                        );
                    }
                }
            }

            // 7. Interleaved / Constant-Separator Steganography Detection:
            // Detects sequences or words where a constant 1..=4 char separator is inserted between
            // every character, e.g.:
            //   - "礐a礑a礒a礓a礔"      (sequence "礐礑礒礓礔" interleaved with "a")
            //   - "h.g.f.e.d.c.b.a"    (sequence "hgfedcba" interleaved with ".")
            //   - "b22a22t22t22e22r22y22" (word "battery" interleaved with "22")
            for sep_len in 1..=MAX_INTERLEAVE_SEP_LEN {
                let stride = sep_len + 1;
                if i + 2 * stride >= n {
                    break;
                }

                let (Some(sep_lower), Some(sep_orig)) =
                    (lower_chars.get(i + 1..i + 1 + sep_len), chars.get(i + 1..i + 1 + sep_len))
                else {
                    break;
                };

                // Skip redundant suffix re-evaluations if `i` is already inside the exact same separator stride
                if i >= stride && lower_chars.get(i - sep_len..i) == Some(sep_lower) {
                    continue;
                }

                core_chars.clear();
                core_lower.clear();
                core_kb_ids.clear();
                core_chars.push(ch_i);
                core_lower.push(lower_i);
                core_kb_ids.push(kb_i);

                let mut k = 1usize;
                while i + (k - 1) * stride + 1 + sep_len <= n {
                    let prev_sep_start = i + (k - 1) * stride + 1;
                    if lower_chars.get(prev_sep_start..prev_sep_start + sep_len) != Some(sep_lower)
                    {
                        break;
                    }
                    let next_core_idx = i + k * stride;
                    if let (Some(&nc), Some(&nl), Some(&nk)) = (
                        chars.get(next_core_idx),
                        lower_chars.get(next_core_idx),
                        kb_ids.get(next_core_idx),
                    ) {
                        core_chars.push(nc);
                        core_lower.push(nl);
                        core_kb_ids.push(nk);
                        k += 1;
                    } else {
                        break;
                    }
                }

                // Require at least 3 interleaved core characters (i.e. >= 2 identical separators)
                let num_core = core_chars.len();
                if num_core < 3 {
                    continue;
                }

                let (Some(&first_core_lower), Some(&first_core_char), Some(&second_core_char)) =
                    (core_lower.first(), core_chars.first(), core_chars.get(1))
                else {
                    continue;
                };

                // If both the core and separator are all the exact same character (e.g. "aaaa..."),
                // it is a simple tandem repeat, not an interleaved word.
                if core_lower.iter().all(|&c| c == first_core_lower)
                    && sep_lower.iter().all(|&c| c == first_core_lower)
                {
                    continue;
                }

                // Span covers either `c0 S c1 S ... c_{m-1}` or `c0 S c1 S ... c_{m-1} S` (trailing separator)
                let last_core_idx = i + (num_core - 1) * stride;
                let has_trailing_sep = lower_chars
                    .get(last_core_idx + 1..last_core_idx + 1 + sep_len)
                    == Some(sep_lower);
                let span_end =
                    if has_trailing_sep { last_core_idx + 1 + sep_len } else { last_core_idx + 1 };

                let sep_bits = Self::separator_entropy(sep_orig) + 1.0; // +1 bit for interleaving pattern

                // 7a. Check if the de-interleaved core characters form a monotonic or keyboard sequence
                let core_delta = (second_core_char as i64) - (first_core_char as i64);
                let is_core_sequence = core_delta.abs() == 1
                    && core_chars
                        .windows(2)
                        .all(|w| matches!(w, [a, b] if (*b as i64) - (*a as i64) == core_delta));
                let is_core_kb = !is_core_sequence
                    && num_core >= 4
                    && Self::keyboard_walk_len_ids(&core_kb_ids) == num_core;

                if is_core_sequence || is_core_kb {
                    let seq_bits = Self::char_entropy(first_core_char)
                        + (num_core as f64).log2()
                        + if is_core_kb { 2.0 } else { 1.0 };
                    let total_bits = seq_bits + sep_bits;
                    let kind = if is_core_kb {
                        RawSegmentKind::KeyboardSequence(i, span_end)
                    } else {
                        RawSegmentKind::Sequence(i, span_end)
                    };
                    Self::relax_dp(
                        &mut dp,
                        span_end,
                        current_cost + total_bits,
                        i,
                        RawSegment { kind, entropy_bits: total_bits },
                    );
                } else {
                    // 7b. Otherwise, check if the de-interleaved core characters form a single-script word or dictionary match
                    let core_family = UnicodePool::script_family(first_core_char);
                    if core_family != ScriptFamily::NonWord
                        && core_chars.iter().all(|&c| UnicodePool::script_family(c) == core_family)
                    {
                        // Check dictionary match on the de-interleaved core string
                        core_lower_str.clear();
                        core_lower_str.extend(core_chars.iter().copied().flat_map(Self::fold_char));
                        let matched_dict = self.is_exact_dict_match(&core_lower_str);

                        if matched_dict {
                            let total_bits = verified_bits + sep_bits;
                            Self::relax_dp(
                                &mut dp,
                                span_end,
                                current_cost + total_bits,
                                i,
                                RawSegment {
                                    kind: RawSegmentKind::DictionaryWord(i, span_end),
                                    entropy_bits: total_bits,
                                },
                            );
                        } else if num_core >= 4 {
                            let word_bits = self.unverified_run_entropy(&core_chars, core_family);
                            let total_bits = word_bits + sep_bits;
                            Self::relax_dp(
                                &mut dp,
                                span_end,
                                current_cost + total_bits,
                                i,
                                RawSegment {
                                    kind: RawSegmentKind::UnverifiedWord(i, span_end),
                                    entropy_bits: total_bits,
                                },
                            );
                        }
                    }
                }
            }
        }

        // Zeroize reusable interleave & UAX #29 buffers
        core_chars.zeroize();
        core_lower.zeroize();
        core_kb_ids.zeroize();
        core_lower_str.zeroize();
        kb_ids.zeroize();
        is_grapheme_continuation.zeroize();
        uax29_word_end.zeroize();

        // Reconstruct minimum-entropy segmentation path and materialize strings only for the winning path
        let mut raw_segments = Vec::new();
        let mut curr = n;
        while curr > 0 {
            let Some(&(_, prev, ref seg)) = dp.get(curr) else {
                break;
            };
            if let Some(s) = seg {
                raw_segments.push(s.into_segment(&chars));
            }
            curr = prev;
        }
        raw_segments.reverse();

        // Zeroize temporary character buffers and DP table
        chars.zeroize();
        lower_chars.zeroize();
        kb_ids.zeroize();
        core_kb_ids.zeroize();
        dp.zeroize();

        let mut segments = Self::merge_literals(raw_segments);
        let mut total_entropy: f64 = segments.iter().map(|s| s.entropy_bits).sum();
        let mut warnings = Vec::new();
        let mut unverified_count = 0usize;

        if truncated && self.emit_warnings {
            warnings.push(format!(
                "Password exceeded {} characters; only the first {} NFC characters were analyzed.",
                self.max_chars, self.max_chars
            ));
        }

        // Inspect segments for bad repeating patterns, sequences, or unverified words
        for seg in &segments {
            match &seg.kind {
                SegmentKind::TandemRepeat { unit, repeats, full_match } => {
                    if self.emit_warnings {
                        warnings.push(format!(
                            "Repeating pattern detected: \"{}\" repeats \"{}\" {} times.",
                            full_match, unit, repeats
                        ));
                    }
                    total_entropy = (total_entropy - 4.0).max(1.0);
                }
                SegmentKind::PreviousRepeat(s) => {
                    if self.emit_warnings {
                        warnings.push(format!(
                            "Repeated combination or word detected: \"{}\" appears multiple times.",
                            s
                        ));
                    }
                    total_entropy = (total_entropy - 3.0).max(1.0);
                }
                SegmentKind::Sequence(s) => {
                    if self.emit_warnings {
                        warnings.push(format!("Sequential characters detected: \"{}\".", s));
                    }
                }
                SegmentKind::KeyboardSequence(s) => {
                    if self.emit_warnings {
                        warnings.push(format!("Keyboard walk detected: \"{}\".", s));
                    }
                }
                SegmentKind::UnverifiedWord(_) => {
                    unverified_count += 1;
                }
                _ => {}
            }
        }

        if unverified_count > 0 && self.emit_warnings {
            warnings.push(format!(
                "{} unverified word(s) not in a large dictionary; scored conservatively (top-{} pool per word).",
                unverified_count, self.unverified_pool_size
            ));
        }

        // If caller disabled segment retention, zeroize and clear segment strings before returning
        if !self.include_segments {
            segments.zeroize();
            segments.clear();
        }

        let rating = match total_entropy {
            e if e < 36.0 => StrengthRating::VeryWeak,
            e if e < 60.0 => StrengthRating::Weak,
            e if e < 80.0 => StrengthRating::Reasonable,
            e if e < 128.0 => StrengthRating::Strong,
            _ => StrengthRating::Cryptographic,
        };

        PasswordAnalysis {
            normalized_length: n,
            truncated,
            entropy_bits: total_entropy,
            rating,
            segments,
            warnings,
        }
    }

    /// Merges consecutive alphabetic Literal segments of the same script family and coalesces
    /// multi-scalar extended grapheme clusters (such as combined ZWJ emojis, flags, or skin-tone
    /// modifiers), while keeping standalone NonWord delimiters (like '/', '-', ' ') separate.
    fn merge_literals(segments: Vec<Segment>) -> Vec<Segment> {
        let mut merged: Vec<Segment> = Vec::new();
        for mut seg in segments {
            if let SegmentKind::Literal(ref ch_str) = seg.kind
                && let Some(ch) = ch_str.chars().next()
                && let Some(last) = merged.last_mut()
                && let SegmentKind::Literal(ref mut prev_str) = last.kind
                && let Some(prev_ch) = prev_str.chars().next()
            {
                let family = UnicodePool::script_family(ch);
                let same_alpha_script = family != ScriptFamily::NonWord
                    && UnicodePool::script_family(prev_ch) == family;

                if same_alpha_script {
                    prev_str.push_str(ch_str);
                    last.entropy_bits += seg.entropy_bits;
                    seg.zeroize();
                    continue;
                } else if !ch.is_ascii() {
                    let old_len = prev_str.len();
                    let prev_graphemes = prev_str.graphemes(true).count();
                    prev_str.push_str(ch_str);
                    if prev_str.graphemes(true).count() == prev_graphemes {
                        last.entropy_bits += seg.entropy_bits;
                        seg.zeroize();
                        continue;
                    }
                    prev_str.truncate(old_len);
                }
            }
            merged.push(seg);
        }
        merged
    }
}

#[cfg(test)]
mod tests;
