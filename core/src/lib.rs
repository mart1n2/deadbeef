#[macro_use]
mod address;
pub mod config;
mod create2;
mod keccak;
mod pattern;
mod safe;

pub use self::{
    address::{Address, NonZeroAddress},
    config::{Configuration, ConfigurationError},
    pattern::{AddressPattern, PatternError},
    safe::{Safe, Transaction},
};
pub use hex_literal::hex;
use rand::{rngs::SmallRng, Rng as _, SeedableRng as _};
use std::sync::atomic::{AtomicU32, Ordering};

/// Search for a vanity address with the specified Safe parameters and prefix.
pub fn search(safe: &mut Safe, prefix: &[u8]) {
    let pattern = AddressPattern::from_prefix_bytes(prefix).expect("invalid vanity address prefix");
    search_pattern(safe, &pattern);
}

/// Search for a vanity address satisfying the precompiled address `pattern`.
pub fn search_pattern(safe: &mut Safe, pattern: &AddressPattern) {
    let mut nonces = NonceSequence::new();
    while !search_iter_pattern(safe, pattern, |nonce| nonces.write_next(nonce)) {}
}

/// Run a single iteration of the vanity address search.
///
/// This function is publically exposed to facilitate benchmarking.
#[doc(hidden)]
pub fn search_iter(safe: &mut Safe, prefix: &[u8], update: impl FnOnce(&mut [u8; 32])) -> bool {
    assert!(!prefix.is_empty() && prefix.len() <= 20, "invalid prefix");
    safe.update_salt_nonce(update);
    safe.creation_address().0.starts_with(prefix)
}

/// Run one search iteration against a precompiled address pattern.
#[doc(hidden)]
pub fn search_iter_pattern(
    safe: &mut Safe,
    pattern: &AddressPattern,
    update: impl FnOnce(&mut [u8; 32]),
) -> bool {
    safe.update_salt_nonce(update);
    pattern.matches(safe.creation_address())
}

/// Counts the number of leading nibbles (hex digits) of `address` equal to
/// `nibble`, which must be in `0..=15`.
pub fn leading_count(address: Address, nibble: u8) -> u32 {
    let mut count = 0;
    for byte in address.0 {
        if byte >> 4 != nibble {
            break;
        }
        count += 1;
        if byte & 0xf != nibble {
            break;
        }
        count += 1;
    }
    count
}

/// Searches for salt nonces that maximise the number of leading `nibble`
/// nibbles in the Safe address.
///
/// Whenever an address is found with strictly more leading nibbles than `best`
/// (a counter that may be shared across threads), `best` is raised and
/// `improved` is invoked with the new count and the `safe` (with its winning
/// salt nonce set). This never returns; run it until the process is stopped.
pub fn search_leading(
    safe: &mut Safe,
    nibble: u8,
    best: &AtomicU32,
    mut improved: impl FnMut(u32, &Safe),
) {
    assert!(nibble <= 0xf, "leading nibble must be in 0..=15");
    let mut nonces = NonceSequence::new();
    loop {
        safe.update_salt_nonce(|nonce| nonces.write_next(nonce));
        let count = leading_count(safe.creation_address(), nibble);

        // Only report (and only once) if we are the thread that raised the
        // shared best to this count.
        let mut current = best.load(Ordering::Relaxed);
        while count > current {
            match best.compare_exchange_weak(current, count, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    improved(count, safe);
                    break;
                }
                Err(actual) => current = actual,
            }
        }
    }
}

/// A random 192-bit search domain plus a sequential 64-bit counter.
///
/// Keccak makes sequential nonces just as suitable for vanity search as fully
/// random nonces, while incrementing a counter avoids generating 32 random
/// bytes for every candidate.
struct NonceSequence {
    nonce: [u8; 32],
    counter: u64,
}

impl NonceSequence {
    fn new() -> Self {
        let mut nonce = [0; 32];
        SmallRng::from_os_rng().fill(&mut nonce);
        let counter = u64::from_le_bytes(nonce[..8].try_into().unwrap());
        Self { nonce, counter }
    }

    #[inline]
    fn write_next(&mut self, destination: &mut [u8; 32]) {
        self.nonce[..8].copy_from_slice(&self.counter.to_le_bytes());
        self.counter = self.counter.wrapping_add(1);
        destination.copy_from_slice(&self.nonce);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_leading_nibbles() {
        let addr = "0x000abc0000000000000000000000000000000000"
            .parse::<Address>()
            .unwrap();
        assert_eq!(leading_count(addr, 0), 3);

        let addr = "0xaaaa000000000000000000000000000000000000"
            .parse::<Address>()
            .unwrap();
        assert_eq!(leading_count(addr, 0xa), 4);

        assert_eq!(leading_count(Address::zero(), 0), 40);

        let addr = "0x1000000000000000000000000000000000000000"
            .parse::<Address>()
            .unwrap();
        assert_eq!(leading_count(addr, 0), 0);
    }
}
