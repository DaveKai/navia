//! Checks smart playlist rules in Navidrome's JSON before anything is sent.
//! The server's parser accepts some shapes that fail later, so this checks the shape and leaves field names open.

use serde_json::{Map, Value};

/// Navidrome's operators, lowercased as its parser matches them.
const OPERATORS: &[&str] = &[
    "is", "isnot", "gt", "lt", "contains", "notcontains", "startswith", "endswith", "intherange", "before", "after", "inthelast", "notinthelast", "inplaylist", "notinplaylist", "ismissing", "ispresent",
];
/// Nesting deeper than this is refused. No editor writes anything close.
const MAX_DEPTH: usize = 16;
/// Rules in one playlist, counting nested ones.
const MAX_RULES: usize = 1000;
/// Longest field name accepted. Navidrome's are short words, and custom tags are tag names.
const MAX_FIELD: usize = 128;

/// Why rules can't be saved. The text is for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RulesProblem(pub String);

fn problem(text: impl Into<String>) -> RulesProblem {
    RulesProblem(text.into())
}

/// Checks a criteria object and returns it ready to send.
/// Whole numbers written as decimals become integers. Unknown top-level keys pass through unchanged.
pub fn checked_criteria(text: &str) -> Result<Value, RulesProblem> {
    let Ok(Value::Object(mut top)) = serde_json::from_str::<Value>(text) else { return Err(problem("The rules aren't in Navidrome's format.")) };
    let group = match (top.contains_key("all"), top.contains_key("any")) {
        (true, true) => return Err(problem("Rules can match all or any at the top, not both.")),
        (false, false) => return Err(problem("Add a rule to save")),
        (true, false) => "all",
        (false, true) => "any",
    };
    let mut count = 0;
    match &top[group] {
        Value::Array(list) if !list.is_empty() => check_list(list, 1, &mut count)?,
        Value::Array(_) => return Err(problem("Add a rule to save")),
        _ => return Err(problem("The rules aren't in Navidrome's format.")),
    }
    if let Some(sort) = top.get("sort") {
        check_sort(sort)?;
    }
    if let Some(order) = top.get("order") {
        let ok = order.as_str().is_some_and(|o| o.is_empty() || o.eq_ignore_ascii_case("asc") || o.eq_ignore_ascii_case("desc"));
        if !ok {
            return Err(problem("The order has to be ascending or descending."));
        }
    }
    whole_number(&mut top, "limit", u32::MAX as i64, "The song limit has to be a whole number.")?;
    whole_number(&mut top, "limitPercent", 100, "The limit has to be a percentage from 0 to 100.")?;
    whole_number(&mut top, "offset", u32::MAX as i64, "The offset has to be a whole number.")?;
    if top.get("refreshDelay").is_some_and(|d| !d.is_string()) {
        return Err(problem("The refresh delay isn't in Navidrome's format."));
    }
    Ok(Value::Object(top))
}

fn check_list(list: &[Value], depth: usize, count: &mut usize) -> Result<(), RulesProblem> {
    if depth > MAX_DEPTH {
        return Err(problem("The rules are nested too deeply."));
    }
    for item in list {
        let Value::Object(expression) = item else { return Err(problem("The rules aren't in Navidrome's format.")) };
        if expression.is_empty() {
            return Err(problem("One rule is empty."));
        }
        for (key, body) in expression {
            *count += 1;
            if *count > MAX_RULES {
                return Err(problem("There are too many rules."));
            }
            let name = key.to_ascii_lowercase();
            if name == "all" || name == "any" {
                match body {
                    Value::Array(inner) if !inner.is_empty() => check_list(inner, depth + 1, count)?,
                    _ => return Err(problem("A group of rules is empty.")),
                }
            } else if OPERATORS.contains(&name.as_str()) {
                check_rule(&name, body)?;
            } else {
                return Err(problem(format!("Navidrome doesn't know the condition \u{201c}{key}\u{201d}.")));
            }
        }
    }
    Ok(())
}

/// Checks one rule's body, which needs exactly one field and a value of the form the operator takes.
fn check_rule(op: &str, body: &Value) -> Result<(), RulesProblem> {
    let Value::Object(fields) = body else { return Err(problem("The rules aren't in Navidrome's format.")) };
    let mut it = fields.iter();
    let (Some((field, value)), None) = (it.next(), it.next()) else { return Err(problem("Each rule needs exactly one field.")) };
    if field.trim().is_empty() || field.len() > MAX_FIELD || field.chars().any(char::is_control) {
        return Err(problem("A rule has no field."));
    }
    let scalar = |v: &Value| matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_));
    let fine = match op {
        "intherange" => value.as_array().is_some_and(|pair| pair.len() == 2 && pair.iter().all(scalar)),
        "inthelast" | "notinthelast" => match value {
            Value::Number(n) => n.as_f64().is_some_and(|d| d >= 0.0),
            Value::String(s) => s.trim().parse::<f64>().is_ok_and(|d| d >= 0.0),
            _ => false,
        },
        "inplaylist" | "notinplaylist" => field.eq_ignore_ascii_case("id") && value.as_str().is_some_and(|s| !s.trim().is_empty()),
        "ismissing" | "ispresent" => scalar(value),
        _ => scalar(value),
    };
    if fine { Ok(()) } else { Err(problem(format!("The rule on \u{201c}{field}\u{201d} has a value Navidrome can't use."))) }
}

fn check_sort(sort: &Value) -> Result<(), RulesProblem> {
    let Some(text) = sort.as_str() else { return Err(problem("The sort order isn't in Navidrome's format.")) };
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let name = part.trim_start_matches(['+', '-']).trim();
        if name.is_empty() || name.len() > MAX_FIELD || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(problem(format!("Navidrome can't sort by \u{201c}{part}\u{201d}.")));
        }
    }
    Ok(())
}

/// Leaves `key` absent or makes it a whole number from 0 to `max`, turning `50.0` into `50`.
fn whole_number(top: &mut Map<String, Value>, key: &str, max: i64, message: &str) -> Result<(), RulesProblem> {
    let Some(value) = top.get(key) else { return Ok(()) };
    let n = match value {
        Value::Null => {
            top.remove(key);
            return Ok(());
        }
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 1e15).map(|f| f as i64)),
        _ => None,
    };
    match n {
        Some(n) if (0..=max).contains(&n) => {
            top.insert(key.to_string(), Value::from(n));
            Ok(())
        }
        _ => Err(problem(message)),
    }
}

/// A playlist name the server will take, trimmed and not empty.
pub fn checked_name(name: &str) -> Result<String, RulesProblem> {
    let name = name.trim();
    if name.is_empty() { Err(problem("Add a name to save")) } else { Ok(name.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok(v: Value) -> Value {
        checked_criteria(&v.to_string()).unwrap()
    }
    fn bad(v: Value) -> String {
        checked_criteria(&v.to_string()).unwrap_err().0
    }

    #[test]
    fn navidromes_own_examples_pass_unchanged() {
        let v = json!({"all": [{"is": {"loved": true}}, {"inTheLast": {"lastPlayed": 30}}, {"any": [{"contains": {"genre": "rock"}}, {"inTheRange": {"year": [1980, 1989]}}]}], "sort": "-playCount,title", "order": "desc", "limit": 50, "refreshDelay": "1h"});
        assert_eq!(ok(v.clone()), v);
        assert_eq!(ok(json!({"any": [{"inPlaylist": {"id": "abc"}}, {"isMissing": {"bpm": true}}]})), json!({"any": [{"inPlaylist": {"id": "abc"}}, {"isMissing": {"bpm": true}}]}));
    }

    #[test]
    fn whole_numbers_written_as_decimals_become_integers() {
        let v = ok(json!({"all": [{"gt": {"rating": 3.5}}], "limit": 25.0, "limitPercent": 10.0}));
        assert_eq!(v["limit"], json!(25));
        assert_eq!(v["limitPercent"], json!(10));
        assert_eq!(v["all"][0]["gt"]["rating"], json!(3.5), "values inside rules stay as they are");
        assert!(ok(json!({"all": [{"is": {"loved": true}}], "limit": null})).get("limit").is_none());
    }

    #[test]
    fn shapes_navidrome_refuses_or_breaks_on_are_caught() {
        assert!(bad(json!({"all": [{"is": {"loved": true}}], "any": []})).contains("not both"));
        assert!(bad(json!({"sort": "title"})).contains("Add a rule"));
        assert!(bad(json!({"all": []})).contains("Add a rule"));
        assert!(bad(json!({"all": [{"is": {"title": "a", "album": "b"}}]})).contains("exactly one field"));
        assert!(bad(json!({"all": [{"matches": {"title": "a"}}]})).contains("matches"));
        assert!(bad(json!({"all": [{"inTheRange": {"year": [1980]}}]})).contains("year"));
        assert!(bad(json!({"all": [{"inTheLast": {"lastPlayed": "soon"}}]})).contains("lastPlayed"));
        assert!(bad(json!({"all": [{"inPlaylist": {"name": "x"}}]})).contains("name"));
        assert!(bad(json!({"all": [{"is": {"title": null}}]})).contains("title"));
        assert!(bad(json!({"all": [{"is": {"": "x"}}]})).contains("no field"));
        assert!(bad(json!({"all": [{"any": []}]})).contains("group"));
        assert!(bad(json!({"all": [{"is": {"loved": true}}], "limit": 2.5})).contains("whole number"));
        assert!(bad(json!({"all": [{"is": {"loved": true}}], "limit": -1})).contains("whole number"));
        assert!(bad(json!({"all": [{"is": {"loved": true}}], "limitPercent": 150})).contains("percentage"));
        assert!(bad(json!({"all": [{"is": {"loved": true}}], "order": "up"})).contains("order"));
        assert!(bad(json!({"all": [{"is": {"loved": true}}], "sort": "title;drop"})).contains("sort"));
        assert!(checked_criteria("not json").is_err());
        assert!(checked_criteria("[]").is_err());
    }

    #[test]
    fn deep_nesting_is_refused() {
        let mut v = json!({"is": {"loved": true}});
        for _ in 0..20 {
            v = json!({"all": [v]});
        }
        assert!(bad(v).contains("deeply"));
    }

    #[test]
    fn names_are_trimmed_and_required() {
        assert_eq!(checked_name("  Mix ").unwrap(), "Mix");
        assert!(checked_name("   ").is_err());
    }
}
