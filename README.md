# `0xdeadbeef`

Tool used for computing vanity Safe addresses.

This tool only officially supports the latest Safe deployment [`v1.4.1`](https://github.com/safe-global/safe-deployments/tree/main/src/assets/v1.4.1).

Since this version of the Safe proxy factory uses the `CREATE2` op-code, we can change the final address by fiddling with the user-specified `saltNonce` parameter.
It works by trying out different values for the `saltNonce` parameter until it finds one that creates an address matching what you want.

Features:

- **Prefix and suffix search** — find an address matching either or both
  hexadecimal affixes (`--prefix 0x5afe --suffix beef`). Odd-length patterns
  are supported.
- **Leading-character search** — find an address with as many leading occurrences of a
  single hex character as possible (`--leading 0`), e.g. for gas-optimized leading-zero
  addresses.
- **Multi-threaded CPU** search by default, or optional **GPU acceleration** (`--gpu`) that
  runs on Apple M-series, NVIDIA and AMD GPUs from a single codebase.

## Building

For longer patterns, this can take a **very** long time, so be sure to build with release:

```sh
cargo build --release
```

### GPU acceleration

The search can optionally run on a GPU, which is dramatically faster than the CPU for longer
patterns. It is built on [`wgpu`](https://github.com/gfx-rs/wgpu), so a single WGSL compute
shader runs across backends:

| Platform | Backend |
| --- | --- |
| Apple M-series | Metal |
| NVIDIA / AMD / Intel (Linux) | Vulkan |
| Windows | DirectX 12 |

GPU support is behind an opt-in feature flag, so the default build stays lean and free of the
`wgpu` dependency:

```sh
cargo build --release --features gpu
```

Then pass `--gpu` to run the search on the GPU (see [Usage](#usage) below). A live progress
line (hash rate, attempts, elapsed, ETA) is printed to stderr while it runs:

```
searching on GPU: Apple M1 (67.1M nonces/dispatch, ~16.8M expected)
34.2M hash/s | tried 1.0G | 29s | ETA ~1s
```

Every match found on the GPU is re-verified on the CPU before being reported, so a shader bug
can never produce a wrong address.

## Usage

```sh
deadbeef --help
```

For example, to generate calldata for creating a Safe with initial owners of `0x1111111111111111111111111111111111111111` and `0x2222222222222222222222222222222222222222` and prefix `0x5afe`:

```sh
deadbeef \
  --owner 0x1111111111111111111111111111111111111111 \
  --owner 0x2222222222222222222222222222222222222222 \
  --prefix 0x5afe
```

`--prefix` and `--suffix` may also be used independently. They match
hexadecimal digits rather than whole bytes, so odd-length values such as
`--prefix 0 --suffix fff` are valid. A prefix and suffix may overlap when their
overlapping digits agree; conflicting or longer-than-address patterns are
rejected before the search starts.

This will output some result like:

```
address:     0x5AFE941f405085500803E7479cEcEC46cB50B70A
factory:     0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67
singleton:   0x41675C099F32341bf84BFc5382aF534df5C7461a
initializer: 0xb63e800d00000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000bd89a1ce4dde368ffab0ec35506eece0b1ffdc540000000000000000000000000000000000000000000000000000000000000160000000000000000000000000fd0732dc9e303f09fcef3a7388ad10a83459ec990000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000111111111111111111111111111111111111111100000000000000000000000022222222222222222222222222222222222222220000000000000000000000000000000000000000000000000000000000000024fe51f64300000000000000000000000029fcb43b46531bca003ddc8fcb67ffe91900c76200000000000000000000000000000000000000000000000000000000
salt nonce:  0xdc50d6fbe25fc5fe0cffb1a2fc5170b76ac620f74f22c7b247d8c73d83072302
---
owners:      0x1111111111111111111111111111111111111111
             0x2222222222222222222222222222222222222222
threshold:   1
to:          0xBD89A1CE4DDe368FFAB0eC35506eEcE0b1fFdc54
data:        0xfe51f64300000000000000000000000029fcb43b46531bca003ddc8fcb67ffe91900c762
fallback:    0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99
---
calldata:    0x1688f0b900000000000000000000000041675c099f32341bf84bfc5382af534df5c7461a0000000000000000000000000000000000000000000000000000000000000060dc50d6fbe25fc5fe0cffb1a2fc5170b76ac620f74f22c7b247d8c73d8307230200000000000000000000000000000000000000000000000000000000000001c4b63e800d00000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000bd89a1ce4dde368ffab0ec35506eece0b1ffdc540000000000000000000000000000000000000000000000000000000000000160000000000000000000000000fd0732dc9e303f09fcef3a7388ad10a83459ec990000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000111111111111111111111111111111111111111100000000000000000000000022222222222222222222222222222222222222220000000000000000000000000000000000000000000000000000000000000024fe51f64300000000000000000000000029fcb43b46531bca003ddc8fcb67ffe91900c7620000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
```

To require both affixes:

```sh
deadbeef ... --prefix dead --suffix beef
```

Note that the owner signature threshold defaults to 1 but can optionally be specified with:

```sh
deadbeef ... --threshold 2 ...
```

For using Safe deployments on different chains can also be used:

```sh
deadbeef ... --chain 100 ...
```

As well as custom fallback handlers:

```sh
deadbeef ... --fallback-handler 0x4e305935b14627eA57CBDbCfF57e81fd9F240403 ...
```

By default, the generated initializer will use the `SafeToL2Setup` contract. This ensures that the Safe deployment transaction can be replayed to get the same address on all supported chains. In order to disable this behaviour (not recommended), set the `--safe-to-l2-setup` flag to the 0 address:

```sh
deadbeef ... --safe-to-l2-setup 0x0000000000000000000000000000000000000000 ...
```

When built with the `gpu` feature (see [GPU acceleration](#gpu-acceleration)), the search can be run on a GPU:

```sh
deadbeef ... --gpu ...
```

### Maximizing leading characters

Instead of fixed affixes, you can search for an address with **as many leading occurrences of a single hex character as possible** (similar to [profanity2's `--leading`](https://github.com/1inch/profanity2#prefix---leading)). This is ideal for e.g. leading-zero (gas-optimized) addresses:

```sh
deadbeef --owner 0x1111111111111111111111111111111111111111 --leading 0
```

Unlike `--prefix`, this runs continuously and prints a new result **every time it beats the previous record** (e.g. 6, then 7, then 8 leading zeros...). Each printed result is a complete, ready-to-use deployment, so just keep the last (best) one and stop the search with `Ctrl-C` whenever you are satisfied. Combine with `--gpu` to find records much faster.

The argument is a single hex digit; a repeated digit like `0x0000` is accepted and treated the same as `0` (`--prefix` is the tool for mixed patterns). `--leading` is mutually exclusive with both `--prefix` and `--suffix`.

## Creating the Safe

The above command will generate some [calldata](https://www.quicknode.com/guides/ethereum-development/transactions/ethereum-transaction-calldata) for creating a Safe with the specified owners and threshold.

To create the safe, simply execute a transaction to the [factory address](https://etherscan.io/address/0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67) with the generated calldata, or use the `createProxyWithNonce` function on Etherscan.
The transaction can be executed from any account (it can be done in MetaMask directly for example).

### Metamask Steps

Go to Settings -> Advanced and enable `Show hex data`. When you go to create a transaction you will have a new optional field labelled `Hex data`.

Send a 0Ξ transaction to the factory address, placing the generated calldata in the `Hex data` field.

Metamask will recognise it as a contract interaction in the confirmation step.

### Etherscan

Use the `--params` flag to output contract-ready inputs.

1. Visit the `factory` URL from the command output. The link should open up the explorer at the correct location, if not click on _Contract_ > _Write Contract_.
2. Click on _Connect to Web3_ to connect the account you wish to pay for the Safe creation.
3. Fill the fields for the function _3. createProxyWithNonce (0x1688f0b9)_ using the generated outputs.

## Unsupported Chains

Safe deployments on non-officially supported networks can also be used by overriding all contract addresses and the proxy init code:

```sh
deadbeef ... \
  --chain $UNSUPPORTED_CHAIN \
  --proxy-factory 0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --proxy-init-code 0xbb \
  --singleton 0xcccccccccccccccccccccccccccccccccccccccc
```

**Use this with caution**, this assumes that the proxy address is computed in the exact same was as on Ethereum, which may not be the case for all networks.
This feature is not officially supported by the tool.

## Is This Vegan Friendly 🥦?

Of course!
No actual cows were harmed in the creation or continual use of this tool.

```sh
alias deadbeef=seedfeed
```
