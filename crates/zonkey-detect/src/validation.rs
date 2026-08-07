/// Conservatively recognizes precomposed Vietnamese letters and tone marks.
#[must_use]
pub fn is_vietnamese_candidate(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character,
            '\u{00c0}'..='\u{00c3}' | '\u{00c8}'..='\u{00ca}' | '\u{00cc}'..='\u{00cd}' |
            '\u{00d2}'..='\u{00d5}' | '\u{00d9}'..='\u{00da}' | '\u{00dd}' |
            '\u{00e0}'..='\u{00e3}' | '\u{00e8}'..='\u{00ea}' | '\u{00ec}'..='\u{00ed}' |
            '\u{00f2}'..='\u{00f5}' | '\u{00f9}'..='\u{00fa}' | '\u{00fd}' |
            '\u{0102}'..='\u{0103}' | '\u{0110}'..='\u{0111}' | '\u{0128}'..='\u{0129}' |
            '\u{0168}'..='\u{0169}' | '\u{01a0}'..='\u{01b0}' | '\u{1ea0}'..='\u{1ef9}'
        )
    })
}
