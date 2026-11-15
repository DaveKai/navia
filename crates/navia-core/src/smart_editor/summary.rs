//! Rules in words, each rule as the rules list says it and the whole playlist in one line.
//! For example "Jazz or Soul songs from after 2000, plus 3 more conditions, most played first, top 50".

use serde_json::{Map, Value};

use super::Say;
use super::fields::{SORT_RANDOM, direction_label, field_by_id, field_name, op_label, op_name};
use super::rules::{EditorState, group_of, number_text, parse_sort, ready_rows, to_criteria};

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => number_text(n),
        other => other.to_string(),
    }
}

fn field_label(field: &str, say: Say) -> String {
    field_by_id(&field_name(field)).map_or(field.to_string(), |f| say.tr(f.label))
}

fn playlist_id(value: &Value) -> String {
    value.as_str().map(str::to_string).or_else(|| value.get("id").and_then(Value::as_str).map(str::to_string)).unwrap_or_default()
}

/// One plain rule in words, like "Album contains Blue".
fn describe_leaf(op: &str, field: &str, value: &Value, say: Say) -> String {
    let label = field_label(field, say);
    let cond = op_name(op).map_or_else(|| op.to_string(), |o| say.tr(op_label(o)));
    if op.eq_ignore_ascii_case("isMissing") || op.eq_ignore_ascii_case("isPresent") {
        return say.trf("{field} {condition}", &[("field", &label), ("condition", &cond)]);
    }
    if op.eq_ignore_ascii_case("inPlaylist") || op.eq_ignore_ascii_case("notInPlaylist") {
        return say.trf("{condition} {playlist}", &[("condition", &cond), ("playlist", &say.playlist(&playlist_id(value)))]);
    }
    let val = match value {
        Value::Array(a) if a.len() == 2 => say.trf("{from} to {to}", &[("from", &value_text(&a[0])), ("to", &value_text(&a[1]))]),
        Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Bool(b) => if *b { say.tr("Yes") } else { say.tr("No") },
        other if op.eq_ignore_ascii_case("inTheLast") || op.eq_ignore_ascii_case("notInTheLast") => value_text(other).parse::<u64>().map_or_else(|_| value_text(other), |d| say.trn("{count} day", "{count} days", d)),
        other => value_text(other),
    };
    say.trf("{field} {condition} {value}", &[("field", &label), ("condition", &cond), ("value", &val)])
}

/// A leaf's operator, normalized field and value, or `None` when it isn't one plain rule.
fn leaf_of(raw: &Value) -> Option<(&'static str, String, &Value)> {
    let obj = raw.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let (op, body) = obj.iter().next()?;
    let op = op_name(op)?;
    let body = body.as_object()?;
    if body.len() != 1 {
        return None;
    }
    let (field, value) = body.iter().next()?;
    Some((op, field_name(field), value))
}

/// A rule or group in words, for the rules list and for rules kept as written.
pub fn describe(raw: &Value, say: Say) -> String {
    let Some(obj) = raw.as_object() else { return raw.to_string() };
    if let Some(g) = group_of(obj) {
        let parts = obj[g].as_array().map(|a| a.iter().map(|c| describe(c, say)).collect::<Vec<_>>().join("; ")).unwrap_or_default();
        return if g == "all" { say.trf("All of: {rules}", &[("rules", &parts)]) } else { say.trf("Any of: {rules}", &[("rules", &parts)]) };
    }
    leaf_of(raw).map_or_else(|| raw.to_string(), |(op, field, value)| describe_leaf(op, &field, value, say))
}

/// A clause after "Songs". Plain ones read on their own, and a run of `where` ones shares one "where".
struct Phrase {
    text: String,
    is_where: bool,
}

/// At most this many clauses, with the rest counted so the summary stays one line.
const SUMMARY_CLAUSES: usize = 4;

/// One rule as a clause, like "from after 2000" or "played in the last 30 days".
fn leaf_text(op: &str, field: &str, value: &Value, say: Say) -> Option<String> {
    let text = value_text(value);
    let n = text.parse::<f64>().unwrap_or(0.0).max(0.0) as u64;
    let pair = value.as_array().filter(|a| a.len() == 2).map(|a| (value_text(&a[0]), value_text(&a[1])));
    let range = |key: &str| pair.as_ref().map(|(a, b)| say.trf(key, &[("from", a), ("to", b)]));
    match op {
        "inPlaylist" => return Some(say.trf("in {playlist}", &[("playlist", &say.playlist(&playlist_id(value)))])),
        "notInPlaylist" => return Some(say.trf("not in {playlist}", &[("playlist", &say.playlist(&playlist_id(value)))])),
        _ => {}
    }
    match (field, op) {
        ("year", "gt") => Some(say.trf("from after {year}", &[("year", &text)])),
        ("year", "lt") => Some(say.trf("from before {year}", &[("year", &text)])),
        ("year", "is") => Some(say.trf("from {year}", &[("year", &text)])),
        ("year", "inTheRange") => range("from {from} to {to}"),
        ("artist", "is") => Some(say.trf("by {artist}", &[("artist", &text)])),
        ("artist", "isNot") => Some(say.trf("not by {artist}", &[("artist", &text)])),
        ("genre", "is") => Some(say.trf("in the genre {genre}", &[("genre", &text)])),
        ("genre", "isNot") => Some(say.trf("not in the genre {genre}", &[("genre", &text)])),
        ("loved", "is" | "isNot") => {
            let yes = match value {
                Value::Bool(b) => *b,
                Value::String(s) => s == "true",
                Value::Number(n) => n.as_i64() == Some(1),
                _ => return None,
            };
            Some(if yes == (op == "is") { say.tr("in your favorites") } else { say.tr("not in your favorites") })
        }
        ("playcount", "gt") => Some(say.trn("played more than {count} time", "played more than {count} times", n)),
        ("playcount", "lt") => Some(say.trn("played fewer than {count} time", "played fewer than {count} times", n)),
        ("playcount", "is") => Some(if n == 0 { say.tr("never played") } else { say.trn("played {count} time", "played {count} times", n) }),
        ("playcount", "inTheRange") => range("played {from} to {to} times"),
        ("lastplayed", "inTheLast") => Some(say.trn("played in the last {count} day", "played in the last {count} days", n)),
        ("lastplayed", "notInTheLast") => Some(say.trn("not played in the last {count} day", "not played in the last {count} days", n)),
        ("lastplayed", "before") => Some(say.trf("last played before {date}", &[("date", &text)])),
        ("lastplayed", "after") => Some(say.trf("last played after {date}", &[("date", &text)])),
        ("lastplayed", "isMissing") => Some(say.tr("never played")),
        ("dateadded", "inTheLast") => Some(say.trn("added in the last {count} day", "added in the last {count} days", n)),
        ("dateadded", "notInTheLast") => Some(say.trn("not added in the last {count} day", "not added in the last {count} days", n)),
        ("dateadded", "before") => Some(say.trf("added before {date}", &[("date", &text)])),
        ("dateadded", "after") => Some(say.trf("added after {date}", &[("date", &text)])),
        _ => None,
    }
}

fn phrase(raw: &Value, say: Say) -> Phrase {
    match leaf_of(raw) {
        Some((op, field, value)) => match leaf_text(op, &field, value, say) {
            Some(text) => Phrase { text, is_where: false },
            None => Phrase { text: describe_leaf(op, &field, value, say), is_where: true },
        },
        None => Phrase { text: describe(raw, say), is_where: true },
    }
}

fn joiner(any: bool, say: Say) -> String {
    if any { say.tr(" or ") } else { say.tr(" and ") }
}

fn is_group(raw: &Value) -> bool {
    raw.as_object().is_some_and(|o| group_of(o).is_some())
}

/// How many plain rules a rule or group holds, counting nested groups' rules.
fn condition_count(raw: &Value) -> u64 {
    match raw.as_object().and_then(|o| group_of(o).map(|g| &o[g])) {
        Some(Value::Array(list)) => list.iter().map(condition_count).sum(),
        _ => 1,
    }
}

/// The top level's plain rules as clauses joined by and/or.
/// Groups aren't spelled out, so their rules and any past [`SUMMARY_CLAUSES`] are counted at the end.
fn join_phrases(items: &[&Value], any: bool, subject: &str, say: Say) -> String {
    let leaves: Vec<&&Value> = items.iter().filter(|r| !is_group(r)).collect();
    let shown: Vec<Phrase> = leaves.iter().take(SUMMARY_CLAUSES).map(|r| phrase(r, say)).collect();
    let more = (leaves.len() - shown.len()) as u64 + items.iter().filter(|r| is_group(r)).map(|r| condition_count(r)).sum::<u64>();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < shown.len() {
        if !shown[i].is_where {
            out.push(shown[i].text.clone());
            i += 1;
            continue;
        }
        let mut run = vec![shown[i].text.clone()];
        while shown.get(i + 1).is_some_and(|p| p.is_where) {
            i += 1;
            run.push(shown[i].text.clone());
        }
        out.push(say.trf("where {rules}", &[("rules", &run.join(&joiner(any, say)))]));
        i += 1;
    }
    if out.is_empty() {
        if more == 0 {
            return subject.to_string();
        }
        let meeting = if any { say.trn("meeting any of {count} condition", "meeting any of {count} conditions", more) } else { say.trn("meeting {count} condition", "meeting {count} conditions", more) };
        return format!("{subject} {meeting}");
    }
    let head = format!("{subject} {}", out.join(&joiner(any, say)));
    if more == 0 {
        return head;
    }
    let tail = if any { say.trn("or {count} other condition", "or {count} other conditions", more) } else { say.trn("plus {count} more condition", "plus {count} more conditions", more) };
    format!("{head}, {tail}")
}

/// The genre a rule names with "is", when it's a plain genre rule.
fn genre_is(raw: &Value) -> Option<String> {
    let (op, field, value) = leaf_of(raw)?;
    (op == "is" && field == "genre").then(|| value.as_str().map(str::to_string)).flatten()
}

/// Lowers a phrase's first letter for mid-sentence use, unless it's an initialism or a range like "A–Z".
fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    let (Some(first), Some(second)) = (chars.next(), chars.next()) else { return s.to_string() };
    if second.is_lowercase() { first.to_lowercase().chain(s.chars().skip(1)).collect() } else { s.to_string() }
}

/// The rules in one line, as the editor and the playlist page say them. Empty when there are no rules.
pub fn summary_sentence(j: &Map<String, Value>, say: Say) -> String {
    let top = group_of(j);
    let single = Value::Object(j.clone());
    let items: Vec<&Value> = match top {
        Some(g) => j[g].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
        None if j.keys().any(|k| op_name(k).is_some()) => vec![&single],
        None => Vec::new(),
    };
    if items.is_empty() {
        return String::new();
    }
    let any = top == Some("any");
    // Genres named with "is" lead, as in "Jazz or Soul songs". That's all of a top-level Any, or under All,
    // one rule or one Any group made only of them.
    let mut genres: Vec<String> = Vec::new();
    let mut rest = items.clone();
    let names: Vec<Option<String>> = items.iter().map(|r| genre_is(r)).collect();
    if any && names.iter().all(Option::is_some) {
        genres = names.into_iter().flatten().collect();
        rest.clear();
    } else if !any {
        let only_genres = |r: &Value| r.get("any").and_then(Value::as_array).filter(|l| !l.is_empty() && r.as_object().is_some_and(|o| o.len() == 1)).and_then(|l| l.iter().map(genre_is).collect::<Option<Vec<_>>>());
        if let Some(at) = items.iter().position(|r| genre_is(r).is_some() || only_genres(r).is_some()) {
            genres = genre_is(items[at]).map_or_else(|| only_genres(items[at]).unwrap_or_default(), |g| vec![g]);
            rest.remove(at);
        }
    }
    let subject = if genres.is_empty() { say.tr("Songs") } else { say.trf("{genres} songs", &[("genres", &genres.join(&say.tr(" or ")))]) };
    let mut parts = vec![join_phrases(&rest, any, &subject, say)];
    let sorts = parse_sort(j.get("sort"), j.get("order"));
    let random = sorts.len() == 1 && sorts[0].field == SORT_RANDOM;
    let limit = j.get("limit").and_then(Value::as_i64).filter(|n| *n > 0).unwrap_or(0);
    let percent = if limit > 0 { 0 } else { j.get("limitPercent").and_then(Value::as_f64).filter(|p| *p > 0.0).map_or(0, |p| p.round() as i64) };
    if random {
        parts.push(if limit > 0 {
            say.trf("{count} at random", &[("count", &say.0.int(limit))])
        } else if percent > 0 {
            say.trf("{percent}% at random", &[("percent", &percent.to_string())])
        } else {
            say.tr("in random order")
        });
    } else {
        if sorts.len() == 1 && sorts[0].field == "playcount" {
            // "Most played first" already names its field.
            parts.push(lower_first(&say.tr(direction_label(&sorts[0].field, sorts[0].desc))));
        } else if !sorts.is_empty() {
            parts.push(sorted_by(&sorts, say, true));
        }
        if limit > 0 {
            parts.push(say.trf("top {count}", &[("count", &say.0.int(limit))]));
        } else if percent > 0 {
            parts.push(say.trf("top {percent}%", &[("percent", &percent.to_string())]));
        }
    }
    parts.join(", ")
}

/// "sorted by Date Added (newest first), then Album (A–Z)". `mid` lowers it for mid-sentence use.
fn sorted_by(sorts: &[super::rules::SortKey], say: Say, mid: bool) -> String {
    let fields = sorts
        .iter()
        .map(|s| if s.field == SORT_RANDOM { say.tr("Random") } else { say.trf("{field} ({direction})", &[("field", &field_label(&s.field, say)), ("direction", &lower_first(&say.tr(direction_label(&s.field, s.desc))))]) })
        .collect::<Vec<_>>()
        .join(&say.tr(", then "));
    if mid { say.trf("sorted by {fields}", &[("fields", &fields)]) } else { say.trf("Sorted by {fields}", &[("fields", &fields)]) }
}

/// The editor's one-line summary, from the rules that are ready.
pub fn editor_sentence(st: &EditorState, say: Say) -> String {
    let ready = EditorState { rows: ready_rows(&st.rows), ..st.clone() };
    match to_criteria(&ready) {
        Value::Object(j) => summary_sentence(&j, say),
        _ => String::new(),
    }
}

/// A saved playlist's rules in words, for the playlist page and Get Info.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleSummary {
    /// The whole playlist in one line.
    pub sentence: String,
    /// "Matches all of" or "Matches any of", empty for a single rule.
    pub match_word: String,
    /// Each top-level rule or group in words.
    pub rules: Vec<String>,
    /// Sort and limit, in words.
    pub order: Vec<String>,
}

pub fn summarize_criteria(raw: &str, say: Say) -> Option<RuleSummary> {
    let Ok(Value::Object(j)) = serde_json::from_str::<Value>(raw) else { return None };
    let mut out = RuleSummary { sentence: summary_sentence(&j, say), ..Default::default() };
    if let Some(g) = group_of(&j) {
        out.match_word = if g == "all" { say.tr("Matches all of") } else { say.tr("Matches any of") };
        out.rules = j[g].as_array().map(|a| a.iter().map(|r| describe(r, say)).collect()).unwrap_or_default();
    } else if j.keys().any(|k| op_name(k).is_some()) {
        out.rules = vec![describe(&Value::Object(j.clone()), say)];
    }
    let sorts = parse_sort(j.get("sort"), j.get("order"));
    if !sorts.is_empty() {
        out.order.push(sorted_by(&sorts, say, false));
    }
    if let Some(limit) = j.get("limit").and_then(Value::as_i64).filter(|n| *n > 0) {
        out.order.push(say.trn("Top {count} song", "Top {count} songs", limit as u64));
    } else if let Some(p) = j.get("limitPercent").and_then(Value::as_f64).filter(|p| *p > 0.0) {
        out.order.push(say.trf("Top {percent}% of matches", &[("percent", &(p.round() as i64).to_string())]));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::super::{English, Words};
    use super::*;

    struct Names;
    impl Words for Names {
        fn text(&self, key: &str) -> String {
            English.text(key)
        }
        fn plural(&self, one: &str, other: &str, n: u64) -> String {
            English.plural(one, other, n)
        }
        fn int(&self, n: i64) -> String {
            n.to_string()
        }
        fn playlist_name(&self, id: &str) -> Option<String> {
            (id == "p1").then(|| "Jazz Mix".to_string())
        }
    }

    fn line(raw: &str) -> String {
        summarize_criteria(raw, Say(&Names)).unwrap().sentence
    }

    #[test]
    fn the_sentence_leads_with_genres_and_counts_groups() {
        assert_eq!(
            line(r#"{"all":[{"any":[{"is":{"genre":"Jazz"}},{"is":{"genre":"Soul"}}]},{"gt":{"year":2000}},{"any":[{"gt":{"playcount":3}},{"all":[{"is":{"loved":true}},{"inTheLast":{"dateadded":90}}]}]}],"sort":"dateadded","order":"desc","limit":50}"#),
            "Jazz or Soul songs from after 2000, plus 3 more conditions, sorted by Date Added (newest first), top 50"
        );
        assert_eq!(line(r#"{"any":[{"is":{"genre":"Jazz"}}],"sort":"-playcount","limit":50}"#), "Jazz songs, most played first, top 50");
        assert_eq!(line(r#"{"all":[{"any":[{"is":{"artist":"A"}},{"is":{"artist":"B"}}]}],"sort":"random","limit":20}"#), "Songs meeting 2 conditions, 20 at random");
        assert_eq!(line(r#"{"all":[{"contains":{"album":"Blue"}},{"lt":{"bpm":100}},{"notInPlaylist":{"id":"p1"}}],"sort":"-year,+album","limitPercent":10}"#), "Songs where Album contains Blue and BPM less than 100 and not in Jazz Mix, sorted by Year (newest first), then Album (A–Z), top 10%");
        assert_eq!(line(r#"{"all":[]}"#), "");
    }

    #[test]
    fn the_list_names_each_rule_and_the_order() {
        let s = summarize_criteria(r#"{"all":[{"inTheLast":{"lastplayed":40}},{"notInPlaylist":{"id":"p1"}},{"isNot":{"releasecountry":"MA"}}],"sort":"-playcount","limit":300}"#, Say(&Names)).unwrap();
        assert_eq!(s.rules.len(), 3);
        assert!(s.rules[1].ends_with("Jazz Mix"));
        assert_eq!(s.order, vec!["Sorted by Play Count (most played first)".to_string(), "Top 300 songs".to_string()]);
        let s = summarize_criteria(r#"{"is":{"genre":"Jazz"}}"#, Say(&English)).unwrap();
        assert_eq!((s.rules.len(), s.match_word.as_str()), (1, ""));
        assert!(summarize_criteria("not json", Say(&English)).is_none());
    }
}
