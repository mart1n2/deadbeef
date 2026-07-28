use crate::Address;
use std::{
    error::Error,
    fmt::{self, Display, Formatter},
};

const ADDRESS_NIBBLES: usize = 40;

/// A precompiled set of hexadecimal constraints on an Ethereum address.
///
/// The pattern is stored as a byte-level value and mask so the search hot path
/// can check prefixes and suffixes with one fixed-width, branch-free loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressPattern {
    value: [u8; 20],
    mask: [u8; 20],
    constrained_nibbles: u32,
}

impl AddressPattern {
    /// Compiles optional hexadecimal prefix and suffix constraints.
    ///
    /// Both affixes may include an optional `0x` prefix and may contain an odd
    /// number of hexadecimal digits. Compatible overlap is allowed.
    pub fn from_affixes(prefix: Option<&str>, suffix: Option<&str>) -> Result<Self, PatternError> {
        let prefix = normalize(prefix.unwrap_or_default());
        let suffix = normalize(suffix.unwrap_or_default());

        if prefix.len() > ADDRESS_NIBBLES {
            return Err(PatternError::TooLong {
                affix: "prefix",
                digits: prefix.len(),
            });
        }
        if suffix.len() > ADDRESS_NIBBLES {
            return Err(PatternError::TooLong {
                affix: "suffix",
                digits: suffix.len(),
            });
        }

        let mut pattern = Self {
            value: [0; 20],
            mask: [0; 20],
            constrained_nibbles: 0,
        };

        for (index, digit) in prefix.bytes().enumerate() {
            pattern.set_nibble(index, decode(digit, "prefix", index)?)?;
        }
        let suffix_start = ADDRESS_NIBBLES - suffix.len();
        for (index, digit) in suffix.bytes().enumerate() {
            pattern.set_nibble(suffix_start + index, decode(digit, "suffix", index)?)?;
        }

        if pattern.constrained_nibbles == 0 {
            return Err(PatternError::Empty);
        }
        Ok(pattern)
    }

    /// Compiles a byte-aligned prefix for compatibility with the original API.
    pub fn from_prefix_bytes(prefix: &[u8]) -> Result<Self, PatternError> {
        if prefix.is_empty() {
            return Err(PatternError::Empty);
        }
        if prefix.len() > 20 {
            return Err(PatternError::TooLong {
                affix: "prefix",
                digits: prefix.len() * 2,
            });
        }

        let mut value = [0; 20];
        let mut mask = [0; 20];
        value[..prefix.len()].copy_from_slice(prefix);
        mask[..prefix.len()].fill(0xff);
        Ok(Self {
            value,
            mask,
            constrained_nibbles: (prefix.len() * 2) as u32,
        })
    }

    /// Returns whether `address` satisfies all compiled constraints.
    #[inline]
    pub fn matches(&self, address: Address) -> bool {
        let mut difference = 0_u8;
        for index in 0..20 {
            difference |= (address.0[index] ^ self.value[index]) & self.mask[index];
        }
        difference == 0
    }

    /// The precompiled target bytes.
    pub fn value(&self) -> [u8; 20] {
        self.value
    }

    /// The precompiled byte masks.
    pub fn mask(&self) -> [u8; 20] {
        self.mask
    }

    /// Number of independently constrained hexadecimal digits.
    pub fn constrained_nibbles(&self) -> u32 {
        self.constrained_nibbles
    }

    fn set_nibble(&mut self, position: usize, digit: u8) -> Result<(), PatternError> {
        let byte = position / 2;
        let shift = if position.is_multiple_of(2) { 4 } else { 0 };
        let nibble_mask = 0x0f << shift;
        let nibble_value = digit << shift;

        if self.mask[byte] & nibble_mask != 0 {
            if self.value[byte] & nibble_mask != nibble_value {
                return Err(PatternError::Conflict { position });
            }
            return Ok(());
        }

        self.mask[byte] |= nibble_mask;
        self.value[byte] |= nibble_value;
        self.constrained_nibbles += 1;
        Ok(())
    }
}

fn normalize(value: &str) -> &str {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
}

fn decode(digit: u8, affix: &'static str, index: usize) -> Result<u8, PatternError> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        b'A'..=b'F' => Ok(digit - b'A' + 10),
        _ => Err(PatternError::InvalidHex {
            affix,
            index,
            digit: digit as char,
        }),
    }
}

/// An invalid address pattern.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatternError {
    /// Neither affix contains a hexadecimal constraint.
    Empty,
    /// An affix is longer than an Ethereum address.
    TooLong { affix: &'static str, digits: usize },
    /// An affix contains a non-hexadecimal character.
    InvalidHex {
        affix: &'static str,
        index: usize,
        digit: char,
    },
    /// Overlapping prefix and suffix constraints disagree.
    Conflict { position: usize },
}

impl Display for PatternError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("at least one non-empty prefix or suffix is required"),
            Self::TooLong { affix, digits } => write!(
                f,
                "{affix} has {digits} hex digits; an address has only {ADDRESS_NIBBLES}"
            ),
            Self::InvalidHex {
                affix,
                index,
                digit,
            } => write!(f, "invalid hex digit {digit:?} at {affix} position {index}"),
            Self::Conflict { position } => write!(
                f,
                "prefix and suffix conflict at address hex position {position}"
            ),
        }
    }
}

impl Error for PatternError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_prefix_and_suffix_at_nibble_granularity() {
        let pattern = AddressPattern::from_affixes(Some("0xabc"), Some("Def")).unwrap();
        let matching = "0xabc0000000000000000000000000000000000def"
            .parse()
            .unwrap();
        let wrong_prefix = "0xabb0000000000000000000000000000000000def"
            .parse()
            .unwrap();
        let wrong_suffix = "0xabc0000000000000000000000000000000000dee"
            .parse()
            .unwrap();

        assert!(pattern.matches(matching));
        assert!(!pattern.matches(wrong_prefix));
        assert!(!pattern.matches(wrong_suffix));
        assert_eq!(pattern.constrained_nibbles(), 6);
    }

    #[test]
    fn accepts_compatible_overlap_and_rejects_conflicts() {
        let exact = "1234567890abcdef1234567890abcdef12345678";
        let compatible = AddressPattern::from_affixes(Some(exact), Some("f12345678")).unwrap();
        assert_eq!(compatible.constrained_nibbles(), 40);

        assert_eq!(
            AddressPattern::from_affixes(Some(exact), Some("012345678")),
            Err(PatternError::Conflict { position: 31 })
        );
    }

    #[test]
    fn rejects_empty_invalid_and_oversized_patterns() {
        assert_eq!(
            AddressPattern::from_affixes(Some("0x"), None),
            Err(PatternError::Empty)
        );
        assert!(matches!(
            AddressPattern::from_affixes(Some("xyz"), None),
            Err(PatternError::InvalidHex { .. })
        ));
        assert!(matches!(
            AddressPattern::from_affixes(Some("00000000000000000000000000000000000000000"), None),
            Err(PatternError::TooLong { .. })
        ));
        assert!(matches!(
            AddressPattern::from_prefix_bytes(&[0; 21]),
            Err(PatternError::TooLong { .. })
        ));
    }
}
