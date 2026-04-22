use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::proof::SelectedCandle;

pub fn select_candle(body_json: &Value) -> Result<SelectedCandle> {
    let candles = candle_array(body_json)?;
    let candle = candles
        .first()
        .ok_or_else(|| anyhow!("Coinbase response did not include any candles"))?;
    parse_candle_object(candle)
}

pub fn contains_selected_candle(body_json: &Value, selected: &SelectedCandle) -> Result<bool> {
    for candle in candle_array(body_json)? {
        if parse_candle_object(candle)? == *selected {
            return Ok(true);
        }
    }
    Ok(false)
}

fn candle_array(body_json: &Value) -> Result<&Vec<Value>> {
    body_json
        .get("candles")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Coinbase candle body must contain a candles array"))
}

fn parse_candle_object(candle: &Value) -> Result<SelectedCandle> {
    let object = candle
        .as_object()
        .ok_or_else(|| anyhow!("Coinbase candle row must be an object"))?;
    let field = |name: &str| {
        object
            .get(name)
            .ok_or_else(|| anyhow!("Coinbase candle omitted {name}"))
    };

    let selected = SelectedCandle {
        time: json_i64(field("start")?).with_context(|| "invalid candle start")?,
        low: json_decimal_string(field("low")?).with_context(|| "invalid candle low")?,
        high: json_decimal_string(field("high")?).with_context(|| "invalid candle high")?,
        open: json_decimal_string(field("open")?).with_context(|| "invalid candle open")?,
        close: json_decimal_string(field("close")?).with_context(|| "invalid candle close")?,
        volume: json_decimal_string(field("volume")?).with_context(|| "invalid candle volume")?,
    };

    validate_positive_decimal("low", &selected.low)?;
    validate_positive_decimal("high", &selected.high)?;
    validate_positive_decimal("open", &selected.open)?;
    validate_positive_decimal("close", &selected.close)?;
    validate_non_negative_decimal("volume", &selected.volume)?;
    Ok(selected)
}

fn json_i64(value: &Value) -> Result<i64> {
    if let Some(n) = value.as_i64() {
        return Ok(n);
    }
    if let Some(s) = value.as_str() {
        return s
            .parse::<i64>()
            .with_context(|| format!("invalid integer string: {s}"));
    }
    Err(anyhow!("expected integer or integer string"))
}

fn json_decimal_string(value: &Value) -> Result<String> {
    match value {
        Value::Number(number) => Ok(number.to_string()),
        Value::String(value) if !value.trim().is_empty() => Ok(value.clone()),
        _ => Err(anyhow!("expected JSON number or decimal string")),
    }
}

fn validate_positive_decimal(field: &str, value: &str) -> Result<()> {
    let parsed = value
        .parse::<f64>()
        .with_context(|| format!("{field} is not numeric: {value}"))?;
    if parsed > 0.0 && parsed.is_finite() {
        Ok(())
    } else {
        Err(anyhow!("{field} must be positive"))
    }
}

fn validate_non_negative_decimal(field: &str, value: &str) -> Result<()> {
    let parsed = value
        .parse::<f64>()
        .with_context(|| format!("{field} is not numeric: {value}"))?;
    if parsed >= 0.0 && parsed.is_finite() {
        Ok(())
    } else {
        Err(anyhow!("{field} must be non-negative"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selects_matching_candle() {
        let body = json!({
            "candles": [
                {
                    "start": "1000",
                    "low": "10.1",
                    "high": "11.2",
                    "open": "10.5",
                    "close": "11.0",
                    "volume": "12.3"
                },
                {
                    "start": "700",
                    "low": "9.0",
                    "high": "10.0",
                    "open": "9.5",
                    "close": "9.7",
                    "volume": "2.0"
                }
            ]
        });
        let selected = select_candle(&body).unwrap();
        assert_eq!(selected.time, 1000);
        assert_eq!(selected.low, "10.1");
        assert_eq!(selected.close, "11.0");
        assert!(contains_selected_candle(&body, &selected).unwrap());
    }

    #[test]
    fn rejects_empty_candles() {
        let body = json!({ "candles": [] });
        assert!(select_candle(&body).is_err());
    }

    #[test]
    fn rejects_malformed_row() {
        let body = json!({ "candles": [{ "start": "1000", "low": "9.0" }] });
        assert!(select_candle(&body).is_err());
    }
}
