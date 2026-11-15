//! The smart playlist editor's logic, shared by both apps.
//! Rules are read from and written to Navidrome's JSON, described in words and previewed over the local library.
//! Text goes through [`Words`], which each app implements over the shared catalog.

pub mod fields;
pub mod preview;
pub mod rules;
pub mod summary;

pub use fields::*;
pub use preview::*;
pub use rules::*;
pub use summary::*;

/// Marks `key` as a catalog key in a table, to be translated later.
pub const fn n_(key: &'static str) -> &'static str {
    key
}

/// The app's translations, for text the core composes.
pub trait Words: Send + Sync {
    /// The active language's text for `key`, or the English key when there's none.
    fn text(&self, key: &str) -> String;
    /// The active language's plural form for `n`, with `{count}` still in it. `other` is the key.
    fn plural(&self, one: &str, other: &str, n: u64) -> String;
    /// A whole number with the language's grouping.
    fn int(&self, n: i64) -> String;
    /// A playlist's name, when the app knows it.
    fn playlist_name(&self, _id: &str) -> Option<String> {
        None
    }
}

/// English with no catalog, returning the keys themselves. For tests and callers with no language.
pub struct English;

impl Words for English {
    fn text(&self, key: &str) -> String {
        key.split("||").next().unwrap_or(key).to_string()
    }
    fn plural(&self, one: &str, other: &str, n: u64) -> String {
        if n == 1 { one.to_string() } else { other.to_string() }
    }
    fn int(&self, n: i64) -> String {
        n.to_string()
    }
}

/// Fills `{name}` placeholders.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..open + close];
        match vars.iter().find(|(k, _)| *k == name) {
            Some((_, v)) => out.push_str(v),
            None => out.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

/// [`Words`] with the helpers the text here is written with.
#[derive(Clone, Copy)]
pub struct Say<'a>(pub &'a dyn Words);

impl Say<'_> {
    pub fn tr(&self, key: &str) -> String {
        self.0.text(key)
    }
    pub fn trf(&self, key: &str, vars: &[(&str, &str)]) -> String {
        fill(&self.0.text(key), vars)
    }
    pub fn trn(&self, one: &str, other: &str, n: u64) -> String {
        self.trnf(one, other, n, &[])
    }
    pub fn trnf(&self, one: &str, other: &str, n: u64, vars: &[(&str, &str)]) -> String {
        let count = self.0.int(n as i64);
        let mut all = vec![("count", count.as_str())];
        all.extend_from_slice(vars);
        fill(&self.0.plural(one, other, n), &all)
    }
    pub fn playlist(&self, id: &str) -> String {
        self.0.playlist_name(id).unwrap_or_else(|| self.tr("another playlist"))
    }
}
