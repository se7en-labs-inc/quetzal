//! Exact natural-number decoding, accepting integral JSON decimals and exponents.
use serde::Deserialize;

/// Haskell's scientific JSON representation omits a zero fractional part.
pub fn serialize_float<S: serde::Serializer>(
    value: &f64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if let Some(integer) = parse_u64(&value.to_string()) {
        serializer.serialize_u64(integer)
    } else {
        use serde::Serialize;
        let text = if value.abs() < 0.1 {
            format!("{value:e}")
        } else {
            value.to_string()
        };
        let number: serde_json::Number = text.parse().map_err(serde::ser::Error::custom)?;
        number.serialize(serializer)
    }
}

pub fn parse_u64(text: &str) -> Option<u64> {
    let (negative, text) = text
        .strip_prefix('-')
        .map_or((false, text), |text| (true, text));
    let (mantissa, exponent) = text.find(['e', 'E']).map_or((text, "0"), |position| {
        (&text[..position], &text[position + 1..])
    });
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let mut digits = mantissa.replace('.', "");
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if digits.bytes().all(|byte| byte == b'0') {
        return Some(0);
    }
    if negative {
        return None;
    }
    let mut scale = exponent
        .parse::<i64>()
        .ok()?
        .checked_sub(i64::try_from(fraction).ok()?)?;
    while scale < 0 && digits.ends_with('0') {
        digits.pop();
        scale += 1;
    }
    if !(0..=19).contains(&scale) {
        return None;
    }
    let mut number = digits.trim_start_matches('0').parse::<u64>().ok()?;
    for _ in 0..scale {
        number = number.checked_mul(10)?;
    }
    Some(number)
}
pub fn deserialize_u64<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    parse_u64(&number.to_string()).ok_or_else(|| {
        serde::de::Error::custom("expected a natural number within the unsigned 64-bit range")
    })
}

/// Decode a Cardano coin quantity, including the ledger's maximum supply.
pub fn deserialize_coin<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    let text = number.to_string();
    if let Some(value) = parse_u64(&text).filter(|value| *value <= 45_000_000_000_000_000) {
        return Ok(value);
    }
    let reason = if let Some(absolute) = text.strip_prefix('-') {
        match parse_u64(absolute) {
            Some(magnitude) => {
                format!("parsing Natural failed, unexpected negative number -{magnitude}")
            }
            None if text.contains('.') => {
                format!("parsing Natural failed, unexpected floating number {text}")
            }
            None => format!("parsing Natural failed, unexpected negative number {text}"),
        }
    } else if text.contains('.') && parse_u64(&text).is_none() {
        format!("parsing Natural failed, unexpected floating number {text}")
    } else {
        "invalid coin value: value has to be lower than or equal to 45000000000000000 lovelace."
            .into()
    };
    Err(serde::de::Error::custom(format!(
        "parsing AddressAmount failed, {reason}"
    )))
}
pub fn deserialize_u32<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    parse_u64(&number.to_string())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            serde::de::Error::custom(format!(
                "parsing Integer failed, unexpected floating number {number}"
            ))
        })
}
