//! Pure, deterministic M1 Telex state-machine foundation.
//!
//! This crate produces abstract edits only. It contains no input execution,
//! operating-system API, recovery classifier, global state, clock, or locale use.

use zonkey_token::TokenState;
use zonkey_types::{EngineAction, EngineEvent, TokenBoundary, UnsupportedBehavior};

pub use zonkey_types::EditPlan;

/// Stateful engine owned by its caller; no global state is used.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelexEngine {
    token: TokenState,
}

impl TelexEngine {
    /// Creates an empty engine.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns current raw and rendered token state.
    #[must_use]
    pub const fn token(&self) -> &TokenState {
        &self.token
    }

    /// Processes one platform-neutral event.
    ///
    /// # Errors
    ///
    /// Returns [`UnsupportedBehavior::NonAsciiInput`] for direct non-ASCII
    /// character input. Telex-generated Unicode remains supported as output.
    pub fn process(&mut self, event: EngineEvent) -> Result<EngineAction, UnsupportedBehavior> {
        match event {
            EngineEvent::Character(character) => self.process_character(character),
            EngineEvent::Backspace => Ok(self.process_backspace()),
            EngineEvent::Boundary(boundary) => Ok(self.process_boundary(boundary)),
        }
    }

    fn process_character(&mut self, character: char) -> Result<EngineAction, UnsupportedBehavior> {
        if let Some(boundary) = TokenBoundary::from_char(character) {
            return Ok(self.process_boundary(boundary));
        }
        if !character.is_ascii() {
            return Err(UnsupportedBehavior::NonAsciiInput(character));
        }

        self.token
            .push_raw(character)
            .map_err(UnsupportedBehavior::NonAsciiInput)?;
        let rendered = compose(self.token.raw_ascii.as_str())?;
        Ok(EngineAction::Apply(self.token.replace_rendered(rendered)))
    }

    fn process_backspace(&mut self) -> EngineAction {
        if self.token.pop_raw().is_none() {
            return EngineAction::Noop;
        }
        let rendered = compose(self.token.raw_ascii.as_str())
            .expect("raw token is guaranteed to contain ASCII only");
        EngineAction::Apply(self.token.replace_rendered(rendered))
    }

    fn process_boundary(&mut self, boundary: TokenBoundary) -> EngineAction {
        match boundary {
            TokenBoundary::Space
            | TokenBoundary::Enter
            | TokenBoundary::Tab
            | TokenBoundary::Punctuation(_) => {
                self.token.clear();
                EngineAction::Commit(boundary)
            }
            TokenBoundary::CursorMove | TokenBoundary::FocusLoss | TokenBoundary::Unknown => {
                self.token.clear();
                EngineAction::Reset
            }
        }
    }
}

/// Composes a raw ASCII sequence using the intentionally limited M1 rule set.
///
/// # Errors
///
/// Returns an unsupported result if `raw` contains non-ASCII characters.
pub fn compose(raw: &str) -> Result<String, UnsupportedBehavior> {
    let mut output = Vec::new();
    for character in raw.chars() {
        if !character.is_ascii() {
            return Err(UnsupportedBehavior::NonAsciiInput(character));
        }
        if try_shape_transform(&mut output, character) || try_tone(&mut output, character) {
            continue;
        }
        output.push(character);
    }
    Ok(output.into_iter().collect())
}

fn try_shape_transform(output: &mut Vec<char>, key: char) -> bool {
    let Some(&last) = output.last() else {
        return false;
    };

    if key == 'd' {
        if last == 'd' {
            *output.last_mut().expect("last was checked") = 'đ';
            return true;
        }
        if last == 'đ' {
            *output.last_mut().expect("last was checked") = 'd';
            output.push('d');
            return true;
        }
    }

    let Some((shape, tone)) = decode_vowel(last) else {
        return false;
    };
    let transition = match (key, shape) {
        ('a', VowelShape::A) => ShapeTransition::Replace(VowelShape::Acirc),
        ('a', VowelShape::Acirc) | ('w', VowelShape::Abreve) => {
            ShapeTransition::Undo(VowelShape::A)
        }
        ('e', VowelShape::E) => ShapeTransition::Replace(VowelShape::Ecirc),
        ('e', VowelShape::Ecirc) => ShapeTransition::Undo(VowelShape::E),
        ('o', VowelShape::O) => ShapeTransition::Replace(VowelShape::Ocirc),
        ('o', VowelShape::Ocirc) | ('w', VowelShape::Ohorn) => ShapeTransition::Undo(VowelShape::O),
        ('w', VowelShape::A) => ShapeTransition::Replace(VowelShape::Abreve),
        ('w', VowelShape::O) => ShapeTransition::Replace(VowelShape::Ohorn),
        ('w', VowelShape::U) => ShapeTransition::Replace(VowelShape::Uhorn),
        ('w', VowelShape::Uhorn) => ShapeTransition::Undo(VowelShape::U),
        _ => return false,
    };

    match transition {
        ShapeTransition::Replace(next) => {
            *output.last_mut().expect("last was checked") = encode_vowel(next, tone);
        }
        ShapeTransition::Undo(next) => {
            *output.last_mut().expect("last was checked") = encode_vowel(next, tone);
            output.push(key);
        }
    }
    true
}

fn try_tone(output: &mut [char], key: char) -> bool {
    let requested = match key {
        'f' => Tone::Grave,
        's' => Tone::Acute,
        'r' => Tone::Hook,
        'x' => Tone::Tilde,
        'j' => Tone::Dot,
        _ => return false,
    };
    let Some(index) = output
        .iter()
        .rposition(|character| decode_vowel(*character).is_some())
    else {
        return false;
    };
    let (shape, current) = decode_vowel(output[index]).expect("position is a vowel");
    if current == requested {
        output[index] = encode_vowel(shape, Tone::None);
        return false;
    }
    output[index] = encode_vowel(shape, requested);
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShapeTransition {
    Replace(VowelShape),
    Undo(VowelShape),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    None,
    Grave,
    Acute,
    Hook,
    Tilde,
    Dot,
}

impl Tone {
    const fn index(self) -> usize {
        match self {
            Self::None => 0,
            Self::Grave => 1,
            Self::Acute => 2,
            Self::Hook => 3,
            Self::Tilde => 4,
            Self::Dot => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VowelShape {
    A,
    Abreve,
    Acirc,
    E,
    Ecirc,
    I,
    O,
    Ocirc,
    Ohorn,
    U,
    Uhorn,
    Y,
}

const VOWELS: [[char; 6]; 12] = [
    ['a', 'à', 'á', 'ả', 'ã', 'ạ'],
    ['ă', 'ằ', 'ắ', 'ẳ', 'ẵ', 'ặ'],
    ['â', 'ầ', 'ấ', 'ẩ', 'ẫ', 'ậ'],
    ['e', 'è', 'é', 'ẻ', 'ẽ', 'ẹ'],
    ['ê', 'ề', 'ế', 'ể', 'ễ', 'ệ'],
    ['i', 'ì', 'í', 'ỉ', 'ĩ', 'ị'],
    ['o', 'ò', 'ó', 'ỏ', 'õ', 'ọ'],
    ['ô', 'ồ', 'ố', 'ổ', 'ỗ', 'ộ'],
    ['ơ', 'ờ', 'ớ', 'ở', 'ỡ', 'ợ'],
    ['u', 'ù', 'ú', 'ủ', 'ũ', 'ụ'],
    ['ư', 'ừ', 'ứ', 'ử', 'ữ', 'ự'],
    ['y', 'ỳ', 'ý', 'ỷ', 'ỹ', 'ỵ'],
];

const SHAPES: [VowelShape; 12] = [
    VowelShape::A,
    VowelShape::Abreve,
    VowelShape::Acirc,
    VowelShape::E,
    VowelShape::Ecirc,
    VowelShape::I,
    VowelShape::O,
    VowelShape::Ocirc,
    VowelShape::Ohorn,
    VowelShape::U,
    VowelShape::Uhorn,
    VowelShape::Y,
];

fn decode_vowel(character: char) -> Option<(VowelShape, Tone)> {
    for (shape_index, row) in VOWELS.iter().enumerate() {
        if let Some(tone_index) = row.iter().position(|candidate| *candidate == character) {
            let tone = match tone_index {
                0 => Tone::None,
                1 => Tone::Grave,
                2 => Tone::Acute,
                3 => Tone::Hook,
                4 => Tone::Tilde,
                5 => Tone::Dot,
                _ => unreachable!("vowel table has exactly six columns"),
            };
            return Some((SHAPES[shape_index], tone));
        }
    }
    None
}

fn encode_vowel(shape: VowelShape, tone: Tone) -> char {
    let shape_index = SHAPES
        .iter()
        .position(|candidate| *candidate == shape)
        .expect("every vowel shape has a table row");
    VOWELS[shape_index][tone.index()]
}

#[cfg(test)]
mod tests {
    use super::{TelexEngine, compose};
    use zonkey_types::{EngineAction, EngineEvent, TokenBoundary, UnsupportedBehavior};

    #[test]
    fn composes_standard_m1_shapes() {
        for (raw, expected) in [
            ("dd", "đ"),
            ("aa", "â"),
            ("aw", "ă"),
            ("ee", "ê"),
            ("oo", "ô"),
            ("ow", "ơ"),
            ("uw", "ư"),
        ] {
            assert_eq!(compose(raw), Ok(String::from(expected)), "raw={raw}");
        }
    }

    #[test]
    fn composes_all_five_tones() {
        for (raw, expected) in [
            ("af", "à"),
            ("as", "á"),
            ("ar", "ả"),
            ("ax", "ã"),
            ("aj", "ạ"),
        ] {
            assert_eq!(compose(raw), Ok(String::from(expected)), "raw={raw}");
        }
    }

    #[test]
    fn repeated_transform_keys_have_explicit_undo() {
        assert_eq!(compose("aaa"), Ok(String::from("aa")));
        assert_eq!(compose("aww"), Ok(String::from("aw")));
        assert_eq!(compose("ass"), Ok(String::from("as")));
        assert_eq!(compose("ddd"), Ok(String::from("dd")));
    }

    #[test]
    fn ordinary_ascii_without_telex_sequences_stays_raw() {
        assert_eq!(compose("hello"), Ok(String::from("hello")));
        assert_eq!(compose("banana"), Ok(String::from("banana")));
        assert_eq!(compose("token"), Ok(String::from("token")));
    }

    #[test]
    fn engine_preserves_raw_separately_from_rendered() {
        let mut engine = TelexEngine::new();
        engine
            .process(EngineEvent::Character('d'))
            .expect("ASCII is supported");
        engine
            .process(EngineEvent::Character('d'))
            .expect("ASCII is supported");
        assert_eq!(engine.token().raw_ascii, "dd");
        assert_eq!(engine.token().rendered, "đ");
    }

    #[test]
    fn backspace_recomposes_from_raw_state() {
        let mut engine = TelexEngine::new();
        for character in "aws".chars() {
            engine
                .process(EngineEvent::Character(character))
                .expect("ASCII is supported");
        }
        assert_eq!(engine.token().rendered, "ắ");
        let action = engine
            .process(EngineEvent::Backspace)
            .expect("backspace is supported");
        assert!(matches!(action, EngineAction::Apply(_)));
        assert_eq!(engine.token().raw_ascii, "aw");
        assert_eq!(engine.token().rendered, "ă");
    }

    #[test]
    fn commit_and_reset_boundaries_are_distinct() {
        let mut engine = TelexEngine::new();
        engine
            .process(EngineEvent::Character('a'))
            .expect("ASCII is supported");
        assert_eq!(
            engine.process(EngineEvent::Boundary(TokenBoundary::Space)),
            Ok(EngineAction::Commit(TokenBoundary::Space))
        );
        assert!(engine.token().is_empty());
        assert_eq!(
            engine.process(EngineEvent::Boundary(TokenBoundary::CursorMove)),
            Ok(EngineAction::Reset)
        );
    }

    #[test]
    fn non_ascii_direct_input_is_typed_unsupported() {
        let mut engine = TelexEngine::new();
        assert_eq!(
            engine.process(EngineEvent::Character('đ')),
            Err(UnsupportedBehavior::NonAsciiInput('đ'))
        );
    }
}
