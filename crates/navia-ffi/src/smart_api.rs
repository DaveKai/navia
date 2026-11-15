//! The smart playlist editor for Swift, wrapping the core's rules, summary and preview (`navia_core::smart_editor`).
//! Editor state lives here. Swift draws [`SmartRowView`]s in order and sends edits by row key. Text the core composes
//! is translated through Swift's catalog ([`SmartWords`]).

use std::sync::{Arc, Mutex};

use navia_core::smart_editor::{self as smart, EditorState, Evaluator, LimitUnit, PlaylistMembers, PreviewSong, Row, RuleValue, Say, Source, Words};

use crate::core_api::NaviaCore;

/// Swift's translations and playlist names, for text the core writes.
#[uniffi::export(callback_interface)]
pub trait SmartWords: Send + Sync {
    /// The active language's text for a catalog key.
    fn text(&self, key: String) -> String;
    /// The language's plural form of a counted text for `n`, with `{count}` still in it. `other` is the key.
    fn plural(&self, one: String, other: String, n: u64) -> String;
    /// A whole number with the language's grouping. Not named `int` because that's a C keyword in the generated header.
    fn number(&self, n: i64) -> String;
    /// A playlist's name, when the app knows it.
    fn playlist_name(&self, id: String) -> Option<String>;
}

struct Bridge(Box<dyn SmartWords>);

impl Words for Bridge {
    fn text(&self, key: &str) -> String {
        self.0.text(key.to_string())
    }
    fn plural(&self, one: &str, other: &str, n: u64) -> String {
        self.0.plural(one.to_string(), other.to_string(), n)
    }
    fn int(&self, n: i64) -> String {
        self.0.number(n)
    }
    fn playlist_name(&self, id: &str) -> Option<String> {
        self.0.playlist_name(id.to_string())
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartRowKind {
    Rule,
    /// A rule the editor can't show as fields, kept as written. `reason` is a catalog key.
    Kept,
    /// A group. `any` is its All/Any, and its rows follow one level deeper. `empty` when it has none.
    Group,
}

/// The kind of value a rule takes, which decides the control.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartValueShape {
    None,
    Text,
    Number,
    Year,
    Days,
    Date,
    Bool,
    Genre,
    Playlist,
    RangeNumber,
    RangeYear,
    RangeDate,
}

impl SmartValueShape {
    fn of(s: smart::Shape) -> Self {
        use smart::Shape as S;
        match s {
            S::None => Self::None,
            S::Text => Self::Text,
            S::Number => Self::Number,
            S::Year => Self::Year,
            S::Days => Self::Days,
            S::Date => Self::Date,
            S::Bool => Self::Bool,
            S::Genre => Self::Genre,
            S::Playlist => Self::Playlist,
            S::RangeNumber => Self::RangeNumber,
            S::RangeYear => Self::RangeYear,
            S::RangeDate => Self::RangeDate,
        }
    }
}

/// One row of the editor in display order.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmartRowView {
    pub key: u64,
    /// 0 for the top level's rows, one more per group.
    pub depth: u32,
    pub kind: SmartRowKind,
    pub field: String,
    pub op: String,
    pub shape: SmartValueShape,
    /// The value, or a range's first value.
    pub value: String,
    /// A range's second value.
    pub value_to: String,
    pub flag: bool,
    /// The value is filled in but malformed.
    pub invalid: bool,
    /// Group only.
    pub any: bool,
    pub empty: bool,
    /// Whether another group fits inside this one.
    pub can_nest: bool,
    /// Kept only.
    pub text: String,
    pub reason: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SmartSortView {
    pub key: u64,
    pub field: String,
    pub desc: bool,
}

/// A field from Navidrome's vocabulary. `label`, `group`, `unit` and `hint` are catalog keys.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmartFieldRow {
    pub id: String,
    pub label: String,
    pub group: String,
    pub unit: Option<String>,
    pub hint: Option<String>,
    /// Playlist membership can't be sorted by.
    pub sortable: bool,
}

/// A sort direction in words for a field's type, as a catalog key.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmartDirection {
    pub desc: bool,
    pub label: String,
}

#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct SmartPreviewResult {
    pub rowids: Vec<u32>,
    pub matched: u32,
    pub sampled: u32,
    /// Field labels or kept rules' text the preview couldn't check. Labels are catalog keys.
    pub unchecked: Vec<String>,
    pub checkable: bool,
    /// Sort fields the preview couldn't apply, as field labels.
    pub sort_unchecked: Vec<String>,
}

impl From<smart::Preview> for SmartPreviewResult {
    fn from(p: smart::Preview) -> Self {
        Self { rowids: p.rowids, matched: p.matched as u32, sampled: p.sampled as u32, unchecked: p.unchecked, checkable: p.checkable, sort_unchecked: p.sort_unchecked }
    }
}

/// A song from an app's built-in sample data, for previews without a server.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmartSampleSong {
    pub rowid: u32,
    pub id: String,
    pub title: String,
    pub album: String,
    pub artist: String,
    pub genre: String,
    pub year: Option<u32>,
    pub plays: u32,
    pub loved: bool,
    pub duration_seconds: u32,
}

/// A saved playlist's rules in words, for the playlist page and Get Info.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SmartSummaryRow {
    pub sentence: String,
    pub match_word: String,
    pub rules: Vec<String>,
    pub order: Vec<String>,
}

/// Every field the editor offers.
#[uniffi::export]
pub fn smart_fields() -> Vec<SmartFieldRow> {
    smart::FIELDS
        .iter()
        .map(|f| SmartFieldRow { id: f.id.into(), label: f.label.into(), group: f.group.into(), unit: f.unit.map(Into::into), hint: f.hint.map(Into::into), sortable: f.kind != smart::FieldKind::Playlist })
        .collect()
}

/// The operators Navidrome supports for a field, in Navidrome's spelling.
#[uniffi::export]
pub fn smart_ops(field: String) -> Vec<String> {
    smart::field_by_id(&field).map(|f| smart::ops_for(f).into_iter().map(Into::into).collect()).unwrap_or_default()
}

/// A condition's label, as a catalog key.
#[uniffi::export]
pub fn smart_op_label(op: String) -> String {
    smart::op_label(&op).to_string()
}

/// The kind of value a field and condition take.
#[uniffi::export]
pub fn smart_value_shape(field: String, op: String) -> SmartValueShape {
    SmartValueShape::of(smart::field_by_id(&field).map_or(smart::Shape::Text, |f| smart::shape_of(f, &op)))
}

/// The value field's placeholder for a field and condition, as a catalog key.
#[uniffi::export]
pub fn smart_value_placeholder(field: String, op: String) -> String {
    smart::field_by_id(&field).map_or(smart::Shape::Text, |f| smart::shape_of(f, &op)).placeholder().to_string()
}

/// A field's two sort directions in words, the default first.
#[uniffi::export]
pub fn smart_directions(field: String) -> Vec<SmartDirection> {
    smart::directions(&field).iter().map(|d| SmartDirection { desc: d.desc, label: d.label.into() }).collect()
}

/// A saved playlist's rules (Navidrome's JSON) in words, or None when they can't be read.
#[uniffi::export]
pub fn smart_summary(criteria: String, words: Box<dyn SmartWords>) -> Option<SmartSummaryRow> {
    let bridge = Bridge(words);
    smart::summarize_criteria(&criteria, Say(&bridge)).map(|s| SmartSummaryRow { sentence: s.sentence, match_word: s.match_word, rules: s.rules, order: s.order })
}

/// The smart playlist editor's rules, sort and limit.
#[derive(uniffi::Object)]
pub struct NaviaSmartEditor {
    state: Mutex<EditorState>,
    words: Bridge,
}

impl NaviaSmartEditor {
    fn edit<T>(&self, f: impl FnOnce(&mut EditorState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }
}

#[uniffi::export]
impl NaviaSmartEditor {
    /// An editor for saved rules (Navidrome's JSON), or a blank one.
    #[uniffi::constructor]
    pub fn new(criteria: Option<String>, words: Box<dyn SmartWords>) -> Arc<Self> {
        let words = Bridge(words);
        let state = smart::parse_criteria(criteria.as_deref(), Say(&words));
        Arc::new(Self { state: Mutex::new(state), words })
    }

    /// Replaces the rules with saved ones.
    pub fn load(&self, criteria: Option<String>) {
        let state = smart::parse_criteria(criteria.as_deref(), Say(&self.words));
        *self.state.lock().unwrap() = state;
    }

    pub fn rows(&self) -> Vec<SmartRowView> {
        let st = self.state.lock().unwrap();
        smart::flatten(&st.rows)
            .into_iter()
            .map(|f| {
                let depth = f.depth as u32;
                let mut v = SmartRowView { key: f.row.key(), depth, kind: SmartRowKind::Rule, field: String::new(), op: String::new(), shape: SmartValueShape::None, value: String::new(), value_to: String::new(), flag: false, invalid: false, any: false, empty: false, can_nest: false, text: String::new(), reason: String::new() };
                match f.row {
                    Row::Simple(r) => {
                        v.field = r.field.clone();
                        v.op = r.op.clone();
                        v.shape = SmartValueShape::of(smart::field_by_id(&r.field).map_or(smart::Shape::Text, |field| smart::shape_of(field, &r.op)));
                        match &r.value {
                            RuleValue::One(a) => v.value = a.clone(),
                            RuleValue::Range(a, b) => (v.value, v.value_to) = (a.clone(), b.clone()),
                            RuleValue::Flag(b) => v.flag = *b,
                        }
                        v.invalid = smart::rule_issue(r) == smart::Issue::Invalid;
                    }
                    Row::Kept { text, reason, .. } => {
                        v.kind = SmartRowKind::Kept;
                        v.text = text.clone();
                        v.reason = reason.to_string();
                    }
                    Row::Group { any, rows, .. } => {
                        v.kind = SmartRowKind::Group;
                        v.any = *any;
                        v.empty = rows.is_empty();
                        // Top-level groups are depth 1 of `MAX_GROUP_DEPTH`, and a nested one sits one deeper.
                        v.can_nest = (f.depth as usize) + 2 < smart::MAX_GROUP_DEPTH;
                    }
                }
                v
            })
            .collect()
    }

    pub fn match_any(&self) -> bool {
        self.state.lock().unwrap().match_any
    }

    /// Sets a group's All/Any. None is the top level.
    pub fn set_any(&self, group: Option<u64>, any: bool) {
        self.edit(|st| st.set_any(group, any));
    }

    pub fn add_rule(&self, group: Option<u64>) -> Option<u64> {
        self.edit(|st| st.add_rule(group))
    }

    pub fn add_group(&self, group: Option<u64>) -> Option<u64> {
        self.edit(|st| st.add_group(group))
    }

    pub fn remove(&self, key: u64) {
        self.edit(|st| st.remove(key));
    }

    pub fn set_field(&self, key: u64, field: String) {
        self.edit(|st| st.set_field(key, &field));
    }

    pub fn set_op(&self, key: u64, op: String) {
        self.edit(|st| st.set_op(key, &op));
    }

    /// Types into a rule's value, keeping only what it can hold. `side` 1 is a range's second value.
    pub fn set_value(&self, key: u64, side: u8, text: String) {
        self.edit(|st| st.set_value_text(key, side, &text));
    }

    pub fn set_flag(&self, key: u64, on: bool) {
        self.edit(|st| st.set_flag(key, on));
    }

    /// Picks a value from a list, like a genre or a playlist.
    pub fn set_choice(&self, key: u64, value: String) {
        self.edit(|st| st.set_choice(key, &value));
    }

    pub fn sorts(&self) -> Vec<SmartSortView> {
        self.state.lock().unwrap().sorts.iter().map(|s| SmartSortView { key: s.key, field: s.field.clone(), desc: s.desc }).collect()
    }

    pub fn set_sort_field(&self, index: u32, field: String) {
        self.edit(|st| st.set_sort_field(index as usize, &field));
    }

    pub fn set_sort_desc(&self, index: u32, desc: bool) {
        self.edit(|st| st.set_sort_desc(index as usize, desc));
    }

    /// Adds a sort row at `index`, or the end when past it, with `field` or the next unused common one.
    pub fn add_sort(&self, index: u32, field: Option<String>) {
        self.edit(|st| st.add_sort(index as usize, field.as_deref()));
    }

    pub fn remove_sort(&self, index: u32) {
        self.edit(|st| st.remove_sort(index as usize));
    }

    /// The sort row that should have Album after it, since sorting by date added mixes albums added the same day.
    pub fn album_hint_at(&self) -> Option<u32> {
        smart::album_hint_at(&self.state.lock().unwrap().sorts).map(|i| i as u32)
    }

    pub fn limit(&self) -> String {
        self.state.lock().unwrap().limit.clone()
    }

    pub fn limit_is_percent(&self) -> bool {
        self.state.lock().unwrap().unit == LimitUnit::Percent
    }

    pub fn set_limit(&self, text: String) {
        self.edit(|st| st.set_limit(&text));
    }

    pub fn set_limit_percent(&self, percent: bool) {
        self.edit(|st| st.set_unit(if percent { LimitUnit::Percent } else { LimitUnit::Songs }));
    }

    /// Navidrome's JSON for these rules as Save sends them, without empty groups.
    pub fn criteria(&self) -> String {
        smart::to_criteria(&self.state.lock().unwrap()).to_string()
    }

    /// The rules that are ready, as one line. Empty with no rules.
    pub fn sentence(&self) -> String {
        smart::editor_sentence(&self.state.lock().unwrap(), Say(&self.words))
    }

    /// Whether any rule is left once empty groups are ignored.
    pub fn has_rules(&self) -> bool {
        smart::has_rules(&self.state.lock().unwrap().rows)
    }

    /// Whether every rule is ready to save.
    pub fn rows_complete(&self) -> bool {
        smart::rows_complete(&self.state.lock().unwrap().rows)
    }

    /// Runs the preview over the local library. Blocks, so call it off the main thread.
    pub fn preview_library(&self, core: Arc<NaviaCore>, max: u32) -> SmartPreviewResult {
        let st = self.state.lock().unwrap().clone();
        smart::preview_library(&core.db, &st, max as usize).into()
    }

    /// Runs the preview over an app's sample songs, which carry only the common fields.
    pub fn preview_sample(&self, songs: Vec<SmartSampleSong>, max: u32) -> SmartPreviewResult {
        let st = self.state.lock().unwrap().clone();
        let mut eval = Evaluator::new(&st, Source::Sample, PlaylistMembers::new(), smart::today_days());
        for s in songs {
            eval.feed(&PreviewSong {
                rowid: s.rowid,
                id: s.id,
                title: s.title,
                album: s.album,
                artist: s.artist,
                genres: if s.genre.is_empty() { Vec::new() } else { vec![s.genre] },
                year: s.year.map(f64::from),
                plays: Some(f64::from(s.plays)),
                loved: Some(s.loved),
                duration: Some(f64::from(s.duration_seconds)),
                ..Default::default()
            });
        }
        eval.finish(&st, max as usize).into()
    }
}

/// Groups nest at most this deep, counting the top level.
#[uniffi::export]
pub fn smart_max_group_depth() -> u32 {
    smart::MAX_GROUP_DEPTH as u32
}
