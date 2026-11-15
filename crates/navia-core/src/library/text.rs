//! Search text normalization, shared by indexing and query parsing.
//! Folds case and diacritics, unifies some Arabic letter variants and splits CJK into single
//! characters, so a query matches however a tag was typed.

use unicode_normalization::UnicodeNormalization;

/// Combining marks we remove. Kana voicing marks and Indic vowel signs stay because they change
/// the word.
fn is_foldable_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 |
        0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7 |
        0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 |
        0x06E7..=0x06E8 | 0x06EA..=0x06ED |
        0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// Ideographs and kana: each becomes its own search token.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF |
        0xFF66..=0xFF9F | 0x20000..=0x2FFFF | 0x30000..=0x3134F)
}

fn fold_letter(c: char) -> char {
    match c {
        // Arabic hamza-carrying alefs and alef maksura, typed interchangeably.
        'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
        'ى' => 'ي',
        'ة' => 'ه',
        // Final sigma.
        'ς' => 'σ',
        other => other,
    }
}

/// Normalized, space-separated form used for the index and for queries.
pub fn normalize(text: &str) -> String {
    if text.is_ascii() {
        // Nothing to decompose or fold: the same result, several times faster.
        return text.to_ascii_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    }
    let folded: String = text.nfkd().filter(|c| !is_foldable_mark(*c)).nfc().collect();
    let mut out = String::with_capacity(folded.len() + 8);
    let mut prev_cjk = false;
    for c in folded.chars().flat_map(char::to_lowercase).map(fold_letter) {
        let cjk = is_cjk(c);
        if cjk && !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        if prev_cjk && !cjk && !c.is_whitespace() {
            out.push(' ');
        }
        out.push(if c.is_whitespace() { ' ' } else { c });
        prev_cjk = cjk;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Characters dropped inside a word rather than splitting it, so "Don't" is "dont" and
/// "R.E.M." is "rem".
fn is_joiner(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '\u{02BC}' | '`' | '\u{00B4}' | '.')
}

/// The words of a text for the fuzzy search index. "&" reads as "and", and each CJK character is
/// a word of its own.
pub fn search_words(text: &str) -> Vec<String> {
    let normalized = normalize(text);
    let mut words = Vec::new();
    let mut word = String::new();
    for c in normalized.chars() {
        if c.is_alphanumeric() || unicode_normalization::char::is_combining_mark(c) {
            if is_cjk(c) {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                words.push(c.to_string());
            } else {
                word.push(c);
            }
        } else if is_joiner(c) && !word.is_empty() {
            continue;
        } else {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            if c == '&' {
                words.push("and".to_string());
            }
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Whether a search word is a single CJK character.
pub fn is_cjk_word(word: &str) -> bool {
    let mut chars = word.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if is_cjk(c))
}

/// Build an FTS5 MATCH expression from user input, or `None` if nothing is searchable.
/// Terms are quoted prefix matches, so input can't reach FTS5 syntax. CJK runs become phrases.
pub fn fts_query(input: &str) -> Option<String> {
    const MAX_TERMS: usize = 16;
    let normalized = normalize(input);
    let tokens: Vec<&str> = normalized.split(' ').filter(|t| t.chars().any(char::is_alphanumeric)).collect();
    if tokens.is_empty() {
        return None;
    }
    let mut terms: Vec<String> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    for token in tokens {
        if token.chars().count() == 1 && token.chars().all(is_cjk) {
            run.push(token);
            continue;
        }
        if !run.is_empty() {
            terms.push(format!("\"{}\"*", run.join(" ")));
            run.clear();
        }
        terms.push(format!("\"{}\"*", token.replace('"', "\"\"")));
    }
    if !run.is_empty() {
        terms.push(format!("\"{}\"*", run.join(" ")));
    }
    terms.truncate(MAX_TERMS);
    Some(terms.join(" AND "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_diacritics() {
        assert_eq!(normalize("Beyoncé  KNOWLES"), "beyonce knowles");
        assert_eq!(normalize("Motörhead"), "motorhead");
    }

    #[test]
    fn strips_arabic_points_and_folds_alef() {
        assert_eq!(normalize("مُحَمَّد"), "محمد");
        assert_eq!(normalize("أغنية"), normalize("اغنيه"));
    }

    #[test]
    fn spaces_out_cjk_but_keeps_latin_words() {
        assert_eq!(normalize("堕天使 love"), "堕 天 使 love");
        assert_eq!(normalize("King Gnu 白日"), "king gnu 白 日");
    }

    #[test]
    fn keeps_kana_voicing_and_hangul_syllables() {
        assert_eq!(normalize("ガ"), "ガ");
        assert_ne!(normalize("ガ"), normalize("カ"));
        assert_eq!(normalize("방탄"), "방탄");
    }

    #[test]
    fn search_words_fold_punctuation_and_symbols() {
        assert_eq!(search_words("AC/DC"), ["ac", "dc"]);
        assert_eq!(search_words("Don\u{2019}t Stop"), ["dont", "stop"]);
        assert_eq!(search_words("R.E.M."), ["rem"]);
        assert_eq!(search_words("Simon & Garfunkel"), ["simon", "and", "garfunkel"]);
        assert_eq!(search_words("Song (feat. Drake)"), ["song", "feat", "drake"]);
        assert_eq!(search_words("ＢＥＹＯＮＣÉ"), ["beyonce"]);
        assert_eq!(search_words("堕天使 love"), ["堕", "天", "使", "love"]);
        assert_eq!(search_words("방탄소년단"), ["방탄소년단"]);
        assert_eq!(search_words("हिन्दी"), ["हिन्दी"], "vowel signs stay in the word");
    }

    #[test]
    fn query_uses_prefix_terms_and_cjk_phrases() {
        assert_eq!(fts_query("beat abb").as_deref(), Some("\"beat\"* AND \"abb\"*"));
        assert_eq!(fts_query("天使").as_deref(), Some("\"天 使\"*"));
        assert_eq!(fts_query("天使 love").as_deref(), Some("\"天 使\"* AND \"love\"*"));
    }

    #[test]
    fn query_quotes_fts_syntax_and_drops_punctuation_only_terms() {
        assert_eq!(fts_query("a\"b").as_deref(), Some("\"a\"\"b\"*"));
        assert_eq!(fts_query("ac/dc NEAR(x)").as_deref(), Some("\"ac/dc\"* AND \"near(x)\"*"));
        assert_eq!(fts_query("& - !"), None);
        assert_eq!(fts_query("   "), None);
    }
}
