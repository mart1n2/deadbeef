#[macro_use]
mod address;
pub mod config;
mod create2;
mod keccak;
mod safe;

pub use self::{
    address::{Address, NonZeroAddress},
    config::Configuration,
    safe::{Safe, Transaction},
};
pub use hex_literal::hex;
use rand::{rngs::SmallRng, Rng as _, SeedableRng as _};
use std::sync::atomic::{AtomicU32, Ordering};

/// Search for a vanity address with the specified Safe parameters and prefix.
pub fn search(safe: &mut Safe, prefix: &[u8]) {
    let mut rng = SmallRng::from_os_rng();
    while !search_iter(safe, prefix, |n| rng.fill(n)) {}
}

/// Run a single iteration of the vanity address search.
///
/// This function is publically exposed to facilitate benchmarking.
#[doc(hidden)]
pub fn search_iter(safe: &mut Safe, prefix: &[u8], update: impl FnOnce(&mut [u8; 32])) -> bool {
    safe.update_salt_nonce(update);
    safe.creation_address().0.starts_with(prefix)
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
    let mut rng = SmallRng::from_os_rng();
    loop {
        safe.update_salt_nonce(|n| rng.fill(n));
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
