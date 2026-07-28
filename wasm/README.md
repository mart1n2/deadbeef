# WebAssembly `0xdeadbeef`

The `deadbeef` tool packaged for the Web using WebAssembly.

## Requirements

- [`wasm-opt` (Binaryen)](https://github.com/WebAssembly/binaryen)
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/)
- [`deno`](https://deno.land/manual/getting_started/installation)

## Building

As the build process is non-trivial, a `Makefile` is provided.
To build the WebAssembly package, just run:

```sh
make
```

## Testing

Easy-peasy-lemon-squeezy:

```sh
make test
```

## Example

An example web page that computes a Safe creation transaction for a single owner is provided.
To check it out, navigate to [`http://localhost:8000/example`](http://localhost:8000/example) while running:

```sh
make host
```

The worker accepts optional prefix and suffix constraints; at least one must be
non-empty. Both are hexadecimal digit patterns, so odd-length values are
supported:

```ts
const worker = new DeadbeefWorker(config, "0xabc", "def");
const creation = await worker.wait();
```

For a suffix-only search, omit the second argument:

```ts
const worker = new DeadbeefWorker(config, undefined, "beef");
```
