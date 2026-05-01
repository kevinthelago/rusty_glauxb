/// GPU hardware tier inferred from wgpu adapter info.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareTier {
    Discrete,
    Integrated,
    Software,
    Virtual,
    Unknown,
}

impl std::fmt::Display for HardwareTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Discrete   => write!(f, "discrete"),
            Self::Integrated => write!(f, "integrated"),
            Self::Software   => write!(f, "software"),
            Self::Virtual    => write!(f, "virtual"),
            Self::Unknown    => write!(f, "unknown"),
        }
    }
}

/// Adapter capabilities queried once at startup and used to label diagnostics.
#[derive(Clone)]
pub struct AdapterCaps {
    pub name:                  String,
    pub backend:               String,
    pub tier:                  HardwareTier,
    pub has_timestamp_query:   bool,
    pub has_polygon_mode_line: bool,
    /// Nanoseconds per timestamp tick, from `Adapter::get_timestamp_period()`.
    pub timestamp_period_ns:   f32,
}

impl AdapterCaps {
    pub fn from_adapter(adapter: &wgpu::Adapter) -> Self {
        let info     = adapter.get_info();
        let features = adapter.features();

        let tier = match info.device_type {
            wgpu::DeviceType::DiscreteGpu   => HardwareTier::Discrete,
            wgpu::DeviceType::IntegratedGpu => HardwareTier::Integrated,
            wgpu::DeviceType::Cpu           => HardwareTier::Software,
            wgpu::DeviceType::VirtualGpu    => HardwareTier::Virtual,
            _                               => HardwareTier::Unknown,
        };

        Self {
            name:                  info.name.clone(),
            backend:               format!("{:?}", info.backend),
            tier,
            has_timestamp_query:   features.contains(wgpu::Features::TIMESTAMP_QUERY),
            has_polygon_mode_line: features.contains(wgpu::Features::POLYGON_MODE_LINE),
            timestamp_period_ns:   1.0, // updated after device creation via apply_queue
        }
    }

    /// Call after `request_device` to fill in the queue-side timestamp period.
    pub fn apply_queue(&mut self, queue: &wgpu::Queue) {
        self.timestamp_period_ns = queue.get_timestamp_period();
    }
}
