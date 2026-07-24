// Vanity Safe address search kernel.
//
// Each invocation tries one candidate `saltNonce` and computes the resulting
// `CREATE2` address by replaying the exact two-hash pipeline used on the CPU
// (see `deadbeef-core`):
//
//   1. create2_salt = keccak256(salt_prefix[32] ++ nonce[32])
//   2. address      = keccak256(0xff ++ factory[20] ++ create2_salt[32]
//                                    ++ init_code_hash[32])[12..32]
//
// then checks whether `address` starts with the requested prefix and, on a
// match, records the winning nonce.
//
// WGSL has no 64-bit integer type, so each of the 25 Keccak lanes is stored as
// a `vec2<u32>` = (low 32 bits, high 32 bits). Messages are assembled at the
// u32-word level (little-endian); the 136-byte Keccak-256 rate block is 34
// words. Both messages here fit in a single rate block.

struct Params {
    // keccak256(initializer): constant 32-byte prefix of the hash-1 preimage.
    salt_prefix: array<u32, 8>,
    // Precomputed hash-2 rate block (0xff ++ factory ++ 0*32 ++ init_code_hash,
    // padded). The 32 bytes where create2_salt goes (byte offset 21..53) are
    // left zero and OR-injected per invocation below.
    hash2_base: array<u32, 34>,
    // Target address prefix, packed little-endian aligned to address byte 0.
    prefix: array<u32, 5>,
    // Number of prefix bytes to match (1..=20).
    prefix_len: u32,
    // Search mode: 0 = prefix match, 1 = maximise leading nibbles.
    mode: u32,
    // Leading mode: the target nibble (0..=15) to count.
    leading_nibble: u32,
    // Leading mode: minimum leading-nibble count worth reporting.
    threshold: u32,
    // Random per-dispatch nonce seed; word 0 is overwritten with the global id.
    nonce_base: array<u32, 8>,
}

struct SearchResult {
    found: atomic<u32>,
    // Leading mode: the leading-nibble count of the reported match.
    count: u32,
    nonce: array<u32, 8>,
}

const MODE_PREFIX: u32 = 0u;
const MODE_LEADING: u32 = 1u;

@group(0) @binding(0) var<storage, read> params: Params;
@group(0) @binding(1) var<storage, read_write> result: SearchResult;

// Nonces each invocation tries per dispatch. Must match `ITERS` on the host.
// Amortises kernel-launch and readback overhead over many hashes.
const ITERS: u32 = 64u;

// Keccak round constants, split into (low, high) 32-bit halves.
var<private> RC: array<vec2<u32>, 24> = array<vec2<u32>, 24>(
    vec2<u32>(0x00000001u, 0x00000000u),
    vec2<u32>(0x00008082u, 0x00000000u),
    vec2<u32>(0x0000808au, 0x80000000u),
    vec2<u32>(0x80008000u, 0x80000000u),
    vec2<u32>(0x0000808bu, 0x00000000u),
    vec2<u32>(0x80000001u, 0x00000000u),
    vec2<u32>(0x80008081u, 0x80000000u),
    vec2<u32>(0x00008009u, 0x80000000u),
    vec2<u32>(0x0000008au, 0x00000000u),
    vec2<u32>(0x00000088u, 0x00000000u),
    vec2<u32>(0x80008009u, 0x00000000u),
    vec2<u32>(0x8000000au, 0x00000000u),
    vec2<u32>(0x8000808bu, 0x00000000u),
    vec2<u32>(0x0000008bu, 0x80000000u),
    vec2<u32>(0x00008089u, 0x80000000u),
    vec2<u32>(0x00008003u, 0x80000000u),
    vec2<u32>(0x00008002u, 0x80000000u),
    vec2<u32>(0x00000080u, 0x80000000u),
    vec2<u32>(0x0000800au, 0x00000000u),
    vec2<u32>(0x8000000au, 0x80000000u),
    vec2<u32>(0x80008081u, 0x80000000u),
    vec2<u32>(0x00008080u, 0x80000000u),
    vec2<u32>(0x80000001u, 0x00000000u),
    vec2<u32>(0x80008008u, 0x80000000u),
);

// Rotate a 64-bit lane left by `n` bits (0 <= n < 64).
fn rotl64(v: vec2<u32>, n: u32) -> vec2<u32> {
    if (n == 0u) {
        return v;
    }
    if (n < 32u) {
        let lo = (v.x << n) | (v.y >> (32u - n));
        let hi = (v.y << n) | (v.x >> (32u - n));
        return vec2<u32>(lo, hi);
    }
    if (n == 32u) {
        return vec2<u32>(v.y, v.x);
    }
    let m = n - 32u;
    let lo = (v.y << m) | (v.x >> (32u - m));
    let hi = (v.x << m) | (v.y >> (32u - m));
    return vec2<u32>(lo, hi);
}

// One `chi` row: a[b0..b4] combined, given the five `b` lanes of the row.
fn chi_row(b0: vec2<u32>, b1: vec2<u32>, b2: vec2<u32>, b3: vec2<u32>, b4: vec2<u32>)
    -> array<vec2<u32>, 5> {
    return array<vec2<u32>, 5>(
        b0 ^ (~b1 & b2),
        b1 ^ (~b2 & b3),
        b2 ^ (~b3 & b4),
        b3 ^ (~b4 & b0),
        b4 ^ (~b0 & b1),
    );
}

// The Keccak-f[1600] permutation, in place. Fully unrolled with compile-time
// constant lane indices and rotation amounts so the 25 lanes stay in registers
// (no thread-local spills), which is essential for GPU throughput. Only the
// round loop remains, gathering a single round constant per iteration.
fn keccakf(a: ptr<function, array<vec2<u32>, 25>>) {
    var s = *a;
    for (var round = 0u; round < 24u; round = round + 1u) {
        // theta
        let c0 = s[0] ^ s[5] ^ s[10] ^ s[15] ^ s[20];
        let c1 = s[1] ^ s[6] ^ s[11] ^ s[16] ^ s[21];
        let c2 = s[2] ^ s[7] ^ s[12] ^ s[17] ^ s[22];
        let c3 = s[3] ^ s[8] ^ s[13] ^ s[18] ^ s[23];
        let c4 = s[4] ^ s[9] ^ s[14] ^ s[19] ^ s[24];
        let d0 = c4 ^ rotl64(c1, 1u);
        let d1 = c0 ^ rotl64(c2, 1u);
        let d2 = c1 ^ rotl64(c3, 1u);
        let d3 = c2 ^ rotl64(c4, 1u);
        let d4 = c3 ^ rotl64(c0, 1u);

        // rho + pi: b[dest] = rotl(a[src] ^ d[src%5], rho[src])
        var b: array<vec2<u32>, 25>;
        b[0] = rotl64(s[0] ^ d0, 0u);
        b[10] = rotl64(s[1] ^ d1, 1u);
        b[20] = rotl64(s[2] ^ d2, 62u);
        b[5] = rotl64(s[3] ^ d3, 28u);
        b[15] = rotl64(s[4] ^ d4, 27u);
        b[16] = rotl64(s[5] ^ d0, 36u);
        b[1] = rotl64(s[6] ^ d1, 44u);
        b[11] = rotl64(s[7] ^ d2, 6u);
        b[21] = rotl64(s[8] ^ d3, 55u);
        b[6] = rotl64(s[9] ^ d4, 20u);
        b[7] = rotl64(s[10] ^ d0, 3u);
        b[17] = rotl64(s[11] ^ d1, 10u);
        b[2] = rotl64(s[12] ^ d2, 43u);
        b[12] = rotl64(s[13] ^ d3, 25u);
        b[22] = rotl64(s[14] ^ d4, 39u);
        b[23] = rotl64(s[15] ^ d0, 41u);
        b[8] = rotl64(s[16] ^ d1, 45u);
        b[18] = rotl64(s[17] ^ d2, 15u);
        b[3] = rotl64(s[18] ^ d3, 21u);
        b[13] = rotl64(s[19] ^ d4, 8u);
        b[14] = rotl64(s[20] ^ d0, 18u);
        b[24] = rotl64(s[21] ^ d1, 2u);
        b[9] = rotl64(s[22] ^ d2, 61u);
        b[19] = rotl64(s[23] ^ d3, 56u);
        b[4] = rotl64(s[24] ^ d4, 14u);

        // chi (per row of 5 lanes)
        let r0 = chi_row(b[0], b[1], b[2], b[3], b[4]);
        let r1 = chi_row(b[5], b[6], b[7], b[8], b[9]);
        let r2 = chi_row(b[10], b[11], b[12], b[13], b[14]);
        let r3 = chi_row(b[15], b[16], b[17], b[18], b[19]);
        let r4 = chi_row(b[20], b[21], b[22], b[23], b[24]);
        s[0] = r0[0];
        s[1] = r0[1];
        s[2] = r0[2];
        s[3] = r0[3];
        s[4] = r0[4];
        s[5] = r1[0];
        s[6] = r1[1];
        s[7] = r1[2];
        s[8] = r1[3];
        s[9] = r1[4];
        s[10] = r2[0];
        s[11] = r2[1];
        s[12] = r2[2];
        s[13] = r2[3];
        s[14] = r2[4];
        s[15] = r3[0];
        s[16] = r3[1];
        s[17] = r3[2];
        s[18] = r3[3];
        s[19] = r3[4];
        s[20] = r4[0];
        s[21] = r4[1];
        s[22] = r4[2];
        s[23] = r4[3];
        s[24] = r4[4];

        // iota
        s[0] = s[0] ^ RC[round];
    }
    *a = s;
}

// Keccak-256 of a single, already-padded 136-byte rate block (34 words).
// Returns the first 32 output bytes as 8 little-endian words.
fn keccak256_block(block: array<u32, 34>) -> array<u32, 8> {
    var st: array<vec2<u32>, 25>;
    for (var i = 0u; i < 25u; i = i + 1u) {
        st[i] = vec2<u32>(0u, 0u);
    }
    // Absorb the 17-lane (136-byte) rate; the state starts zero so assignment
    // is equivalent to XOR.
    for (var i = 0u; i < 17u; i = i + 1u) {
        st[i] = vec2<u32>(block[2u * i], block[2u * i + 1u]);
    }
    keccakf(&st);

    var out: array<u32, 8>;
    for (var i = 0u; i < 4u; i = i + 1u) {
        out[2u * i] = st[i].x;
        out[2u * i + 1u] = st[i].y;
    }
    return out;
}

// Counts the leading nibbles of the address (digest bytes 12..32) equal to `t`.
fn count_leading(digest: array<u32, 8>, t: u32) -> u32 {
    var count = 0u;
    for (var i = 0u; i < 20u; i = i + 1u) {
        let byte = (digest[3u + i / 4u] >> (8u * (i % 4u))) & 0xffu;
        if (((byte >> 4u) & 0xfu) != t) {
            return count;
        }
        count = count + 1u;
        if ((byte & 0xfu) != t) {
            return count;
        }
        count = count + 1u;
    }
    return count;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let nfull = params.prefix_len / 4u;
    let rem = params.prefix_len % 4u;
    let mask = select(0u, (1u << (8u * rem)) - 1u, rem > 0u);

    // Assemble the constant part of the hash-1 rate block once. The candidate
    // nonce occupies words 8..16: word 8 = global id (distinct per invocation),
    // word 9 = the per-iteration counter, words 10..16 = the random seed.
    var b1: array<u32, 34>;
    for (var i = 0u; i < 34u; i = i + 1u) {
        b1[i] = 0u;
    }
    for (var i = 0u; i < 8u; i = i + 1u) {
        b1[i] = params.salt_prefix[i];
    }
    b1[8] = gid.x; // nonce word 0
    for (var j = 2u; j < 8u; j = j + 1u) {
        b1[8u + j] = params.nonce_base[j]; // nonce words 2..8
    }
    b1[16] = 0x00000001u; // keccak padding start (byte 64)
    b1[33] = 0x80000000u; // keccak padding end   (byte 135)

    // Each invocation tries ITERS nonces, varying nonce word 1.
    for (var iter = 0u; iter < ITERS; iter = iter + 1u) {
        b1[9] = iter; // nonce word 1
        let salt = keccak256_block(b1);

        // Hash 2: address = keccak256(0xff ++ factory ++ create2_salt ++ init_code_hash).
        // create2_salt lands at message byte offset 21 (word 5, +1 byte), so
        // inject it into the precomputed base block with a uniform 8-bit shift.
        var b2: array<u32, 34>;
        for (var i = 0u; i < 34u; i = i + 1u) {
            b2[i] = params.hash2_base[i];
        }
        for (var k = 0u; k < 8u; k = k + 1u) {
            b2[5u + k] = b2[5u + k] | (salt[k] << 8u);
            b2[6u + k] = b2[6u + k] | (salt[k] >> 24u);
        }
        let digest = keccak256_block(b2);

        // Address = digest bytes 12..32 = words 3..8.
        var matches = false;
        var report_count = 0u;
        if (params.mode == MODE_PREFIX) {
            matches = true;
            for (var k = 0u; k < nfull; k = k + 1u) {
                if (digest[3u + k] != params.prefix[k]) {
                    matches = false;
                }
            }
            if (rem > 0u && (digest[3u + nfull] & mask) != (params.prefix[nfull] & mask)) {
                matches = false;
            }
        } else {
            report_count = count_leading(digest, params.leading_nibble);
            matches = report_count >= params.threshold;
        }

        if (matches) {
            // First matching invocation claims the slot and writes its count and
            // nonce (which is exactly the nonce region of the hash-1 block).
            let prev = atomicExchange(&result.found, 1u);
            if (prev == 0u) {
                result.count = report_count;
                for (var k = 0u; k < 8u; k = k + 1u) {
                    result.nonce[k] = b1[8u + k];
                }
            }
        }
    }
}
