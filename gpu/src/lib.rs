//! GPU-accelerated vanity Safe address search.
//!
//! This crate mirrors [`deadbeef_core::search`] but offloads the work to a GPU
//! via [`wgpu`], which runs the same Keccak-256 `CREATE2` pipeline on Metal
//! (Apple M-series), Vulkan (NVIDIA/AMD/Intel on Linux) and DX12 (Windows) from
//! a single WGSL compute shader (`keccak256.wgsl`).
//!
//! Every GPU invocation tries one candidate `saltNonce`. A dispatch runs
//! millions of them in parallel; the host reads back the first match, verifies
//! it on the CPU (guarding against any shader bug), and reports it.

use bytemuck::Zeroable as _;
use deadbeef_core::Safe;
use rand::{rngs::SmallRng, Rng as _, SeedableRng as _};
use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    mem::size_of,
    time::{Duration, Instant},
};

/// Workgroup size; must match `@workgroup_size(...)` in `keccak256.wgsl`.
const WORKGROUP_SIZE: u32 = 256;

/// Nonces tried per invocation; must match `ITERS` in `keccak256.wgsl`. Each
/// thread grinds a batch of nonces so kernel-launch and readback overhead is
/// amortised over many hashes.
const ITERS: u32 = 64;

/// Number of workgroups dispatched per iteration. A dispatch tries
/// `groups * WORKGROUP_SIZE * ITERS` nonces (~67M), large enough to keep the
/// GPU saturated yet short enough for a responsive progress readout.
const DEFAULT_GROUPS: u32 = 1 << 12;

/// Fixed shader inputs plus the per-dispatch nonce seed. Laid out to match the
/// `Params` storage struct in `keccak256.wgsl` (tightly packed `u32`s).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    salt_prefix: [u32; 8],
    hash2_base: [u32; 34],
    prefix: [u32; 5],
    prefix_len: u32,
    mode: u32,
    leading_nibble: u32,
    threshold: u32,
    nonce_base: [u32; 8],
}

const MODE_PREFIX: u32 = 0;
const MODE_LEADING: u32 = 1;

/// Result written back by the shader, matching `SearchResult` in the shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SearchResult {
    found: u32,
    count: u32,
    nonce: [u32; 8],
}

/// Live search statistics, reported to the caller during [`Gpu::search`].
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// Total number of nonces tried so far.
    pub attempts: u64,
    /// Wall-clock time elapsed since the search started.
    pub elapsed: Duration,
    /// Average throughput in hashes (nonces) per second.
    pub hashrate: f64,
}

/// An error setting up the GPU.
#[derive(Debug)]
pub enum GpuError {
    /// No suitable GPU adapter could be acquired.
    NoAdapter(String),
    /// The GPU device could not be created.
    Device(String),
}

impl Display for GpuError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::NoAdapter(e) => write!(f, "no suitable GPU adapter found: {e}"),
            Self::Device(e) => write!(f, "failed to create GPU device: {e}"),
        }
    }
}

impl Error for GpuError {}

/// A GPU context for running vanity address searches.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    params_buf: wgpu::Buffer,
    result_buf: wgpu::Buffer,
    staging_buf: wgpu::Buffer,
    groups: u32,
    adapter_name: String,
}

impl Gpu {
    /// Acquires a high-performance GPU and builds the compute pipeline.
    pub fn new() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        let adapter_name = adapter.get_info().name;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("deadbeef-gpu"),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("keccak256"),
            source: wgpu::ShaderSource::Wgsl(include_str!("keccak256.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("deadbeef-bind-group-layout"),
            entries: &[storage_entry(0, true), storage_entry(1, false)],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("deadbeef-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("deadbeef-pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let result_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("result"),
            size: size_of::<SearchResult>() as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: size_of::<SearchResult>() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("deadbeef-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: result_buf.as_entire_binding(),
                },
            ],
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group,
            params_buf,
            result_buf,
            staging_buf,
            groups: DEFAULT_GROUPS,
            adapter_name,
        })
    }

    /// The name of the GPU adapter in use (e.g. `"Apple M1"`).
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Number of nonces tried per GPU dispatch.
    pub fn batch_size(&self) -> u64 {
        self.groups as u64 * WORKGROUP_SIZE as u64 * ITERS as u64
    }

    /// Searches for a `saltNonce` whose Safe address starts with `prefix`,
    /// setting it on `safe` before returning. `progress` is invoked after every
    /// dispatch with live statistics.
    ///
    /// The winning nonce is always re-verified on the CPU, so a (hypothetical)
    /// shader bug can never produce a wrong result — at worst the search would
    /// keep running.
    pub fn search(&self, safe: &mut Safe, prefix: &[u8], mut progress: impl FnMut(Progress)) {
        let prefix = &prefix[..prefix.len().min(20)];
        let mut params = build_params(safe, prefix);
        let batch = self.batch_size();

        let mut rng = SmallRng::from_os_rng();
        let start = Instant::now();
        let mut attempts = 0_u64;

        loop {
            let mut nonce_base = [0_u8; 32];
            rng.fill(&mut nonce_base);
            params.nonce_base = bytemuck::cast(nonce_base);

            let result = self.dispatch(&params);
            attempts += batch;

            let elapsed = start.elapsed();
            progress(Progress {
                attempts,
                elapsed,
                hashrate: attempts as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE),
            });

            if result.found != 0 {
                let nonce: [u8; 32] = bytemuck::cast(result.nonce);
                safe.update_salt_nonce(|n| n.copy_from_slice(&nonce));
                if safe.matches(prefix) {
                    return;
                }
                // A reported match that fails CPU verification would indicate a
                // shader bug; ignore it and keep searching rather than emit a
                // wrong address.
            }
        }
    }

    /// Searches for salt nonces that maximise the number of leading `nibble`
    /// nibbles (0..=15) in the Safe address.
    ///
    /// `improved` is invoked with the new count and `safe` (winning nonce set)
    /// each time a strictly better address is found; `progress` is invoked after
    /// every dispatch. This never returns — run it until the process is stopped.
    pub fn search_leading(
        &self,
        safe: &mut Safe,
        nibble: u8,
        mut improved: impl FnMut(u32, &Safe),
        mut progress: impl FnMut(Progress),
    ) {
        let mut params = build_params_leading(safe, nibble, 1);
        let batch = self.batch_size();

        let mut rng = SmallRng::from_os_rng();
        let start = Instant::now();
        let mut attempts = 0_u64;
        let mut best = 0_u32;

        loop {
            let mut nonce_base = [0_u8; 32];
            rng.fill(&mut nonce_base);
            params.nonce_base = bytemuck::cast(nonce_base);

            let result = self.dispatch(&params);
            attempts += batch;

            let elapsed = start.elapsed();
            progress(Progress {
                attempts,
                elapsed,
                hashrate: attempts as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE),
            });

            if result.found != 0 {
                let nonce: [u8; 32] = bytemuck::cast(result.nonce);
                safe.update_salt_nonce(|n| n.copy_from_slice(&nonce));
                // Re-verify the count on the CPU before reporting.
                let count = deadbeef_core::leading_count(safe.creation_address(), nibble);
                if count > best {
                    best = count;
                    params.threshold = best + 1;
                    improved(count, safe);
                }
            }
        }
    }

    /// Runs a single dispatch and reads back the result.
    fn dispatch(&self, params: &Params) -> SearchResult {
        self.queue
            .write_buffer(&self.params_buf, 0, bytemuck::bytes_of(params));
        self.queue.write_buffer(
            &self.result_buf,
            0,
            bytemuck::bytes_of(&SearchResult::zeroed()),
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(self.groups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.result_buf,
            0,
            &self.staging_buf,
            0,
            size_of::<SearchResult>() as u64,
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = self.staging_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("failed to poll device");
        rx.recv()
            .expect("map channel closed")
            .expect("failed to map staging buffer");

        let result = {
            let view = slice
                .get_mapped_range()
                .expect("failed to read staging buffer");
            *bytemuck::from_bytes::<SearchResult>(&view)
        };
        self.staging_buf.unmap();
        result
    }
}

/// Builds a storage-buffer binding layout entry.
fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Derives the fixed shader inputs (independent of search mode) from a [`Safe`].
/// Mode-specific fields and `nonce_base` are left zeroed for the caller to set.
fn build_base_params(safe: &Safe) -> Params {
    let salt_prefix: [u32; 8] = bytemuck::cast(safe.salt_prefix());
    let factory = safe.factory();
    let init_code_hash = safe.init_code_hash();

    // Precomputed hash-2 rate block: 0xff ++ factory ++ <salt gap> ++ init_code_hash,
    // Keccak-padded. Bytes 21..53 (create2_salt) are left zero and injected in
    // the shader.
    let mut block = [0_u8; 136];
    block[0] = 0xff;
    block[1..21].copy_from_slice(&factory.0);
    block[53..85].copy_from_slice(&init_code_hash);
    block[85] = 0x01; // keccak padding start (message is 85 bytes)
    block[135] = 0x80; // keccak padding end (last byte of the 136-byte rate)
    let hash2_base: [u32; 34] = bytemuck::cast(block);

    Params {
        salt_prefix,
        hash2_base,
        prefix: [0_u32; 5],
        prefix_len: 0,
        mode: MODE_PREFIX,
        leading_nibble: 0,
        threshold: 0,
        nonce_base: [0_u32; 8],
    }
}

/// Builds shader inputs for a prefix search.
fn build_params(safe: &Safe, prefix: &[u8]) -> Params {
    let n = prefix.len().min(20);
    let mut prefix_bytes = [0_u8; 20];
    prefix_bytes[..n].copy_from_slice(&prefix[..n]);

    Params {
        prefix: bytemuck::cast(prefix_bytes),
        prefix_len: n as u32,
        mode: MODE_PREFIX,
        ..build_base_params(safe)
    }
}

/// Builds shader inputs for a leading-nibble search with the given report
/// threshold.
fn build_params_leading(safe: &Safe, nibble: u8, threshold: u32) -> Params {
    Params {
        mode: MODE_LEADING,
        leading_nibble: nibble as u32,
        threshold,
        ..build_base_params(safe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadbeef_core::{address, config, Configuration, Safe};

    fn test_safe() -> Safe {
        Safe::new(Configuration {
            proxy: config::Proxy {
                factory: address!(nz "fafafafafafafafafafafafafafafafafafafafa"),
                init_code: deadbeef_core::hex!("c0dec0dec0dec0dec0dec0dec0dec0dec0dec0de").to_vec(),
                singleton: address!(nz "5afe5afe5afe5afe5afe5afe5afe5afe5afe5afe"),
            },
            account: config::Account {
                owners: vec![address!(nz "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")],
                threshold: 1,
                setup: None,
                fallback_handler: None,
                identifier: None,
            },
        })
    }

    // Validates the WGSL Keccak-256 against the CPU reference by matching a full
    // 20-byte address. Requires a GPU, so ignored by default (headless CI).
    #[test]
    #[ignore = "requires a GPU"]
    fn gpu_matches_cpu() {
        let safe = test_safe();
        let gpu = Gpu::new().expect("failed to init GPU");

        // Invocation gid=0, iter=0 uses `nonce_base` with its first 8 bytes
        // (words 0 and 1) zeroed. Compute the reference address for exactly that
        // nonce on the CPU, then require the GPU to reproduce all 20 bytes.
        let nonce_base = [0xab_u8; 32];
        let mut expected_nonce = nonce_base;
        expected_nonce[0..8].copy_from_slice(&[0; 8]);

        let mut cpu = safe.clone();
        cpu.update_salt_nonce(|n| n.copy_from_slice(&expected_nonce));
        let address = cpu.creation_address();

        let mut params = build_params(&safe, &address.0);
        params.nonce_base = bytemuck::cast(nonce_base);
        let result = gpu.dispatch(&params);

        assert_ne!(result.found, 0, "GPU did not reproduce the CPU address");
        let nonce: [u8; 32] = bytemuck::cast(result.nonce);
        assert_eq!(nonce, expected_nonce, "GPU winning nonce mismatch");
    }
}
