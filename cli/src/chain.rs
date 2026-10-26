//! Module for chain-specific data.

use crate::deployment::{self, Deployment};
use deadbeef_core::Address;
use std::{
    fmt::{self, Display, Formatter},
    num::ParseIntError,
    str::FromStr,
};

/// A Safe supported chain.
#[derive(Clone, Copy, Debug)]
pub struct Chain(u128);

impl Chain {
    /// Returns a [`Chain`] for Ethereum Mainnet.
    pub const fn ethereum() -> Self {
        Self(1)
    }

    /// Returns the [`Details`] for this chain using the requested Safe
    /// version, or the chain's default deployment when no version is given.
    ///
    /// Returns [`None`] if the chain is not officially supported, and an error
    /// if the requested version is not deployed on the chain.
    pub fn details_for(&self, version: Option<SafeVersion>) -> Result<Option<Details>, String> {
        let Some(details) = self.details() else {
            return match version {
                Some(version) => Err(format!(
                    "'--safe-version {version}' requires a supported chain, but chain {self} \
                     is not supported; specify the contract addresses manually instead"
                )),
                None => Ok(None),
            };
        };
        let Some(version) = version else {
            return Ok(Some(details));
        };
        if !self.has(version) {
            return Err(format!("Safe v{version} is not deployed on chain {self}"));
        }
        Ok(Some(Details {
            deployment: version.deployment(),
            ..details
        }))
    }

    /// Returns whether the canonical deployment of the given Safe version is
    /// available on this chain, according to the Safe deployments repository:
    /// <https://github.com/safe-global/safe-deployments/tree/main/src/assets>
    fn has(&self, version: SafeVersion) -> bool {
        match version {
            SafeVersion::V1_4_1 => !matches!(self.0, 324 | 10200),
            SafeVersion::V1_5_0 => !matches!(self.0, 324 | 1101 | 10200 | 81457),
        }
    }

    /// Returns the [`Details`] for this chain, or [`None`] if the chain is
    /// not officially supported.
    pub fn details(&self) -> Option<Details> {
        match self.0 {
            1 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://etherscan.io"),
                singleton: Singleton::Safe,
            }),
            10 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://optimistic.etherscan.io"),
                singleton: Singleton::SafeL2,
            }),
            56 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://bscscan.com"),
                singleton: Singleton::SafeL2,
            }),
            100 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://gnosisscan.io"),
                singleton: Singleton::SafeL2,
            }),
            130 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://uniscan.xyz"),
                singleton: Singleton::SafeL2,
            }),
            137 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://polygonscan.com"),
                singleton: Singleton::SafeL2,
            }),
            146 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://sonicscan.org"),
                singleton: Singleton::SafeL2,
            }),
            196 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer {
                    url: "https://www.oklink.com/xlayer",
                    selector: "/contract#category=write&id=2",
                },
                singleton: Singleton::SafeL2,
            }),
            324 => todo!("zkSync Era is currently not supported."),
            480 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::blockscout("https://worldchain-mainnet.explorer.alchemy.com"),
                singleton: Singleton::SafeL2,
            }),
            1101 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://zkevm.polygonscan.com"),
                singleton: Singleton::SafeL2,
            }),
            5000 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://mantlescan.xyz"),
                singleton: Singleton::SafeL2,
            }),
            8453 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://basescan.org"),
                singleton: Singleton::SafeL2,
            }),
            10200 => Some(Details {
                deployment: &deployment::v1_3_0::CANONICAL,
                explorer: Explorer::blockscout("https://gnosis-chiado.blockscout.com"),
                singleton: Singleton::SafeL2,
            }),
            42161 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://arbiscan.io"),
                singleton: Singleton::SafeL2,
            }),
            42220 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::blockscout("https://explorer.celo.org/mainnet"),
                singleton: Singleton::SafeL2,
            }),
            43114 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer {
                    url: "https://snowtrace.io",
                    selector: "/contract/43114/writeContract?chainid=43114#F3",
                },
                singleton: Singleton::SafeL2,
            }),
            57073 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::blockscout("https://explorer.inkonchain.com"),
                singleton: Singleton::SafeL2,
            }),
            59144 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://lineascan.build"),
                singleton: Singleton::SafeL2,
            }),
            80094 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://berascan.com"),
                singleton: Singleton::SafeL2,
            }),
            81457 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://blastscan.io"),
                singleton: Singleton::SafeL2,
            }),
            84532 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://sepolia.basescan.org"),
                singleton: Singleton::SafeL2,
            }),
            534352 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://scrollscan.com"),
                singleton: Singleton::SafeL2,
            }),
            11155111 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::etherscan("https://sepolia.etherscan.io"),
                singleton: Singleton::SafeL2,
            }),
            1313161554 => Some(Details {
                deployment: &deployment::v1_4_1::CANONICAL,
                explorer: Explorer::blockscout("https://aurorascan.dev"),
                singleton: Singleton::SafeL2,
            }),
            _ => None,
        }
    }
}

impl Default for Chain {
    fn default() -> Self {
        Self::ethereum()
    }
}

impl Display for Chain {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Chain {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "eth" => return Ok(Self(1)),
            "oeth" => return Ok(Self(10)),
            "bnb" => return Ok(Self(56)),
            "gno" => return Ok(Self(100)),
            "unichain" => return Ok(Self(130)),
            "matic" => return Ok(Self(137)),
            "sonic" => return Ok(Self(146)),
            "xlayer" => return Ok(Self(196)),
            "zksync" => return Ok(Self(324)),
            "wc" => return Ok(Self(480)),
            "zkevm" => return Ok(Self(1101)),
            "mnt" => return Ok(Self(5000)),
            "base" => return Ok(Self(8453)),
            "chiado" => return Ok(Self(10200)),
            "arb1" => return Ok(Self(42161)),
            "celo" => return Ok(Self(42220)),
            "avax" => return Ok(Self(43114)),
            "ink" => return Ok(Self(57073)),
            "linea" => return Ok(Self(59144)),
            "berachain" => return Ok(Self(80094)),
            "blast" => return Ok(Self(81457)),
            "basesep" => return Ok(Self(84532)),
            "scr" => return Ok(Self(534352)),
            "sep" => return Ok(Self(11155111)),
            "aurora" => return Ok(Self(1313161554)),
            _ => {}
        };

        let (s, radix) = match s.strip_prefix("0x") {
            Some(s) => (s, 16),
            None => (s, 10),
        };
        let value = u128::from_str_radix(s, radix)?;

        Ok(Self(value))
    }
}

/// The chain details.
pub struct Details {
    deployment: &'static Deployment,
    explorer: Explorer,
    singleton: Singleton,
}

impl Details {
    /// Returns the deployment information for the chain.
    pub fn deployment(&self) -> &Deployment {
        self.deployment
    }

    /// Returns the explorer URL to the `createProxyWithNonce` function.
    pub fn explorer(&self) -> &Explorer {
        &self.explorer
    }

    /// Returns the singleton contract kind for the chain.
    pub fn singleton(&self) -> Singleton {
        self.singleton
    }
}

/// The explorer.
pub struct Explorer {
    url: &'static str,
    selector: &'static str,
}

impl Explorer {
    /// Create a new Etherscan-like explorer instance.
    fn etherscan(url: &'static str) -> Self {
        Self {
            url,
            selector: "#writeContract#F3",
        }
    }

    /// Create a new Blockscout-like explorer instance.
    fn blockscout(url: &'static str) -> Self {
        Self {
            url,
            selector: "?tab=read_write_contract#0x1688f0b9",
        }
    }

    /// Returns the explorer URL to the `createProxyWithNonce` function.
    pub fn create_proxy_with_nonce_url(&self, proxy_factory: Address) -> String {
        format!("{}/address/{}{}", self.url, proxy_factory, self.selector)
    }
}

/// A supported Safe contract version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum SafeVersion {
    /// Safe v1.4.1.
    #[value(name = "1.4.1")]
    V1_4_1,
    /// Safe v1.5.0.
    #[value(name = "1.5.0")]
    V1_5_0,
}

impl SafeVersion {
    /// Returns the canonical contract deployment for this version.
    pub fn deployment(self) -> &'static Deployment {
        match self {
            Self::V1_4_1 => &deployment::v1_4_1::CANONICAL,
            Self::V1_5_0 => &deployment::v1_5_0::CANONICAL,
        }
    }
}

impl Display for SafeVersion {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::V1_4_1 => f.write_str("1.4.1"),
            Self::V1_5_0 => f.write_str("1.5.0"),
        }
    }
}

/// The supported singleton contract.
#[derive(Clone, Copy, Debug)]
pub enum Singleton {
    /// The `Safe` singleton contract.
    Safe,
    /// The `SafeL2` singleton contract.
    SafeL2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_proxy_with_nonce_urls() {
        for chain in [
            1, 10, 56, 100, 130, 137, 146, 196, /*324,*/ 480, 1101, 5000, 8453, 10200, 42161,
            42220, 43114, 57073, 59144, 80094, 81457, 84532, 534352, 11155111, 1313161554,
        ] {
            let details = Chain(chain).details().unwrap();
            let url = details
                .explorer
                .create_proxy_with_nonce_url(details.deployment.safe_proxy_factory.get());
            println!("{}: {}", chain, url);
        }
    }

    #[test]
    fn safe_version_selection() {
        let arbitrum = Chain(42161);
        let factory = |version| {
            arbitrum
                .details_for(version)
                .unwrap()
                .unwrap()
                .deployment
                .safe_proxy_factory
        };
        assert_eq!(
            factory(None),
            deployment::v1_4_1::CANONICAL.safe_proxy_factory
        );
        assert_eq!(
            factory(Some(SafeVersion::V1_5_0)),
            deployment::v1_5_0::CANONICAL.safe_proxy_factory
        );

        // Chiado only has v1.3.0, which stays its default.
        let chiado = Chain(10200);
        assert_eq!(
            chiado.details_for(None).unwrap().unwrap().deployment.safe,
            deployment::v1_3_0::CANONICAL.safe
        );
        assert!(chiado.details_for(Some(SafeVersion::V1_4_1)).is_err());

        // Chains without the requested deployment and unknown chains are
        // rejected.
        assert!(Chain(81457).details_for(Some(SafeVersion::V1_5_0)).is_err());
        assert!(Chain(1337).details_for(Some(SafeVersion::V1_5_0)).is_err());
        assert!(Chain(1337).details_for(None).unwrap().is_none());
    }
}
