//! GPU context: instance, adapter, device, queue.

use crate::RenderError;

/// The wgpu objects every renderer needs. Create one per process.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// Creates an instance honouring `WGPU_BACKEND` / `WGPU_ADAPTER_NAME` and
/// friends from the environment.
pub fn new_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
}

impl Gpu {
    /// A device for offscreen rendering only (no window, no display needed).
    pub fn headless() -> Result<Self, RenderError> {
        Self::new(new_instance(), None)
    }

    /// A device on `instance`, compatible with `surface` if given.
    pub fn new(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, RenderError> {
        let adapter = match Self::adapter_from_env(&instance, surface) {
            Some(a) => a,
            None => pollster::block_on(
                instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::from_env()
                        .unwrap_or(wgpu::PowerPreference::HighPerformance),
                    force_fallback_adapter: false,
                    compatible_surface: surface,
                    ..Default::default()
                }),
            )
            .map_err(|e| RenderError::NoAdapter(e.to_string()))?,
        };
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?}, {})", info.name, info.backend, info.driver);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bopl-render device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// `WGPU_ADAPTER_NAME=<substring>` (case-insensitive): the first
    /// adapter whose name contains it (and that can present to `surface`).
    fn adapter_from_env(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Option<wgpu::Adapter> {
        let want = std::env::var("WGPU_ADAPTER_NAME").ok()?.to_lowercase();
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let found = adapters.into_iter().find(|a| {
            a.get_info().name.to_lowercase().contains(&want)
                && surface.is_none_or(|s| a.is_surface_supported(s))
        });
        if found.is_none() {
            log::warn!("WGPU_ADAPTER_NAME={want}: no matching adapter, using the default");
        }
        found
    }

    /// Adapter name and backend, for logs and `--list-devices`.
    pub fn describe(&self) -> String {
        let i = self.adapter.get_info();
        format!(
            "{} ({:?}, {:?}, driver {} {})",
            i.name, i.backend, i.device_type, i.driver, i.driver_info
        )
    }
}
