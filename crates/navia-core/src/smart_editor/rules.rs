//! The editor's rules, with groups to any depth, several sort fields and a limit in songs or percent.
//! Read from and written to Navidrome's JSON, and rules the rows can't show are kept word for word.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};

use super::fields::{FIELDS, Shape, SORT_RANDOM, field_by_id, field_name, op_name, ops_for, shape_of};
use super::summary::describe;
use super::{Say, n_};

/// Groups nest at most this deep, counting the top level. [`crate::smart_rules`] refuses deeper.
pub const MAX_GROUP_DEPTH: usize = 16;

#[derive(Clone, PartialEq, Debug)]
pub enum RuleValue {
    One(String),
    Flag(bool),
    Range(String, String),
}

pub fn blank_value(s: Shape) -> RuleValue {
    if s.is_range() {
        RuleValue::Range(String::new(), String::new())
    } else if s == Shape::Bool {
        RuleValue::Flag(true)
    } else {
        RuleValue::One(String::new())
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct SimpleRule {
    pub key: u64,
    pub field: String,
    pub op: String,
    pub value: RuleValue,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Row {
    Simple(SimpleRule),
    /// A rule the rows can't show, kept word for word so saving never loses it. `reason` is a catalog key.
    Kept { key: u64, raw: Value, text: String, reason: &'static str },
    /// A group of rules and groups with its own All/Any.
    Group { key: u64, any: bool, rows: Vec<Row> },
}

impl Row {
    pub fn key(&self) -> u64 {
        match self {
            Row::Simple(r) => r.key,
            Row::Kept { key, .. } | Row::Group { key, .. } => *key,
        }
    }
}

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

pub fn next_key() -> u64 {
    NEXT_KEY.fetch_add(1, Ordering::Relaxed)
}

pub fn new_rule() -> SimpleRule {
    let field = &FIELDS[0];
    let op = ops_for(field)[0];
    SimpleRule { key: next_key(), field: field.id.to_string(), op: op.to_string(), value: blank_value(shape_of(field, op)) }
}

/// A new group starts with one empty rule, so it's never empty unless that rule is removed.
pub fn new_group() -> Row {
    Row::Group { key: next_key(), any: true, rows: vec![Row::Simple(new_rule())] }
}

fn to_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s == "true" => Some(true),
        Value::String(s) if s == "false" => Some(false),
        Value::Number(n) if n.as_i64() == Some(1) => Some(true),
        Value::Number(n) if n.as_i64() == Some(0) => Some(false),
        _ => None,
    }
}

pub(crate) fn group_of(raw: &Map<String, Value>) -> Option<&'static str> {
    if raw.get("all").is_some_and(Value::is_array) {
        Some("all")
    } else if raw.get("any").is_some_and(Value::is_array) {
        Some("any")
    } else {
        None
    }
}

fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(number_text(n)),
        _ => None,
    }
}

pub(crate) fn number_text(n: &serde_json::Number) -> String {
    n.as_i64().map_or_else(|| n.to_string(), |i| i.to_string())
}

fn value_from_json(shape: Shape, v: &Value) -> Option<RuleValue> {
    if shape == Shape::None {
        return Some(RuleValue::Flag(true));
    }
    if shape == Shape::Bool {
        return to_bool(v).map(RuleValue::Flag);
    }
    if shape.is_range() {
        let list = v.as_array()?;
        if list.len() != 2 {
            return None;
        }
        return Some(RuleValue::Range(scalar_text(&list[0])?, scalar_text(&list[1])?));
    }
    scalar_text(v).map(RuleValue::One)
}

fn keep(raw: &Value, reason: &'static str, say: Say) -> Row {
    Row::Kept { key: next_key(), raw: raw.clone(), text: describe(raw, say), reason }
}

/// One rule or group from Navidrome's JSON. Anything the rows can't show is kept as written, with the reason.
pub fn parse_rule(raw: &Value, say: Say) -> Row {
    let Some(obj) = raw.as_object() else { return keep(raw, n_("Unrecognized rule"), say) };
    if let Some(g) = group_of(obj) {
        // A group with both lists, or other keys beside its list, can't be shown faithfully.
        if obj.len() != 1 {
            return keep(raw, n_("Unrecognized rule"), say);
        }
        let rows = obj[g].as_array().map(|list| list.iter().map(|r| parse_rule(r, say)).collect()).unwrap_or_default();
        return Row::Group { key: next_key(), any: g == "any", rows };
    }
    let (Some((raw_op, body)), 1) = (obj.iter().next(), obj.len()) else { return keep(raw, n_("Unrecognized rule"), say) };
    let Some(body_obj) = body.as_object() else { return keep(raw, n_("Unrecognized rule"), say) };
    let Some(op) = op_name(raw_op) else { return keep(raw, n_("Unrecognized rule"), say) };
    if body_obj.len() != 1 {
        return keep(raw, n_("Unrecognized rule"), say);
    }
    let (raw_field, raw_value) = body_obj.iter().next().expect("one entry");
    if raw_field.eq_ignore_ascii_case("mood") {
        return keep(raw, n_("Moods are not offered"), say);
    }
    if op == "inPlaylist" || op == "notInPlaylist" {
        let (true, Some(id)) = (raw_field == "id", raw_value.as_str()) else { return keep(raw, n_("Playlist given by file path"), say) };
        return Row::Simple(SimpleRule { key: next_key(), field: "id".to_string(), op: op.to_string(), value: RuleValue::One(id.to_string()) });
    }
    let Some(f) = field_by_id(&field_name(raw_field)) else { return keep(raw, n_("Field not in the standard set"), say) };
    let mut norm = op.to_string();
    if op == "isMissing" || op == "isPresent" {
        let Some(b) = to_bool(raw_value) else { return keep(raw, n_("Unrecognized value"), say) };
        if !b {
            norm = if op == "isMissing" { "isPresent".to_string() } else { "isMissing".to_string() };
        }
    }
    if !ops_for(f).contains(&norm.as_str()) {
        return keep(raw, n_("Navidrome rejects this condition on this field"), say);
    }
    let Some(value) = value_from_json(shape_of(f, &norm), raw_value) else { return keep(raw, n_("Unrecognized value"), say) };
    Row::Simple(SimpleRule { key: next_key(), field: f.id.to_string(), op: norm, value })
}

/// One sort field and its direction.
#[derive(Clone, PartialEq, Debug)]
pub struct SortKey {
    pub key: u64,
    pub field: String,
    pub desc: bool,
}

pub fn sort_key(field: &str, desc: bool) -> SortKey {
    SortKey { key: next_key(), field: field.to_string(), desc }
}

/// Navidrome's sort text, one field with `order` for the direction, or a comma list with +/-.
pub fn parse_sort(sort: Option<&Value>, order: Option<&Value>) -> Vec<SortKey> {
    let Some(sort) = sort.and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else { return Vec::new() };
    let flip = order.and_then(Value::as_str).is_some_and(|o| o.eq_ignore_ascii_case("desc"));
    sort.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|part| {
            let id = part.trim_start_matches(['+', '-']);
            let field = if id == SORT_RANDOM { SORT_RANDOM.to_string() } else { field_name(id) };
            sort_key(&field, part.starts_with('-') != flip)
        })
        .collect()
}

/// Whether a limit counts songs or is a percentage of the matches.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LimitUnit {
    #[default]
    Songs,
    Percent,
}

#[derive(Clone, PartialEq, Debug)]
pub struct EditorState {
    pub match_any: bool,
    pub rows: Vec<Row>,
    pub sorts: Vec<SortKey>,
    /// The limit as typed. `unit` says whether it's `limit` or `limitPercent`, from 1 to 100.
    pub limit: String,
    pub unit: LimitUnit,
    /// Top-level keys the editor doesn't show, sent back as they were.
    pub extra: Map<String, Value>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self { match_any: false, rows: Vec::new(), sorts: vec![sort_key("title", false)], limit: String::new(), unit: LimitUnit::Songs, extra: Map::new() }
    }
}

/// The editor's state for saved rules, or a blank one.
pub fn parse_criteria(raw: Option<&str>, say: Say) -> EditorState {
    let mut state = EditorState::default();
    let Some(raw) = raw else { return state };
    let Ok(Value::Object(mut j)) = serde_json::from_str::<Value>(raw) else { return state };
    let all = j.remove("all");
    let any = j.remove("any");
    let sort = j.remove("sort");
    let order = j.remove("order");
    let limit = j.remove("limit");
    let limit_percent = j.remove("limitPercent");
    state.extra = j.clone();
    let all_list = all.as_ref().filter(|v| v.is_array());
    let any_list = any.as_ref().filter(|v| v.is_array());
    if any_list.is_some() && all_list.is_none() {
        state.match_any = true;
    }
    if let Some(list) = if state.match_any { any_list } else { all_list }.and_then(Value::as_array) {
        state.rows = list.iter().map(|r| parse_rule(r, say)).collect();
    }
    if let (Some(_), Some(any)) = (all_list, any_list) {
        // Navidrome rejects both at the top level, but keep the second if it turns up so nothing is dropped.
        state.rows.push(keep(&json!({ "any": any }), n_("Second top-level group"), say));
    }
    if all_list.is_none() && any_list.is_none() && j.keys().any(|k| op_name(k).is_some()) {
        state.rows = vec![parse_rule(&Value::Object(j), say)];
        state.extra.clear();
    }
    let sorts = parse_sort(sort.as_ref(), order.as_ref());
    if !sorts.is_empty() {
        state.sorts = sorts;
    }
    if let Some(n) = limit.as_ref().and_then(Value::as_i64).filter(|n| *n > 0) {
        state.limit = n.to_string();
    } else if let Some(p) = limit_percent.as_ref().and_then(Value::as_f64).filter(|p| *p > 0.0) {
        state.limit = (p.round() as i64).clamp(1, 100).to_string();
        state.unit = LimitUnit::Percent;
    }
    state
}

fn value_to_json(r: &SimpleRule, s: Shape) -> Value {
    let number = |v: &str| v.parse::<f64>().map_or(Value::Null, |n| if n.fract() == 0.0 { json!(n as i64) } else { json!(n) });
    match (s, &r.value) {
        (Shape::None, _) => Value::Bool(true),
        (Shape::Bool, RuleValue::Flag(b)) => Value::Bool(*b),
        (Shape::Number | Shape::Year | Shape::Days, RuleValue::One(v)) => number(v),
        (Shape::RangeNumber | Shape::RangeYear, RuleValue::Range(a, b)) => json!([number(a), number(b)]),
        (Shape::RangeDate, RuleValue::Range(a, b)) => json!([a, b]),
        (_, RuleValue::One(v)) => Value::String(v.clone()),
        _ => Value::Null,
    }
}

pub fn rule_to_json(r: &SimpleRule) -> Value {
    let shape = field_by_id(&r.field).map_or(Shape::Text, |f| shape_of(f, &r.op));
    let mut body = Map::new();
    body.insert(r.field.clone(), value_to_json(r, shape));
    let mut outer = Map::new();
    outer.insert(r.op.clone(), Value::Object(body));
    Value::Object(outer)
}

pub fn row_to_json(r: &Row) -> Value {
    match r {
        Row::Simple(s) => rule_to_json(s),
        Row::Kept { raw, .. } => raw.clone(),
        Row::Group { any, rows, .. } => json!({ if *any { "any" } else { "all" }: rows.iter().map(row_to_json).collect::<Vec<_>>() }),
    }
}

/// Empty groups add nothing, so they're left out of what's saved, the preview and the summary.
pub fn drop_empty(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .filter_map(|r| match r {
            Row::Group { key, any, rows } => {
                let inner = drop_empty(rows);
                (!inner.is_empty()).then(|| Row::Group { key: *key, any: *any, rows: inner })
            }
            other => Some(other.clone()),
        })
        .collect()
}

/// The limit as a whole number, or 0 for none. A percentage is at most 100.
pub fn limit_value(st: &EditorState) -> u32 {
    let n: u32 = st.limit.parse().unwrap_or(0);
    if st.unit == LimitUnit::Percent { n.min(100) } else { n }
}

/// A typed limit kept to digits, and to 100 at most for a percentage, so it never needs an error.
pub fn clamp_limit(text: &str, unit: LimitUnit) -> String {
    let digits: String = text.chars().filter(char::is_ascii_digit).take(6).collect();
    if unit == LimitUnit::Percent && digits.parse::<u32>().is_ok_and(|n| n > 100) { "100".to_string() } else { digits }
}

/// Navidrome's JSON for these rules, as Save sends them.
/// One sort field is written with `order` so older servers read it, and several as a `+a,-b` list.
pub fn to_criteria(st: &EditorState) -> Value {
    let mut out = st.extra.clone();
    out.insert(if st.match_any { "any" } else { "all" }.to_string(), Value::Array(drop_empty(&st.rows).iter().map(row_to_json).collect()));
    match st.sorts.as_slice() {
        [] => {}
        [one] => {
            out.insert("sort".to_string(), json!(one.field));
            if one.desc && one.field != SORT_RANDOM {
                out.insert("order".to_string(), json!("desc"));
            }
        }
        many => {
            out.insert("sort".to_string(), json!(many.iter().map(|s| format!("{}{}", if s.desc { '-' } else { '+' }, s.field)).collect::<Vec<_>>().join(",")));
        }
    }
    let n = limit_value(st);
    if n > 0 {
        out.insert(if st.unit == LimitUnit::Songs { "limit" } else { "limitPercent" }.to_string(), json!(n));
    }
    Value::Object(out)
}

fn matches_pattern(shape: Shape, v: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match shape {
        Shape::Number | Shape::RangeNumber => {
            let body = v.strip_prefix('-').unwrap_or(v);
            match body.split_once('.') {
                Some((a, b)) => digits(a) && digits(b),
                None => digits(body),
            }
        }
        Shape::Year | Shape::RangeYear => v.len() == 4 && digits(v),
        Shape::Days => digits(v),
        Shape::Date | Shape::RangeDate => {
            let b = v.as_bytes();
            v.len() == 10 && b[4] == b'-' && b[7] == b'-' && digits(&v[..4]) && digits(&v[5..7]) && digits(&v[8..])
        }
        _ => true,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Issue {
    None,
    Empty,
    Invalid,
}

pub fn rule_issue(r: &SimpleRule) -> Issue {
    let Some(f) = field_by_id(&r.field) else { return Issue::Invalid };
    let s = shape_of(f, &r.op);
    if s == Shape::None || s == Shape::Bool {
        return Issue::None;
    }
    let values: Vec<&str> = match &r.value {
        RuleValue::Range(a, b) => vec![a, b],
        RuleValue::One(v) => vec![v],
        RuleValue::Flag(_) => vec![],
    };
    if values.iter().any(|v| v.trim().is_empty()) {
        return Issue::Empty;
    }
    if values.iter().any(|v| !matches_pattern(s, v)) {
        return Issue::Invalid;
    }
    Issue::None
}

/// Whether every rule, in every group, is ready to save. Empty groups don't count against it.
pub fn rows_complete(rows: &[Row]) -> bool {
    rows.iter().all(|r| match r {
        Row::Simple(s) => rule_issue(s) == Issue::None,
        Row::Kept { .. } => true,
        Row::Group { rows, .. } => rows_complete(rows),
    })
}

/// Whether any rule is left once empty groups are set aside.
pub fn has_rules(rows: &[Row]) -> bool {
    !drop_empty(rows).is_empty()
}

/// The rows that can be read now, skipping rules still being typed and empty groups.
pub fn ready_rows(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .filter_map(|r| match r {
            Row::Simple(s) => (rule_issue(s) == Issue::None).then(|| r.clone()),
            Row::Kept { .. } => Some(r.clone()),
            Row::Group { key, any, rows } => {
                let inner = ready_rows(rows);
                (!inner.is_empty()).then(|| Row::Group { key: *key, any: *any, rows: inner })
            }
        })
        .collect()
}

/// Sorting by date added alone mixes albums added the same day, so Navidrome's docs suggest Album next.
pub fn wants_album_next(sorts: &[SortKey], i: usize) -> bool {
    sorts.get(i).is_some_and(|s| s.field == "dateadded" || s.field == "albumdateadded") && sorts.get(i + 1).is_none_or(|n| n.field != "album")
}

/// The first sort row that would want Album after it ([`wants_album_next`]).
pub fn album_hint_at(sorts: &[SortKey]) -> Option<usize> {
    (0..sorts.len()).find(|i| wants_album_next(sorts, *i))
}

/// The field a new sort row starts with, the first of these not sorted by yet.
pub fn next_sort_field(sorts: &[SortKey]) -> &'static str {
    ["album", "title", "artist", "year"].into_iter().find(|f| !sorts.iter().any(|s| s.field == *f)).unwrap_or("title")
}

impl EditorState {
    /// The rows a group holds, `None` for the top level.
    pub fn rows_of_mut(&mut self, group: Option<u64>) -> Option<&mut Vec<Row>> {
        match group {
            None => Some(&mut self.rows),
            Some(key) => find_group(&mut self.rows, key),
        }
    }

    pub fn rule_mut(&mut self, key: u64) -> Option<&mut SimpleRule> {
        find_rule(&mut self.rows, key)
    }

    pub fn rule(&self, key: u64) -> Option<&SimpleRule> {
        fn walk(rows: &[Row], key: u64) -> Option<&SimpleRule> {
            rows.iter().find_map(|r| match r {
                Row::Simple(s) if s.key == key => Some(s),
                Row::Group { rows, .. } => walk(rows, key),
                _ => None,
            })
        }
        walk(&self.rows, key)
    }

    /// Sets a group's All/Any, `None` for the top level.
    pub fn set_any(&mut self, group: Option<u64>, any: bool) {
        match group {
            None => self.match_any = any,
            Some(key) => {
                if let Some(Row::Group { any: a, .. }) = find_row(&mut self.rows, key) {
                    *a = any;
                }
            }
        }
    }

    pub fn add_rule(&mut self, group: Option<u64>) -> Option<u64> {
        let rule = new_rule();
        let key = rule.key;
        self.rows_of_mut(group)?.push(Row::Simple(rule));
        Some(key)
    }

    /// Adds a group, unless that would nest deeper than [`MAX_GROUP_DEPTH`].
    pub fn add_group(&mut self, group: Option<u64>) -> Option<u64> {
        if group.is_some_and(|g| self.depth_of(g).is_none_or(|d| d + 1 >= MAX_GROUP_DEPTH)) {
            return None;
        }
        let row = new_group();
        let key = row.key();
        self.rows_of_mut(group)?.push(row);
        Some(key)
    }

    /// How deep a group sits. Top-level groups are 1.
    pub fn depth_of(&self, key: u64) -> Option<usize> {
        fn walk(rows: &[Row], key: u64, depth: usize) -> Option<usize> {
            rows.iter().find_map(|r| match r {
                Row::Group { key: k, .. } if *k == key => Some(depth),
                Row::Group { rows, .. } => walk(rows, key, depth + 1),
                _ => None,
            })
        }
        walk(&self.rows, key, 1)
    }

    pub fn remove(&mut self, key: u64) {
        fn walk(rows: &mut Vec<Row>, key: u64) -> bool {
            if let Some(i) = rows.iter().position(|r| r.key() == key) {
                rows.remove(i);
                return true;
            }
            rows.iter_mut().any(|r| match r {
                Row::Group { rows, .. } => walk(rows, key),
                _ => false,
            })
        }
        walk(&mut self.rows, key);
    }

    /// Changes a rule's field, keeping its condition and value where the new field can take them.
    pub fn set_field(&mut self, key: u64, id: &str) {
        let Some(next) = field_by_id(id) else { return };
        let Some(rule) = self.rule_mut(key) else { return };
        let old_shape = field_by_id(&rule.field).map(|f| shape_of(f, &rule.op));
        let ops = ops_for(next);
        let op = if ops.contains(&rule.op.as_str()) { rule.op.clone() } else { ops[0].to_string() };
        let next_shape = shape_of(next, &op);
        if old_shape != Some(next_shape) {
            rule.value = blank_value(next_shape);
        }
        rule.field = next.id.to_string();
        rule.op = op;
    }

    pub fn set_op(&mut self, key: u64, op: &str) {
        let Some(rule) = self.rule_mut(key) else { return };
        let Some(f) = field_by_id(&rule.field) else {
            rule.op = op.to_string();
            return;
        };
        if shape_of(f, op) != shape_of(f, &rule.op) {
            rule.value = blank_value(shape_of(f, op));
        }
        rule.op = op.to_string();
    }

    /// Types into a rule's value, keeping only what the value can hold. `side` 1 is a range's second value.
    pub fn set_value_text(&mut self, key: u64, side: u8, text: &str) {
        let Some(rule) = self.rule_mut(key) else { return };
        let shape = field_by_id(&rule.field).map_or(Shape::Text, |f| shape_of(f, &rule.op));
        let text = shape.clean(text);
        match &mut rule.value {
            RuleValue::One(v) => *v = text,
            RuleValue::Range(a, b) => {
                if side == 0 { *a = text } else { *b = text }
            }
            RuleValue::Flag(_) => {}
        }
    }

    pub fn set_flag(&mut self, key: u64, on: bool) {
        if let Some(rule) = self.rule_mut(key) {
            rule.value = RuleValue::Flag(on);
        }
    }

    /// Picks a value from a list, such as a genre or a playlist.
    pub fn set_choice(&mut self, key: u64, value: &str) {
        if let Some(rule) = self.rule_mut(key) {
            rule.value = RuleValue::One(value.to_string());
        }
    }

    /// Sets a sort row's field. Random takes the whole order, so it leaves no other rows.
    pub fn set_sort_field(&mut self, i: usize, field: &str) {
        let desc = super::fields::directions(field)[0].desc;
        if field == SORT_RANDOM {
            self.sorts = vec![sort_key(SORT_RANDOM, false)];
        } else if let Some(s) = self.sorts.get_mut(i) {
            s.field = field.to_string();
            s.desc = desc;
        }
    }

    pub fn set_sort_desc(&mut self, i: usize, desc: bool) {
        if let Some(s) = self.sorts.get_mut(i) {
            s.desc = desc;
        }
    }

    /// Adds a sort row at `at`, or at the end, with `field` or the next unused common one.
    pub fn add_sort(&mut self, at: usize, field: Option<&str>) {
        if self.sorts.first().is_some_and(|s| s.field == SORT_RANDOM) {
            return;
        }
        let field = field.unwrap_or_else(|| next_sort_field(&self.sorts)).to_string();
        let desc = super::fields::directions(&field)[0].desc;
        self.sorts.insert(at.min(self.sorts.len()), sort_key(&field, desc));
    }

    /// Removes a sort row. The last one stays.
    pub fn remove_sort(&mut self, i: usize) {
        if self.sorts.len() > 1 && i < self.sorts.len() {
            self.sorts.remove(i);
        }
    }

    pub fn set_limit(&mut self, text: &str) {
        self.limit = clamp_limit(text, self.unit);
    }

    pub fn set_unit(&mut self, unit: LimitUnit) {
        self.unit = unit;
        self.limit = clamp_limit(&self.limit, unit);
    }
}

fn find_row(rows: &mut [Row], key: u64) -> Option<&mut Row> {
    for r in rows.iter_mut() {
        if r.key() == key {
            return Some(r);
        }
        if let Row::Group { rows, .. } = r {
            if let Some(found) = find_row(rows, key) {
                return Some(found);
            }
        }
    }
    None
}

fn find_group(rows: &mut [Row], key: u64) -> Option<&mut Vec<Row>> {
    match find_row(rows, key)? {
        Row::Group { rows, .. } => Some(rows),
        _ => None,
    }
}

fn find_rule(rows: &mut [Row], key: u64) -> Option<&mut SimpleRule> {
    match find_row(rows, key)? {
        Row::Simple(s) => Some(s),
        _ => None,
    }
}

/// One row of the editor in display order. Depth 0 is the top level's rules.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatRow<'a> {
    pub row: &'a Row,
    pub depth: usize,
    /// The group this row is in, `None` for the top level.
    pub parent: Option<u64>,
}

/// The rows in display order with their depth, for apps that draw a flat list.
pub fn flatten(rows: &[Row]) -> Vec<FlatRow<'_>> {
    fn walk<'a>(rows: &'a [Row], depth: usize, parent: Option<u64>, out: &mut Vec<FlatRow<'a>>) {
        for r in rows {
            out.push(FlatRow { row: r, depth, parent });
            if let Row::Group { key, rows, .. } = r {
                walk(rows, depth + 1, Some(*key), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(rows, 0, None, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::super::English;
    use super::*;

    fn say() -> Say<'static> {
        Say(&English)
    }

    #[test]
    fn parses_and_serializes_navidromes_rules() {
        let raw = r#"{"all":[{"is":{"genre":"Rock"}},{"inTheRange":{"year":[1990,1999]}},{"isPresent":{"bitdepth":true}},{"is":{"loved":true}}],"sort":"title","order":"desc","limit":50}"#;
        let st = parse_criteria(Some(raw), say());
        assert!(!st.match_any);
        assert_eq!(st.rows.len(), 4);
        assert!(st.sorts[0].desc);
        assert_eq!(st.limit, "50");
        assert!(st.rows.iter().all(|r| matches!(r, Row::Simple(_))));
        assert_eq!(to_criteria(&st), serde_json::from_str::<Value>(raw).unwrap());
    }

    #[test]
    fn groups_nest_and_round_trip() {
        let raw = r#"{"all":[{"any":[{"is":{"genre":"Jazz"}},{"is":{"genre":"Soul"}}]},{"gt":{"year":2000}},{"any":[{"gt":{"playcount":3}},{"all":[{"is":{"loved":true}},{"inTheLast":{"dateadded":90}}]}]}],"sort":"-dateadded,+album","limitPercent":10}"#;
        let st = parse_criteria(Some(raw), say());
        assert!(matches!(&st.rows[0], Row::Group { any: true, rows, .. } if rows.len() == 2));
        assert!(matches!(&st.rows[2], Row::Group { rows, .. } if matches!(&rows[1], Row::Group { any: false, .. })));
        assert_eq!((st.sorts.len(), st.sorts[0].desc, st.sorts[1].desc), (2, true, false));
        assert_eq!((st.limit.as_str(), st.unit), ("10", LimitUnit::Percent));
        assert_eq!(to_criteria(&st), serde_json::from_str::<Value>(raw).unwrap());
        assert_eq!(flatten(&st.rows).iter().map(|f| f.depth).collect::<Vec<_>>(), vec![0, 1, 1, 0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn unsupported_rules_are_kept_verbatim() {
        let raw = r#"{"any":[{"is":{"mood":"sad"}},{"contains":{"unknownfield":"x"}},{"inTheRange":{"title":["a","b"]}},{"all":[],"any":[]}],"sort":"title","extraKey":1}"#;
        let st = parse_criteria(Some(raw), say());
        assert!(st.match_any);
        assert!(st.rows.iter().all(|r| matches!(r, Row::Kept { .. })));
        assert_eq!(to_criteria(&st), serde_json::from_str::<Value>(raw).unwrap());
    }

    #[test]
    fn empty_groups_are_left_out_and_dont_block() {
        let mut st = EditorState::default();
        let g = st.add_group(None).unwrap();
        let Row::Group { rows, .. } = &st.rows[0] else { panic!() };
        let inner = rows[0].key();
        assert!(!rows_complete(&st.rows), "the new group's empty rule isn't finished");
        st.remove(inner);
        assert!(rows_complete(&st.rows));
        assert!(!has_rules(&st.rows));
        st.add_rule(None);
        st.set_field(st.rows[1].key(), "loved");
        assert!(has_rules(&st.rows));
        assert_eq!(to_criteria(&st)["all"].as_array().unwrap().len(), 1);
        assert!(st.add_rule(Some(g)).is_some());
    }

    #[test]
    fn groups_stop_at_the_depth_limit() {
        let mut st = EditorState::default();
        let mut at = None;
        let mut made = 0;
        while let Some(g) = st.add_group(at) {
            at = Some(g);
            made += 1;
        }
        assert_eq!(made, MAX_GROUP_DEPTH - 1);
        assert!(crate::smart_rules::checked_criteria(&to_criteria(&st).to_string()).is_ok(), "the core's own check takes this depth");
    }

    #[test]
    fn sorts_and_limits() {
        let mut st = EditorState::default();
        st.set_sort_field(0, "dateadded");
        assert_eq!(album_hint_at(&st.sorts), Some(0));
        st.add_sort(1, Some("album"));
        assert_eq!(album_hint_at(&st.sorts), None);
        assert_eq!(to_criteria(&st)["sort"], json!("-dateadded,+album"));
        st.set_sort_field(1, "random");
        assert_eq!(st.sorts.len(), 1);
        assert_eq!(to_criteria(&st)["sort"], json!("random"));
        st.set_unit(LimitUnit::Percent);
        st.set_limit("250");
        assert_eq!(st.limit, "100");
        assert_eq!(to_criteria(&st)["limitPercent"], json!(100));
        st.set_unit(LimitUnit::Songs);
        st.set_limit("250");
        assert_eq!(to_criteria(&st)["limit"], json!(250));
    }

    #[test]
    fn is_missing_false_becomes_is_present() {
        let st = parse_criteria(Some(r#"{"all":[{"isMissing":{"bpm":false}}]}"#), say());
        let Row::Simple(r) = &st.rows[0] else { panic!("kept") };
        assert_eq!(r.op, "isPresent");
    }

    #[test]
    fn validation_catches_empty_and_malformed() {
        let mut r = new_rule();
        assert_eq!(rule_issue(&r), Issue::Empty);
        r.field = "year".into();
        r.op = "is".into();
        r.value = RuleValue::One("19".into());
        assert_eq!(rule_issue(&r), Issue::Invalid);
        r.value = RuleValue::One("1999".into());
        assert_eq!(rule_issue(&r), Issue::None);
        r.field = "lastplayed".into();
        r.op = "is".into();
        r.value = RuleValue::One("2024-1-1".into());
        assert_eq!(rule_issue(&r), Issue::Invalid);
    }
}
