//! Preserve exact 64-bit seeds through JSON/WebView IPC. Existing numeric files remain readable.
//! Small seeds keep their historic numeric representation; large seeds use decimal text.
use serde::{de::Error, Deserialize, Deserializer, Serializer};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub fn serialize<S: Serializer>(seed: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    if *seed <= MAX_SAFE_INTEGER {
        serializer.serialize_u64(*seed)
    } else {
        serializer.serialize_str(&seed.to_string())
    }
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        Number(u64),
        Decimal(String),
    }
    match Input::deserialize(deserializer)? {
        Input::Number(seed) => Ok(seed),
        Input::Decimal(text)
            if !text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit()) =>
        {
            text.parse().map_err(D::Error::custom)
        }
        Input::Decimal(_) => Err(D::Error::custom("seed must be an unsigned decimal integer")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, Serialize, Deserialize)]
    struct Receipt {
        #[serde(with = "super")]
        seed: u64,
    }

    #[test]
    fn preserves_legacy_and_maximum_seeds_through_webview_json() {
        for seed in [0, 42, MAX_SAFE_INTEGER, MAX_SAFE_INTEGER + 1, u64::MAX] {
            let legacy = format!(r#"{{"seed":{seed}}}"#);
            let receipt: Receipt = serde_json::from_str(&legacy).unwrap();
            let wire = serde_json::to_value(&receipt).unwrap();
            if seed > MAX_SAFE_INTEGER {
                assert_eq!(wire["seed"], seed.to_string());
            } else {
                assert_eq!(wire["seed"].as_u64(), Some(seed));
            }
            let restored: Receipt = serde_json::from_value(wire).unwrap();
            assert_eq!(restored.seed, seed);
        }
        assert!(serde_json::from_str::<Receipt>(r#"{"seed":"1e6"}"#).is_err());
    }
}
