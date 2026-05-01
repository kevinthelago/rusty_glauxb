//! GPU timestamp queries for precise per-frame GPU execution time.
//!
//! Flow (two-frame lag):
//!   Frame N  — `begin_frame()` inserts begin/end timestamps into render passes.
//!              `resolve(encoder)` copies query results into a mappable buffer.
//!              After `queue.submit()`, `schedule_readback()` calls `map_async`.
//!   Frame N+1 — `poll_result(device)` checks if the map completed and returns
//!               the GPU time in milliseconds.  Returns `None` if not ready yet
//!               or if the feature is absent on this adapter.

use std::sync::{Arc, Mutex};

const N_SLOTS: u64 = 4; // globe-begin, globe-end, egui-begin, egui-end
const BUF_SIZE: u64 = N_SLOTS * 8; // 8 bytes per u64 timestamp

pub struct GpuTimer {
    query_set:   wgpu::QuerySet,
    resolve_buf: wgpu::Buffer,
    map_buf:     wgpu::Buffer,
    /// Nanoseconds per timestamp tick from `Queue::get_timestamp_period()`.
    period_ns:   f32,
    /// `map_async` has been called; waiting for the callback.
    pending:     bool,
    /// `copy_buffer_to_buffer` was issued this frame; `schedule_readback` should map.
    needs_map:   bool,
    /// Set to `true` by the `map_async` callback when the buffer is ready to read.
    mapped:      Arc<Mutex<bool>>,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, period_ns: f32) -> Self {
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("diag:timestamps"),
            ty:    wgpu::QueryType::Timestamp,
            count: N_SLOTS as u32,
        });
        let resolve_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("diag:resolve"),
            size:               BUF_SIZE,
            usage:              wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let map_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("diag:map"),
            size:               BUF_SIZE,
            usage:              wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            query_set,
            resolve_buf,
            map_buf,
            period_ns,
            pending:   false,
            needs_map: false,
            mapped:    Arc::new(Mutex::new(false)),
        }
    }

    /// Borrow the QuerySet so callers can build `RenderPassTimestampWrites`.
    pub fn query_set(&self) -> &wgpu::QuerySet {
        &self.query_set
    }

    /// Issue resolve + copy into the mappable buffer.
    /// Call after all render passes are encoded, before `encoder.finish()`.
    /// If a previous readback is still in flight, the copy is skipped for this
    /// frame — `map_buf` must not be written while `map_async` is outstanding.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.query_set, 0..N_SLOTS as u32, &self.resolve_buf, 0);
        if !self.pending {
            encoder.copy_buffer_to_buffer(&self.resolve_buf, 0, &self.map_buf, 0, BUF_SIZE);
            self.needs_map = true;
        } else {
            self.needs_map = false;
        }
    }

    /// Schedule the async buffer map.  Call immediately after `queue.submit()`.
    pub fn schedule_readback(&mut self) {
        if !self.needs_map {
            return;
        }
        self.needs_map = false;
        self.pending   = true;
        let flag = Arc::clone(&self.mapped);
        self.map_buf
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    *flag.lock().unwrap() = true;
                }
            });
    }

    /// Attempt to read back the previous frame's GPU timestamps.
    /// Returns GPU time in milliseconds (globe + egui passes combined), or
    /// `None` if the data isn't ready yet.
    /// Call at the START of `render()`, before encoding new commands.
    pub fn poll_result(&mut self, device: &wgpu::Device) -> Option<f64> {
        if !self.pending {
            return None;
        }
        // Let wgpu process any completed async operations.
        device.poll(wgpu::Maintain::Poll);

        let ready = *self.mapped.lock().unwrap();
        if !ready {
            return None;
        }

        let timestamps = {
            let view = self.map_buf.slice(..).get_mapped_range();
            let bytes: &[u8] = &view;
            let mut arr = [0u64; N_SLOTS as usize];
            for (i, chunk) in bytes.chunks_exact(8).enumerate().take(N_SLOTS as usize) {
                arr[i] = u64::from_le_bytes(chunk.try_into().unwrap());
            }
            arr
        };

        self.map_buf.unmap();
        *self.mapped.lock().unwrap() = false;
        self.pending = false;

        // slots 0/1 = globe pass begin/end; slots 2/3 = egui pass begin/end
        let globe_ticks = timestamps[1].saturating_sub(timestamps[0]);
        let egui_ticks  = timestamps[3].saturating_sub(timestamps[2]);
        let total_ns    = (globe_ticks + egui_ticks) as f64 * self.period_ns as f64;
        Some(total_ns / 1_000_000.0) // → milliseconds
    }
}
