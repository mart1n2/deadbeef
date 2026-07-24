mod chain;
mod deployment;

use self::chain::{Chain, Details, Explorer, Singleton};
use clap::Parser;
use deadbeef_core::{config, Address, Configuration, NonZeroAddress, Safe};
use hex::FromHexError;
use std::{
    num::NonZeroUsize,
    process,
    str::FromStr,
    sync::{atomic::AtomicU32, mpsc, Arc},
    thread,
};

/// Generate vanity addresses for Safe deployments.
#[derive(Clone, Parser)]
struct Args {
    /// The number of parallel threads to use. Defaults to the number of CPUs.
    #[arg(short = 'n', long, default_value_t = num_cpus::get())]
    threads: usize,

    /// Safe owners.
    ///
    /// Can be specified multiple times in order to specify multiple owners.
    /// They will be included in the provided order.
    #[arg(short, long = "owner", required = true, num_args = 1..)]
    owners: Vec<NonZeroAddress>,

    /// Owner signature threshold.
    #[arg(short, long, default_value_t = 1)]
    threshold: usize,

    /// The prefix to look for.
    ///
    /// Mutually exclusive with '--leading'; exactly one must be specified.
    #[arg(short, long)]
    prefix: Option<Hex>,

    /// Search for the maximum number of leading nibbles equal to the given hex
    /// character (e.g. '--leading 0' for as many leading zeros as possible).
    ///
    /// Unlike '--prefix', this runs continuously, printing each address that
    /// beats the previous record until interrupted (Ctrl-C).
    #[arg(short = 'L', long, conflicts_with = "prefix")]
    leading: Option<Nibble>,

    /// The chain ID to find a vanity Safe address for. If the chain is not
    /// supported, then all of '--proxy-factory', '--proxy-init-code', and
    /// '--singleton' must be specified.
    #[arg(short, long, default_value_t = Chain::ethereum())]
    chain: Chain,

    /// Override for the `SafeProxyFactory` address.
    #[arg(long)]
    proxy_factory: Option<NonZeroAddress>,

    /// Override for the `SafeProxy` init code.
    #[arg(long)]
    proxy_init_code: Option<Hex>,

    /// Override for the `Safe` singleton address.
    #[arg(long)]
    singleton: Option<NonZeroAddress>,

    /// Override for the `SafeL2` singleton address.
    ///
    /// For unsupported chains, if this is specified then `--safe-to-l2-setup`
    /// must also be specified.
    #[arg(long)]
    l2_singleton: Option<NonZeroAddress>,

    /// Override for the `SafeToL2Setup` address.
    ///
    /// Specifying the 0 address (or not specifying the contract address for
    /// unknown chains) will disable this feature (which is not recommended).
    ///
    /// For unsupported chains, if this is specified then `--l2-singleton` must
    /// also be specified.
    #[arg(long)]
    safe_to_l2_setup: Option<Address>,

    /// Override for the fallback handler address.
    #[arg(long)]
    fallback_handler: Option<NonZeroAddress>,

    /// Quiet mode.
    ///
    /// Only output the transaction calldata without any extra information.
    #[arg(short, long, conflicts_with = "params")]
    quiet: bool,

    /// Parameters mode.
    ///
    /// Only output the parameters for the calling the `createProxyWithNonce`
    /// function on the `SafeProxyFactory`.
    #[arg(short = 'P', long, conflicts_with = "quiet")]
    params: bool,

    /// Use the GPU to accelerate the search.
    ///
    /// Runs the search on a GPU via `wgpu` (Metal on Apple, Vulkan/DX12 on
    /// NVIDIA/AMD). Only available when built with `--features gpu`.
    #[cfg(feature = "gpu")]
    #[arg(long)]
    gpu: bool,
}

/// Helper type for parsing hexadecimal byte input from the command line.
#[derive(Clone)]
struct Hex(Vec<u8>);

impl Hex {
    fn cloned(&self) -> Vec<u8> {
        self.0.clone()
    }
}

impl FromStr for Hex {
    type Err = FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode(s.strip_prefix("0x").unwrap_or(s)).map(Hex)
    }
}

/// A single hexadecimal digit (nibble), used for the leading-character search.
///
/// Accepts one hex digit (`0`, `a`) or several *identical* ones (`aaaa`,
/// `0x0000`) — the latter is just a more emphatic way of writing the former,
/// since the leading search maximises a single repeated character.
#[derive(Clone, Copy)]
struct Nibble(u8);

impl FromStr for Nibble {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        let mut digits = s.chars().map(|c| c.to_digit(16));
        let first = match digits.next() {
            Some(Some(d)) => d,
            _ => return Err(format!("expected one or more hex digits, got {s:?}")),
        };
        for digit in digits {
            match digit {
                Some(d) if d == first => {}
                Some(_) => {
                    return Err(
                        "'--leading' takes a single repeated hex digit (e.g. 0 or aaaa); \
                         use '--prefix' for mixed patterns"
                            .to_string(),
                    )
                }
                None => return Err(format!("invalid hex digit in {s:?}")),
            }
        }
        Ok(Nibble(first as u8))
    }
}

fn main() {
    let args = Args::parse();

    let threads = NonZeroUsize::new(args.threads);
    let chain = args.chain.details();
    let config = chain
        .as_ref()
        .map(|details| {
            let contracts = details.deployment();
            let setup = args
                .safe_to_l2_setup
                .unwrap_or(contracts.safe_to_l2_setup)
                .non_zero()
                .map(|address| config::SafeToL2Setup {
                    address,
                    l2_singleton: args.l2_singleton.unwrap_or(contracts.safe_l2),
                });

            Configuration {
                proxy: config::Proxy {
                    factory: args.proxy_factory.unwrap_or(contracts.safe_proxy_factory),
                    init_code: args
                        .proxy_init_code
                        .as_ref()
                        .map(Hex::cloned)
                        .unwrap_or(contracts.safe_proxy_init_code.to_vec()),
                    singleton: args.singleton.unwrap_or_else(|| {
                        match (&setup, details.singleton()) {
                            // If we are using the `SafeToL2Setup`, then always
                            // use the `Safe` singleton.
                            (Some(_), _) => contracts.safe,
                            (_, Singleton::Safe) => contracts.safe,
                            (_, Singleton::SafeL2) => contracts.safe_l2,
                        }
                    }),
                },
                account: config::Account {
                    owners: args.owners.clone(),
                    threshold: args.threshold,
                    setup,
                    fallback_handler: args
                        .fallback_handler
                        .or_else(|| contracts.compatibility_fallback_handler.non_zero()),
                    identifier: None,
                },
            }
        })
        .or_else(|| {
            Some(Configuration {
                proxy: config::Proxy {
                    factory: args.proxy_factory?,
                    init_code: args.proxy_init_code?.cloned(),
                    singleton: args.singleton?,
                },
                account: config::Account {
                    owners: args.owners.clone(),
                    threshold: args.threshold,
                    setup: match (
                        args.safe_to_l2_setup.and_then(Address::non_zero),
                        args.l2_singleton,
                    ) {
                        (None, None) => None,
                        // For unsupported chains, if either `SafeToL2Setup` or
                        // `SafeL2` is specified, then both must be specified.
                        (safe_to_l2_setup, l2_singleton) => Some(config::SafeToL2Setup {
                            address: safe_to_l2_setup?,
                            l2_singleton: l2_singleton?,
                        }),
                    },
                    fallback_handler: args.fallback_handler,
                    identifier: None,
                },
            })
        })
        .expect("unsupported chain");
    let explorer = chain.as_ref().map(Details::explorer);

    match (args.prefix.clone(), args.leading) {
        // Prefix search: find one matching address, then print it.
        (Some(prefix), None) => {
            let prefix = prefix.0;
            #[cfg(feature = "gpu")]
            let safe = if args.gpu {
                run_gpu_search(Safe::new(config.clone()), &prefix)
            } else {
                run_cpu_search(threads, || (Safe::new(config.clone()), prefix.clone()))
            };
            #[cfg(not(feature = "gpu"))]
            let safe = run_cpu_search(threads, || (Safe::new(config.clone()), prefix.clone()));

            print_result(args.quiet, args.params, &config, explorer, &safe);
        }
        // Leading search: run continuously, printing each new record.
        (None, Some(nibble)) => {
            let nibble = nibble.0;
            let (quiet, params) = (args.quiet, args.params);
            eprintln!("searching for the most leading {nibble:x} nibbles (press Ctrl-C to stop)");
            let report = |count: u32, safe: &Safe| {
                eprintln!("--- new record: {count} leading {nibble:x} nibble(s) ---");
                print_result(quiet, params, &config, explorer, safe);
            };

            #[cfg(feature = "gpu")]
            if args.gpu {
                run_gpu_leading(Safe::new(config.clone()), nibble, report);
            } else {
                run_cpu_leading(threads, &config, nibble, report);
            }
            #[cfg(not(feature = "gpu"))]
            run_cpu_leading(threads, &config, nibble, report);
        }
        (None, None) => {
            eprintln!("error: one of '--prefix' or '--leading' must be specified");
            process::exit(2);
        }
        // Prevented by clap's `conflicts_with`.
        (Some(_), Some(_)) => unreachable!(),
    }

    process::exit(0);
}

/// Prints the full deployment details for a found Safe, honouring the
/// `--quiet` and `--params` output modes.
fn print_result(
    quiet: bool,
    params: bool,
    config: &Configuration,
    explorer: Option<&Explorer>,
    safe: &Safe,
) {
    let transaction = safe.transaction();

    if quiet {
        println!("0x{}", hex::encode(&transaction.calldata));
    } else if params {
        let factory = explorer
            .map(|explorer| explorer.create_proxy_with_nonce_url(config.proxy.factory.get()))
            .unwrap_or_else(|| config.proxy.factory.to_string());

        println!("address:     {}", safe.creation_address());
        println!("factory:     {}", factory);
        println!("singleton:   {}", config.proxy.singleton);
        println!("initializer: 0x{}", hex::encode(safe.initializer()));
        println!("salt nonce:  0x{}", hex::encode(safe.salt_nonce()));
    } else {
        let (to, data) = config
            .account
            .setup
            .as_ref()
            .map(|setup| (setup.address.get(), setup.encode()))
            .unwrap_or_default();
        let fallback = config
            .account
            .fallback_handler
            .map(NonZeroAddress::get)
            .unwrap_or_default();

        println!("address:     {}", safe.creation_address());
        println!("factory:     {}", config.proxy.factory);
        println!("singleton:   {}", config.proxy.singleton);
        println!("initializer: 0x{}", hex::encode(safe.initializer()));
        println!("salt nonce:  0x{}", hex::encode(safe.salt_nonce()));
        println!("---");
        println!("owners:      {}", config.account.owners[0]);
        for owner in &config.account.owners[1..] {
            println!("             {}", owner);
        }
        println!("threshold:   {}", config.account.threshold);
        println!("to:          {}", to);
        println!("data:        0x{}", hex::encode(&data));
        println!("fallback:    {}", fallback);
        println!("---");
        println!("calldata:    0x{}", hex::encode(&transaction.calldata));
    }
}

/// Runs the search across CPU threads (or on the current thread when
/// `threads` is `None`), returning the Safe with its winning salt nonce set.
fn run_cpu_search(threads: Option<NonZeroUsize>, setup: impl Fn() -> (Safe, Vec<u8>)) -> Safe {
    if let Some(threads) = threads {
        let (sender, receiver) = mpsc::channel();
        let _threads = (0..threads.get())
            .map(|_| {
                thread::spawn({
                    let (mut safe, prefix) = setup();
                    let result = sender.clone();
                    move || {
                        deadbeef_core::search(&mut safe, &prefix);
                        let _ = result.send(safe);
                    }
                })
            })
            .collect::<Vec<_>>();
        receiver.recv().expect("missing result")
    } else {
        let (mut safe, prefix) = setup();
        deadbeef_core::search(&mut safe, &prefix);
        safe
    }
}

/// Runs a continuous leading-nibble search across CPU threads, invoking
/// `report` for each new record. Runs until the process is interrupted.
fn run_cpu_leading(
    threads: Option<NonZeroUsize>,
    config: &Configuration,
    nibble: u8,
    mut report: impl FnMut(u32, &Safe),
) {
    let best = Arc::new(AtomicU32::new(0));
    let (sender, receiver) = mpsc::channel();
    let count = threads.map(NonZeroUsize::get).unwrap_or(1);
    for _ in 0..count {
        let mut safe = Safe::new(config.clone());
        let best = Arc::clone(&best);
        let sender = sender.clone();
        thread::spawn(move || {
            deadbeef_core::search_leading(&mut safe, nibble, &best, |count, safe| {
                let _ = sender.send((count, safe.clone()));
            });
        });
    }
    drop(sender);

    // Blocks forever receiving each new record (worker threads never finish).
    for (count, safe) in receiver {
        report(count, &safe);
    }
}

/// Runs the search on the GPU, rendering a live progress line to stderr, and
/// returns the Safe with its winning salt nonce set.
#[cfg(feature = "gpu")]
fn run_gpu_search(mut safe: Safe, prefix: &[u8]) -> Safe {
    use std::io::{IsTerminal, Write};

    let gpu = deadbeef_gpu::Gpu::new().unwrap_or_else(|err| {
        eprintln!("error: {err}");
        process::exit(1);
    });

    let mut stderr = std::io::stderr();
    let tty = stderr.is_terminal();

    // Expected number of attempts to match the prefix (256 possibilities/byte).
    let expected = 256_f64.powi(prefix.len() as i32);

    eprintln!(
        "searching on GPU: {} ({} nonces/dispatch, ~{} expected)",
        gpu.adapter_name(),
        human(gpu.batch_size() as f64),
        human(expected),
    );

    gpu.search(&mut safe, prefix, |progress| {
        if !tty {
            return;
        }
        // Time-to-match is memoryless, so the expected remaining time stays
        // `expected / hashrate`.
        let eta = if progress.hashrate > 0.0 {
            expected / progress.hashrate
        } else {
            f64::INFINITY
        };
        let _ = write!(
            stderr,
            "\r\x1b[K{} hash/s | tried {} | {} | ETA ~{}",
            human(progress.hashrate),
            human(progress.attempts as f64),
            fmt_duration(progress.elapsed.as_secs_f64()),
            fmt_duration(eta),
        );
        let _ = stderr.flush();
    });

    if tty {
        let _ = writeln!(stderr);
    }
    safe
}

/// Runs a continuous leading-nibble search on the GPU, rendering a live
/// progress line to stderr and invoking `report` for each new record. Runs
/// until the process is interrupted.
#[cfg(feature = "gpu")]
fn run_gpu_leading(mut safe: Safe, nibble: u8, mut report: impl FnMut(u32, &Safe)) {
    use std::io::{IsTerminal, Write};

    let gpu = deadbeef_gpu::Gpu::new().unwrap_or_else(|err| {
        eprintln!("error: {err}");
        process::exit(1);
    });

    let mut stderr = std::io::stderr();
    let tty = stderr.is_terminal();

    eprintln!(
        "searching on GPU: {} ({} nonces/dispatch)",
        gpu.adapter_name(),
        human(gpu.batch_size() as f64),
    );

    gpu.search_leading(
        &mut safe,
        nibble,
        |count, safe| {
            // Finalise the in-place progress line before printing a record.
            if tty {
                let _ = writeln!(std::io::stderr());
            }
            report(count, safe);
        },
        |progress| {
            if !tty {
                return;
            }
            let _ = write!(
                stderr,
                "\r\x1b[K{} hash/s | tried {} | {}",
                human(progress.hashrate),
                human(progress.attempts as f64),
                fmt_duration(progress.elapsed.as_secs_f64()),
            );
            let _ = stderr.flush();
        },
    );
}

/// Formats a number with a decimal SI-style suffix (e.g. `42.7M`).
#[cfg(feature = "gpu")]
fn human(n: f64) -> String {
    const UNITS: [&str; 6] = ["", "K", "M", "G", "T", "P"];
    let mut value = n;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// Formats a duration in seconds as a compact human-readable string.
#[cfg(feature = "gpu")]
fn fmt_duration(secs: f64) -> String {
    if !secs.is_finite() {
        return "long".to_string();
    }
    let secs = secs as u64;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d{:02}h", secs / 86400, (secs % 86400) / 3600)
    }
}
