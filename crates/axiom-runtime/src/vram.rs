//! This process's video memory on the render adapter, from the OS rather than the graphics API.
//!
//! wgpu 24 reports an allocator total only on backends that use `gpu-allocator` (D3D12); its
//! Vulkan backend uses `gpu-alloc`, so `Device::generate_allocator_report` is `None` there. On
//! Windows every API's allocations go through the WDDM video memory manager, and DXGI
//! (`IDXGIAdapter3::QueryVideoMemoryInfo`) reports what the calling process uses on an adapter,
//! whichever API made it (swapchain images and driver-internal memory included). The adapter
//! is matched by PCI vendor + device ID. One reading (two queries) measured 6.6 µs in a debug
//! build (`vram::tests`); the editor makes one every 2 s.

/// What the status bar shows: bytes in use, or why there is no number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VramReading {
    Bytes {
        /// Dedicated (local) video memory this process uses on the adapter.
        local: u64,
        /// System memory the adapter maps for this process (non-local segment).
        shared: u64,
        /// The OS budget for the local segment.
        budget: u64,
    },
    Unavailable(String),
}

/// Opened once per renderer; `read` queries the adapter.
pub struct VideoMemory {
    #[cfg(windows)]
    adapter: Option<windows::Win32::Graphics::Dxgi::IDXGIAdapter3>,
    reason: String,
}

impl VideoMemory {
    /// The DXGI adapter with this PCI vendor / device ID (none on other platforms or when no
    /// adapter matches, e.g. a software adapter).
    pub fn open(vendor: u32, device: u32) -> Self {
        #[cfg(windows)]
        {
            match windows_adapter(vendor, device) {
                Ok(adapter) => Self {
                    adapter: Some(adapter),
                    reason: String::new(),
                },
                Err(reason) => Self {
                    adapter: None,
                    reason,
                },
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (vendor, device);
            Self {
                reason:
                    "per-process video memory is read through DXGI, which exists only on Windows"
                        .into(),
            }
        }
    }

    pub fn read(&self) -> VramReading {
        #[cfg(windows)]
        if let Some(adapter) = &self.adapter {
            use windows::Win32::Graphics::Dxgi::{
                DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
                DXGI_QUERY_VIDEO_MEMORY_INFO,
            };
            let mut local = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            let mut shared = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            // SAFETY: plain COM calls on a live adapter writing into owned structs.
            let result = unsafe {
                adapter
                    .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut local)
                    .and_then(|()| {
                        adapter.QueryVideoMemoryInfo(
                            0,
                            DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
                            &mut shared,
                        )
                    })
            };
            return match result {
                Ok(()) => VramReading::Bytes {
                    local: local.CurrentUsage,
                    shared: shared.CurrentUsage,
                    budget: local.Budget,
                },
                Err(e) => VramReading::Unavailable(format!("QueryVideoMemoryInfo failed: {e}")),
            };
        }
        VramReading::Unavailable(self.reason.clone())
    }
}

#[cfg(windows)]
fn windows_adapter(
    vendor: u32,
    device: u32,
) -> Result<windows::Win32::Graphics::Dxgi::IDXGIAdapter3, String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1};
    if vendor == 0 && device == 0 {
        return Err("the adapter has no PCI vendor / device ID (software or virtual)".into());
    }
    // SAFETY: COM factory creation and enumeration; every returned interface is owned.
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1 failed: {e}"))?;
        let mut index = 0;
        while let Ok(adapter) = factory.EnumAdapters1(index) {
            index += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            if desc.VendorId == vendor && desc.DeviceId == device {
                return adapter.cast::<IDXGIAdapter3>().map_err(|e| {
                    format!("the adapter has no IDXGIAdapter3 (Windows 10 or later needed): {e}")
                });
            }
        }
    }
    Err(format!(
        "no DXGI adapter matches PCI {vendor:04x}:{device:04x}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a Windows machine with a GPU: the number is this process's, so a 64 MiB buffer the
    /// test allocates and writes shows up in it (whichever wgpu backend made the device).
    #[test]
    fn a_buffer_this_process_allocates_shows_in_the_reading() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))
        else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let info = adapter.get_info();
        let vram = VideoMemory::open(info.vendor, info.device);
        if cfg!(not(windows)) || info.device_type == wgpu::DeviceType::Cpu {
            assert!(matches!(vram.read(), VramReading::Unavailable(_)) || cfg!(windows));
            return;
        }
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let used = |v: VramReading| match v {
            VramReading::Bytes { local, shared, .. } => local + shared,
            VramReading::Unavailable(why) => panic!("{} / {:?}: {why}", info.name, info.backend),
        };
        // Cost of one reading (the editor makes one every 2 s on the main thread).
        let started = std::time::Instant::now();
        for _ in 0..100 {
            let _ = vram.read();
        }
        eprintln!(
            "vram read: {:.1} µs each (mean of 100, {} build)",
            started.elapsed().as_secs_f64() * 1e4,
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
        const SIZE: u64 = 64 << 20;
        // Other GPU tests in this process may free memory between the two reads: a few tries.
        let mut seen = vec![];
        for _ in 0..3 {
            let before = used(vram.read());
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vram probe"),
                size: SIZE,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, &vec![1u8; SIZE as usize]);
            queue.submit([]);
            let _ = device.poll(wgpu::Maintain::Wait);
            let after = used(vram.read());
            drop(buffer);
            if after >= before + SIZE * 3 / 4 {
                return;
            }
            seen.push((before, after));
        }
        panic!(
            "{} / {:?}: before -> after {seen:?}",
            info.name, info.backend
        );
    }
}
