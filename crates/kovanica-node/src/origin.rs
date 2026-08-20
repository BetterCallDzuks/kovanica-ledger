//! Geographic origin of a node — ISO 3166-1 alpha-3, **node policy**.
//!
//! This is how Kovanica tracks where users (nodes) come from. It is deliberately
//! **not** part of consensus: origin never enters a block's canonical encoding,
//! never affects GHOSTDAG colouring or linearization, and two honest nodes that
//! disagree on a peer's origin still agree on the DAG. Observed pulses are a
//! local view, recorded when a peer announces itself (in-process gossip today;
//! a continuous p2p overlay would carry the same announcement).
//!
//! Named after the same provenance idea as the KovanicaDAG origins map: a node
//! sets its own country, and peers accumulate pulse counts so operators can see
//! where the network is coming from.

use core::fmt;

/// Why parsing an origin failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginError {
    /// Not exactly three ASCII letters (ISO 3166-1 alpha-3).
    Invalid,
}

impl fmt::Display for OriginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OriginError::Invalid => {
                f.write_str("origin must be a 3-letter ISO 3166-1 alpha-3 code (e.g. HRV)")
            }
        }
    }
}

impl std::error::Error for OriginError {}

/// An ISO 3166-1 alpha-3 country code (uppercase ASCII).
///
/// Any three letters A–Z are accepted — we do not ship a full country table, so
/// a typo like `HHR` is a valid code at this layer. Consensus never sees this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin([u8; 3]);

impl Origin {
    /// Parse a 3-letter country code, case-insensitively.
    pub fn parse(s: &str) -> Result<Self, OriginError> {
        let bytes = s.as_bytes();
        if bytes.len() != 3 {
            return Err(OriginError::Invalid);
        }
        let mut out = [0u8; 3];
        for (i, b) in bytes.iter().enumerate() {
            let c = b.to_ascii_uppercase();
            if !c.is_ascii_alphabetic() {
                return Err(OriginError::Invalid);
            }
            out[i] = c;
        }
        Ok(Origin(out))
    }

    /// The three uppercase ASCII letters.
    pub fn as_bytes(&self) -> &[u8; 3] {
        &self.0
    }

    /// The code as a `&str` (always valid ASCII).
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).unwrap_or("???")
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uppercases_and_rejects_junk() {
        assert_eq!(Origin::parse("hrv").unwrap().as_str(), "HRV");
        assert_eq!(Origin::parse("USA").unwrap().as_str(), "USA");
        assert_eq!(Origin::parse("hr").unwrap_err(), OriginError::Invalid);
        assert_eq!(Origin::parse("HRVV").unwrap_err(), OriginError::Invalid);
        assert_eq!(Origin::parse("12A").unwrap_err(), OriginError::Invalid);
        assert_eq!(Origin::parse("").unwrap_err(), OriginError::Invalid);
    }

    #[test]
    fn ordering_is_byte_order() {
        let a = Origin::parse("HRV").unwrap();
        let b = Origin::parse("USA").unwrap();
        assert!(a < b);
    }
}
