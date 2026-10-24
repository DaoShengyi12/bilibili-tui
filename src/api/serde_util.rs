//! Deserialization helpers for drifting Bilibili payloads.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Deserializer};

/// Treat an explicit `null` like a missing field. Pair with `#[serde(default)]`.
pub fn null_as_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Read a flag sent as a bool, a number, a numeric string or `null`.
/// Pair with `#[serde(default)]`.
pub fn lenient_flag<'de, D>(deserializer: D) -> std::result::Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Bool(value) => value,
        serde_json::Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        serde_json::Value::String(value) => !matches!(value.trim(), "" | "0" | "false"),
        _ => false,
    })
}

/// Decode a list item by item, skipping entries that no longer match `T`.
///
/// A missing or `null` list is empty. A list whose every entry fails is an
/// error, so a wholesale format change is reported instead of shown as empty.
pub fn parse_lenient_list<T>(value: Option<&serde_json::Value>, what: &str) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let items = match value {
        None | Some(serde_json::Value::Null) => return Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items,
        Some(_) => return Err(anyhow!("{what} is not a list")),
    };

    let mut first_error = None;
    let parsed: Vec<T> = items
        .iter()
        .filter_map(|item| {
            T::deserialize(item)
                .map_err(|error| {
                    first_error.get_or_insert(error);
                })
                .ok()
        })
        .collect();

    match first_error {
        Some(error) if parsed.is_empty() => Err(anyhow!("invalid {what} entries: {error}")),
        _ => Ok(parsed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lenient_flag_accepts_bools_numbers_strings_and_null() {
        #[derive(Deserialize)]
        struct Flag {
            #[serde(default, deserialize_with = "lenient_flag")]
            value: bool,
        }
        let parse = |value: serde_json::Value| {
            serde_json::from_value::<Flag>(json!({ "value": value }))
                .unwrap()
                .value
        };
        assert!(parse(json!(true)));
        assert!(parse(json!(1)));
        assert!(parse(json!("1")));
        assert!(!parse(json!(0)));
        assert!(!parse(json!("0")));
        assert!(!parse(json!(null)));
        assert!(!serde_json::from_value::<Flag>(json!({})).unwrap().value);
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Item {
        id: i64,
    }

    #[derive(Debug, Deserialize)]
    struct Wrapper {
        #[serde(default, deserialize_with = "null_as_default")]
        list: Vec<Item>,
    }

    #[test]
    fn null_list_decodes_as_empty() {
        let wrapper: Wrapper = serde_json::from_value(json!({"list": null})).unwrap();
        assert!(wrapper.list.is_empty());
        let wrapper: Wrapper = serde_json::from_value(json!({})).unwrap();
        assert!(wrapper.list.is_empty());
    }

    #[test]
    fn lenient_list_skips_drifted_entries() {
        let value = json!([{"id": 1}, {"id": "bad"}, {"id": 3}]);
        let items: Vec<Item> = parse_lenient_list(Some(&value), "items").unwrap();
        assert_eq!(items, vec![Item { id: 1 }, Item { id: 3 }]);
    }

    #[test]
    fn lenient_list_reports_wholesale_drift() {
        let value = json!([{"id": "bad"}]);
        let error = parse_lenient_list::<Item>(Some(&value), "items").unwrap_err();
        assert!(error.to_string().contains("invalid items entries"));
        let error = parse_lenient_list::<Item>(Some(&json!({"id": 1})), "items").unwrap_err();
        assert!(error.to_string().contains("not a list"));
    }

    #[test]
    fn lenient_list_treats_missing_as_empty() {
        assert!(
            parse_lenient_list::<Item>(None, "items")
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_lenient_list::<Item>(Some(&serde_json::Value::Null), "items")
                .unwrap()
                .is_empty()
        );
    }
}
