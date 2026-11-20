//! Spectrum + waterfall with Vulkan on the app's Surface.
//!
//! The DSP thread writes display lines into the service's FftHistory (CPU ring). Each frame,
//! new lines are copied into a host-visible storage buffer that the fragment shader indexes
//! directly (no image uploads). The trace is a triangle strip built in the vertex shader from
//! the latest line; colours come from the palette in shaders/common.glsl.
//!
//! The instance, device, pipelines and buffers live for the whole process (made on the
//! first attach); each attach only makes a surface, a swapchain and its sync objects.

use super::{log, ANativeWindow, ANativeWindow_getHeight, ANativeWindow_getWidth, ANativeWindow_release};
use crate::service::service;
use ash::{khr, vk};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What the UI sets (setView / setDisplay)
#[derive(Clone, Copy)]
pub struct Params {
    /// Hz across the width (0: whole span)
    pub view_bandwidth: f64,
    /// Hz, view centre relative to the hardware centre
    pub view_offset: f64,
    pub db_min: f32,
    pub db_max: f32,
    /// Height share of the trace (top)
    pub spectrum_fraction: f32,
    pub fill: [f32; 4],
    pub line: [f32; 4],
    pub background: [f32; 4],
    pub line_width: f32,
    pub palette: i32,
}

static PARAMS: Mutex<Params> = Mutex::new(Params {
    view_bandwidth: 0.0,
    view_offset: 0.0,
    db_min: -120.0,
    db_max: -20.0,
    spectrum_fraction: 0.35,
    fill: [0.55, 0.75, 1.0, 0.35],
    line: [0.80, 0.90, 1.0, 1.0],
    background: [0.07, 0.07, 0.09, 1.0],
    line_width: 3.0,
    palette: 0,
});
static DIRTY: AtomicBool = AtomicBool::new(true);

pub fn update(f: impl FnOnce(&mut Params)) {
    f(&mut PARAMS.lock().unwrap());
    DIRTY.store(true, Ordering::Relaxed);
}

/// Must match shaders/common.glsl
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Push {
    bin_start: f32,
    bin_count: f32,
    db_min: f32,
    db_max: f32,
    bins: i32,
    rows: i32,
    newest: i32,
    filled: i32,
    width: f32,
    height: f32,
    off_y: f32,
    line_w: f32,
    fill: [f32; 4],
    line: [f32; 4],
    palette: i32,
    line_offset: i32,
    pad: [i32; 2],
}
const _: () = assert!(std::mem::size_of::<Push>() == 96);

const FRAMES: usize = 2;
const TIMEOUT_NS: u64 = 100_000_000;

trait Ck<T> {
    fn ck(self, what: &str) -> Result<T, String>;
}
impl<T> Ck<T> for Result<T, vk::Result> {
    fn ck(self, what: &str) -> Result<T, String> {
        self.map_err(|e| format!("{what}: {e:?}"))
    }
}

struct HostBuf {
    buf: vk::Buffer,
    mem: vk::DeviceMemory,
    mapped: *mut f32,
}

impl HostBuf {
    const NONE: HostBuf = HostBuf { buf: vk::Buffer::null(), mem: vk::DeviceMemory::null(), mapped: std::ptr::null_mut() };
}

/// Process-lifetime objects
struct Ctx {
    _entry: ash::Entry,
    instance: ash::Instance,
    surf_i: khr::surface::Instance,
    android_i: khr::android_surface::Instance,
    phys: vk::PhysicalDevice,
    qf: u32,
    device: ash::Device,
    queue: vk::Queue,
    swap_d: khr::swapchain::Device,
    format: vk::Format,
    render_pass: vk::RenderPass,
    pipe_layout: vk::PipelineLayout,
    wf_pipe: vk::Pipeline,
    spec_pipe: vk::Pipeline,
    desc_set: vk::DescriptorSet,
    lines: HostBuf,
    latest: HostBuf,
    gpu_gen: u64,
    copied: u64,
    ready: bool,
    bins: usize,
    rows: usize,
    newest: i32,
    filled: i32,
    peak_hold: bool,
}

// Vulkan handles and the mapped pointers are used by one renderer thread at a time (CTX lock)
unsafe impl Send for Ctx {}

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

/// Per-surface objects
struct Surf {
    win: *mut ANativeWindow,
    surface: vk::SurfaceKHR,
    swapchain: vk::SwapchainKHR,
    extent: vk::Extent2D,
    views: Vec<vk::ImageView>,
    fbs: Vec<vk::Framebuffer>,
    render_sem: Vec<vk::Semaphore>,
    pool: vk::CommandPool,
    cmd: Vec<vk::CommandBuffer>,
    fences: [vk::Fence; FRAMES],
    acquire: [vk::Semaphore; FRAMES],
    frame: usize,
}

struct Running {
    thread: JoinHandle<()>,
    run: Arc<AtomicBool>,
    recreate: Arc<AtomicBool>,
}

static RENDERER: Mutex<Option<Running>> = Mutex::new(None);

struct Win(*mut ANativeWindow);
unsafe impl Send for Win {}

/// Takes one window reference (released when the renderer stops)
pub fn attach(w: *mut ANativeWindow) {
    let mut r = RENDERER.lock().unwrap();
    if let Some(old) = r.take() {
        old.run.store(false, Ordering::Relaxed);
        let _ = old.thread.join();
    }
    let run = Arc::new(AtomicBool::new(true));
    let recreate = Arc::new(AtomicBool::new(false));
    let (rn, rc, win) = (run.clone(), recreate.clone(), Win(w));
    DIRTY.store(true, Ordering::Relaxed);
    let thread = std::thread::Builder::new()
        .name("vulkan".into())
        .spawn(move || {
            let win = win;
            if let Err(e) = render_loop(win.0, &rn, &rc) {
                log(&format!("Vulkan: {e}"));
            }
            // SAFETY: the reference handed to attach
            unsafe { ANativeWindow_release(win.0) };
        })
        .expect("vulkan thread");
    *r = Some(Running { thread, run, recreate });
}

pub fn resize() {
    if let Some(r) = RENDERER.lock().unwrap().as_ref() {
        r.recreate.store(true, Ordering::Relaxed);
        DIRTY.store(true, Ordering::Relaxed);
    }
}

pub fn detach() {
    if let Some(r) = RENDERER.lock().unwrap().take() {
        r.run.store(false, Ordering::Relaxed);
        let _ = r.thread.join();
    }
}

fn render_loop(win: *mut ANativeWindow, run: &AtomicBool, recreate: &AtomicBool) -> Result<(), String> {
    let mut g = CTX.lock().unwrap();
    let surface = if g.is_none() {
        let (ctx, surface) = Ctx::new(win)?;
        *g = Some(ctx);
        log("Vulkan context created");
        surface
    } else {
        let ctx = g.as_ref().unwrap();
        let s = ctx.surface(win)?;
        // SAFETY: valid handles
        if !unsafe { ctx.surf_i.get_physical_device_surface_support(ctx.phys, ctx.qf, s) }.unwrap_or(false) {
            unsafe { ctx.surf_i.destroy_surface(s, None) };
            return Err("queue can't present to the new surface".into());
        }
        s
    };
    let ctx = g.as_mut().unwrap();
    let mut s = match ctx.surf(win, surface) {
        Ok(s) => s,
        Err(e) => {
            // SAFETY: the surface is ours
            unsafe { ctx.surf_i.destroy_surface(surface, None) };
            return Err(e);
        }
    };
    log(&format!("Vulkan renderer running ({}x{})", s.extent.width, s.extent.height));
    let res = (|| {
        let mut last = Instant::now();
        while run.load(Ordering::Relaxed) {
            if recreate.swap(false, Ordering::Relaxed) {
                // SAFETY: valid device
                unsafe { ctx.device.device_wait_idle() }.ck("wait idle")?;
                ctx.swapchain(&mut s)?;
            }
            let fresh = ctx.sync_data()?;
            if !fresh && !DIRTY.load(Ordering::Relaxed) && last.elapsed() < Duration::from_millis(500) {
                std::thread::sleep(Duration::from_millis(4));
                continue;
            }
            DIRTY.store(false, Ordering::Relaxed);
            last = Instant::now();
            if s.extent.width == 0 || s.extent.height == 0 {
                std::thread::sleep(Duration::from_millis(16));
                continue;
            }
            match ctx.draw(&mut s)? {
                Draw::Ok => {}
                Draw::Retry => DIRTY.store(true, Ordering::Relaxed),
                Draw::Recreate => recreate.store(true, Ordering::Relaxed),
                Draw::Lost => break,
            }
        }
        Ok(())
    })();
    ctx.destroy_surf(s);
    log("Vulkan renderer stopped");
    res
}

enum Draw {
    Ok,
    Retry,
    Recreate,
    Lost,
}

fn spv(bytes: &[u8]) -> Vec<u32> {
    ash::util::read_spv(&mut std::io::Cursor::new(bytes)).expect("spir-v")
}

impl Ctx {
    fn new(win: *mut ANativeWindow) -> Result<(Ctx, vk::SurfaceKHR), String> {
        // SAFETY: Vulkan calls with valid create infos; every handle comes from the calls above it
        unsafe {
            let entry = ash::Entry::linked();
            let app = vk::ApplicationInfo::default().application_name(c"LittleSDR").api_version(vk::API_VERSION_1_0);
            let ext = [khr::surface::NAME.as_ptr(), khr::android_surface::NAME.as_ptr()];
            let instance = entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app).enabled_extension_names(&ext), None).ck("instance")?;
            let surf_i = khr::surface::Instance::new(&entry, &instance);
            let android_i = khr::android_surface::Instance::new(&entry, &instance);
            let surface = android_i.create_android_surface(&vk::AndroidSurfaceCreateInfoKHR::default().window(win as *mut _), None).ck("surface")?;

            let mut pick = None;
            for d in instance.enumerate_physical_devices().ck("devices")? {
                for (q, p) in instance.get_physical_device_queue_family_properties(d).iter().enumerate() {
                    if p.queue_flags.contains(vk::QueueFlags::GRAPHICS) && surf_i.get_physical_device_surface_support(d, q as u32, surface).unwrap_or(false) {
                        pick = Some((d, q as u32));
                        break;
                    }
                }
                if pick.is_some() {
                    break;
                }
            }
            let (phys, qf) = pick.ok_or("no graphics+present queue")?;
            let prio = [1.0f32];
            let qci = [vk::DeviceQueueCreateInfo::default().queue_family_index(qf).queue_priorities(&prio)];
            let dext = [khr::swapchain::NAME.as_ptr()];
            let device = instance.create_device(phys, &vk::DeviceCreateInfo::default().queue_create_infos(&qci).enabled_extension_names(&dext), None).ck("device")?;
            let queue = device.get_device_queue(qf, 0);
            let swap_d = khr::swapchain::Device::new(&instance, &device);

            // UNORM: palette values are used as they are
            let fmts = surf_i.get_physical_device_surface_formats(phys, surface).unwrap_or_default();
            let format = fmts
                .iter()
                .map(|f| f.format)
                .find(|&f| f == vk::Format::R8G8B8A8_UNORM || f == vk::Format::B8G8R8A8_UNORM)
                .or(fmts.first().map(|f| f.format))
                .unwrap_or(vk::Format::R8G8B8A8_UNORM);

            let att = [vk::AttachmentDescription::default()
                .format(format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
            let refs = [vk::AttachmentReference { attachment: 0, layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL }];
            let sub = [vk::SubpassDescription::default().pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS).color_attachments(&refs)];
            let dep = [vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
            let render_pass = device.create_render_pass(&vk::RenderPassCreateInfo::default().attachments(&att).subpasses(&sub).dependencies(&dep), None).ck("render pass")?;

            let stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
            let b: Vec<_> = (0..2).map(|i| vk::DescriptorSetLayoutBinding::default().binding(i).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1).stage_flags(stages)).collect();
            let desc_layout = device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&b), None).ck("set layout")?;
            let pcr = [vk::PushConstantRange { stage_flags: stages, offset: 0, size: 96 }];
            let layouts = [desc_layout];
            let pipe_layout = device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts).push_constant_ranges(&pcr), None).ck("pipeline layout")?;

            let pipe = |vs: &[u8], fs: &[u8], topo: vk::PrimitiveTopology, blend: bool| -> Result<vk::Pipeline, String> {
                let (vc, fc) = (spv(vs), spv(fs));
                let vm = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vc), None).ck("shader")?;
                let fm = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&fc), None).ck("shader")?;
                let st = [
                    vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vm).name(c"main"),
                    vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(fm).name(c"main"),
                ];
                let vi = vk::PipelineVertexInputStateCreateInfo::default();
                let ia = vk::PipelineInputAssemblyStateCreateInfo::default().topology(topo);
                let vp = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
                let rs = vk::PipelineRasterizationStateCreateInfo::default().polygon_mode(vk::PolygonMode::FILL).cull_mode(vk::CullModeFlags::NONE).line_width(1.0);
                let ms = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
                let mut cba = vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA);
                if blend {
                    cba = cba
                        .blend_enable(true)
                        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                        .color_blend_op(vk::BlendOp::ADD)
                        .src_alpha_blend_factor(vk::BlendFactor::ONE)
                        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                        .alpha_blend_op(vk::BlendOp::ADD);
                }
                let cbas = [cba];
                let cb = vk::PipelineColorBlendStateCreateInfo::default().attachments(&cbas);
                let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
                let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
                let gp = [vk::GraphicsPipelineCreateInfo::default()
                    .stages(&st)
                    .vertex_input_state(&vi)
                    .input_assembly_state(&ia)
                    .viewport_state(&vp)
                    .rasterization_state(&rs)
                    .multisample_state(&ms)
                    .color_blend_state(&cb)
                    .dynamic_state(&ds)
                    .layout(pipe_layout)
                    .render_pass(render_pass)];
                let p = device.create_graphics_pipelines(vk::PipelineCache::null(), &gp, None).map_err(|(_, e)| format!("pipeline: {e:?}"));
                device.destroy_shader_module(vm, None);
                device.destroy_shader_module(fm, None);
                Ok(p?[0])
            };
            let wf_pipe = pipe(
                include_bytes!(concat!(env!("OUT_DIR"), "/waterfall_vert.spv")),
                include_bytes!(concat!(env!("OUT_DIR"), "/waterfall_frag.spv")),
                vk::PrimitiveTopology::TRIANGLE_LIST,
                false,
            )?;
            let spec_pipe = pipe(
                include_bytes!(concat!(env!("OUT_DIR"), "/spectrum_vert.spv")),
                include_bytes!(concat!(env!("OUT_DIR"), "/spectrum_frag.spv")),
                vk::PrimitiveTopology::TRIANGLE_STRIP,
                true,
            )?;

            let ps = [vk::DescriptorPoolSize { ty: vk::DescriptorType::STORAGE_BUFFER, descriptor_count: 2 }];
            let pool = device.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&ps), None).ck("descriptor pool")?;
            let desc_set = device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts)).ck("descriptor set")?[0];

            let ctx = Ctx {
                _entry: entry,
                instance,
                surf_i,
                android_i,
                phys,
                qf,
                device,
                queue,
                swap_d,
                format,
                render_pass,
                pipe_layout,
                wf_pipe,
                spec_pipe,
                desc_set,
                lines: HostBuf::NONE,
                latest: HostBuf::NONE,
                gpu_gen: u64::MAX,
                copied: 0,
                ready: false,
                bins: 0,
                rows: 0,
                newest: 0,
                filled: 0,
                peak_hold: false,
            };
            Ok((ctx, surface))
        }
    }

    fn surface(&self, win: *mut ANativeWindow) -> Result<vk::SurfaceKHR, String> {
        // SAFETY: a live window reference
        unsafe { self.android_i.create_android_surface(&vk::AndroidSurfaceCreateInfoKHR::default().window(win as *mut _), None) }.ck("surface")
    }

    fn surf(&self, win: *mut ANativeWindow, surface: vk::SurfaceKHR) -> Result<Surf, String> {
        let d = &self.device;
        // SAFETY: valid device and create infos
        unsafe {
            let pool = d
                .create_command_pool(&vk::CommandPoolCreateInfo::default().flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER).queue_family_index(self.qf), None)
                .ck("command pool")?;
            let cmd = d
                .allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(FRAMES as u32))
                .ck("command buffers")?;
            let mut fences = [vk::Fence::null(); FRAMES];
            let mut acquire = [vk::Semaphore::null(); FRAMES];
            for i in 0..FRAMES {
                fences[i] = d.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None).ck("fence")?;
                acquire[i] = d.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).ck("semaphore")?;
            }
            let mut s = Surf {
                win,
                surface,
                swapchain: vk::SwapchainKHR::null(),
                extent: vk::Extent2D::default(),
                views: Vec::new(),
                fbs: Vec::new(),
                render_sem: Vec::new(),
                pool,
                cmd,
                fences,
                acquire,
                frame: 0,
            };
            self.swapchain(&mut s)?;
            Ok(s)
        }
    }

    fn swapchain(&self, s: &mut Surf) -> Result<(), String> {
        // SAFETY: valid handles; the old swapchain is idle (device waited by the caller)
        unsafe {
            let caps = self.surf_i.get_physical_device_surface_capabilities(self.phys, s.surface).ck("caps")?;
            let mut extent = caps.current_extent;
            if extent.width == u32::MAX {
                extent = vk::Extent2D { width: ANativeWindow_getWidth(s.win).max(0) as u32, height: ANativeWindow_getHeight(s.win).max(0) as u32 };
            }
            s.extent = extent;
            if extent.width == 0 || extent.height == 0 {
                return Ok(()); // minimised, later
            }
            let mut count = (caps.min_image_count + 1).max(3);
            if caps.max_image_count > 0 {
                count = count.min(caps.max_image_count);
            }
            let alpha = [
                vk::CompositeAlphaFlagsKHR::OPAQUE,
                vk::CompositeAlphaFlagsKHR::INHERIT,
                vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
            ]
            .into_iter()
            .find(|&a| caps.supported_composite_alpha.contains(a))
            .unwrap_or(vk::CompositeAlphaFlagsKHR::INHERIT);
            let transform = if caps.supported_transforms.contains(vk::SurfaceTransformFlagsKHR::IDENTITY) { vk::SurfaceTransformFlagsKHR::IDENTITY } else { caps.current_transform };
            let ci = vk::SwapchainCreateInfoKHR::default()
                .surface(s.surface)
                .min_image_count(count)
                .image_format(self.format)
                .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                .image_extent(extent)
                .image_array_layers(1)
                .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                .pre_transform(transform)
                .composite_alpha(alpha)
                .present_mode(vk::PresentModeKHR::FIFO)
                .clipped(true)
                .old_swapchain(s.swapchain);
            let new = self.swap_d.create_swapchain(&ci, None).ck("swapchain")?;
            self.destroy_images(s);
            if s.swapchain != vk::SwapchainKHR::null() {
                self.swap_d.destroy_swapchain(s.swapchain, None);
            }
            s.swapchain = new;
            for img in self.swap_d.get_swapchain_images(new).ck("images")? {
                let view = self
                    .device
                    .create_image_view(
                        &vk::ImageViewCreateInfo::default().image(img).view_type(vk::ImageViewType::TYPE_2D).format(self.format).subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        }),
                        None,
                    )
                    .ck("image view")?;
                s.views.push(view);
                let att = [view];
                let fb = self
                    .device
                    .create_framebuffer(&vk::FramebufferCreateInfo::default().render_pass(self.render_pass).attachments(&att).width(extent.width).height(extent.height).layers(1), None)
                    .ck("framebuffer")?;
                s.fbs.push(fb);
                s.render_sem.push(self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).ck("semaphore")?);
            }
            Ok(())
        }
    }

    fn destroy_images(&self, s: &mut Surf) {
        // SAFETY: objects of this device, idle
        unsafe {
            for fb in s.fbs.drain(..) {
                self.device.destroy_framebuffer(fb, None);
            }
            for v in s.views.drain(..) {
                self.device.destroy_image_view(v, None);
            }
            for sem in s.render_sem.drain(..) {
                self.device.destroy_semaphore(sem, None);
            }
        }
    }

    fn destroy_surf(&self, mut s: Surf) {
        // SAFETY: wait idle first, then this surface's objects only
        unsafe {
            let _ = self.device.device_wait_idle();
            self.destroy_images(&mut s);
            if s.swapchain != vk::SwapchainKHR::null() {
                self.swap_d.destroy_swapchain(s.swapchain, None);
            }
            for i in 0..FRAMES {
                self.device.destroy_fence(s.fences[i], None);
                self.device.destroy_semaphore(s.acquire[i], None);
            }
            self.device.destroy_command_pool(s.pool, None);
            self.surf_i.destroy_surface(s.surface, None);
        }
    }

    fn host_buf(&self, hb: &mut HostBuf, bytes: u64) -> Result<(), String> {
        // SAFETY: valid device; the old buffer is idle (device waited by the caller)
        unsafe {
            if !hb.mapped.is_null() {
                self.device.unmap_memory(hb.mem);
            }
            if hb.buf != vk::Buffer::null() {
                self.device.destroy_buffer(hb.buf, None);
            }
            if hb.mem != vk::DeviceMemory::null() {
                self.device.free_memory(hb.mem, None);
            }
            *hb = HostBuf::NONE;
            let buf = self
                .device
                .create_buffer(&vk::BufferCreateInfo::default().size(bytes).usage(vk::BufferUsageFlags::STORAGE_BUFFER).sharing_mode(vk::SharingMode::EXCLUSIVE), None)
                .ck("buffer")?;
            let req = self.device.get_buffer_memory_requirements(buf);
            let mp = self.instance.get_physical_device_memory_properties(self.phys);
            let want = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let ty = (0..mp.memory_type_count)
                .find(|&i| req.memory_type_bits & (1 << i) != 0 && mp.memory_types[i as usize].property_flags.contains(want))
                .ok_or("no host-coherent memory")?;
            let mem = self.device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None).ck("memory")?;
            self.device.bind_buffer_memory(buf, mem, 0).ck("bind")?;
            let mapped = self.device.map_memory(mem, 0, bytes, vk::MemoryMapFlags::empty()).ck("map")? as *mut f32;
            *hb = HostBuf { buf, mem, mapped };
            Ok(())
        }
    }

    /// Copies new display lines to the GPU ring; true if anything changed
    fn sync_data(&mut self) -> Result<bool, String> {
        let Some(svc) = service() else { return Ok(false) };
        let h = svc.history().lock().unwrap();
        if h.bins == 0 {
            return Ok(false);
        }
        // SAFETY: mapped buffers sized bins × rows (lines) and 2 × bins (latest + peak)
        unsafe {
            if h.generation != self.gpu_gen {
                self.device.device_wait_idle().ck("wait idle")?;
                let (mut lines, mut latest) = (std::mem::replace(&mut self.lines, HostBuf::NONE), std::mem::replace(&mut self.latest, HostBuf::NONE));
                let r = self.host_buf(&mut lines, (h.bins * h.rows * 4) as u64).and_then(|_| self.host_buf(&mut latest, (h.bins * 2 * 4) as u64));
                (self.lines, self.latest) = (lines, latest);
                r?;
                let bi0 = [vk::DescriptorBufferInfo { buffer: self.lines.buf, offset: 0, range: vk::WHOLE_SIZE }];
                let bi1 = [vk::DescriptorBufferInfo { buffer: self.latest.buf, offset: 0, range: vk::WHOLE_SIZE }];
                let w = [
                    vk::WriteDescriptorSet::default().dst_set(self.desc_set).dst_binding(0).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).buffer_info(&bi0),
                    vk::WriteDescriptorSet::default().dst_set(self.desc_set).dst_binding(1).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).buffer_info(&bi1),
                ];
                self.device.update_descriptor_sets(&w, &[]);
                self.gpu_gen = h.generation;
                self.bins = h.bins;
                self.rows = h.rows;
                std::ptr::copy_nonoverlapping(h.lines.as_ptr(), self.lines.mapped, h.lines.len());
                self.copied = h.written;
                self.ready = true;
            } else if h.written != self.copied {
                let n = h.written - self.copied;
                if n >= self.rows as u64 {
                    std::ptr::copy_nonoverlapping(h.lines.as_ptr(), self.lines.mapped, h.lines.len());
                } else {
                    for r in self.copied..h.written {
                        let off = (r % self.rows as u64) as usize * self.bins;
                        std::ptr::copy_nonoverlapping(h.lines.as_ptr().add(off), self.lines.mapped.add(off), self.bins);
                    }
                }
                self.copied = h.written;
            } else {
                return Ok(false);
            }
            std::ptr::copy_nonoverlapping(h.latest.as_ptr(), self.latest.mapped, self.bins);
            self.peak_hold = h.peak_hold;
            if self.peak_hold && h.peak.len() >= self.bins {
                std::ptr::copy_nonoverlapping(h.peak.as_ptr(), self.latest.mapped.add(self.bins), self.bins);
            }
        }
        self.newest = if h.written > 0 { ((h.written - 1) % self.rows as u64) as i32 } else { 0 };
        self.filled = h.written.min(self.rows as u64) as i32;
        Ok(true)
    }

    fn draw(&mut self, s: &mut Surf) -> Result<Draw, String> {
        if s.swapchain == vk::SwapchainKHR::null() || !self.ready {
            return Ok(Draw::Ok);
        }
        let d = &self.device;
        let f = s.frame;
        // SAFETY: valid handles; bounded waits so that stopping never hangs on a dying surface
        unsafe {
            match d.wait_for_fences(&[s.fences[f]], true, TIMEOUT_NS) {
                Ok(()) => {}
                Err(vk::Result::TIMEOUT) => return Ok(Draw::Retry),
                Err(e) => return Err(format!("fence: {e:?}")),
            }
            let idx = match self.swap_d.acquire_next_image(s.swapchain, TIMEOUT_NS, s.acquire[f], vk::Fence::null()) {
                Ok((i, _)) => i as usize,
                Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => return Ok(Draw::Retry),
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(Draw::Recreate),
                Err(vk::Result::ERROR_SURFACE_LOST_KHR) => return Ok(Draw::Lost),
                Err(e) => return Err(format!("acquire: {e:?}")),
            };
            d.reset_fences(&[s.fences[f]]).ck("reset fence")?;

            let p = *PARAMS.lock().unwrap();
            let sr = service().map_or(0.0, |v| v.sample_rate()).max(1.0);
            let view_bw = if p.view_bandwidth > 0.0 && p.view_bandwidth < sr { p.view_bandwidth } else { sr };
            let bins = self.bins as f64;
            let mut pc = Push {
                bin_count: (bins * view_bw / sr) as f32,
                bin_start: (bins * 0.5 + (p.view_offset - view_bw / 2.0) / sr * bins) as f32,
                db_min: p.db_min,
                db_max: p.db_max,
                bins: self.bins as i32,
                rows: self.rows as i32,
                newest: self.newest,
                filled: self.filled,
                line_w: p.line_width,
                fill: p.fill,
                line: p.line,
                palette: p.palette,
                ..Default::default()
            };
            let (w, h) = (s.extent.width as f32, s.extent.height as f32);
            let spec_h = (h * p.spectrum_fraction.clamp(0.1, 0.9)).floor();
            let wf_h = h - spec_h;

            let cb = s.cmd[f];
            d.reset_command_buffer(cb, vk::CommandBufferResetFlags::empty()).ck("reset cmd")?;
            d.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)).ck("begin")?;
            let clear = [vk::ClearValue { color: vk::ClearColorValue { float32: p.background } }];
            let rp = vk::RenderPassBeginInfo::default()
                .render_pass(self.render_pass)
                .framebuffer(s.fbs[idx])
                .render_area(vk::Rect2D { offset: vk::Offset2D { x: 0, y: 0 }, extent: s.extent })
                .clear_values(&clear);
            d.cmd_begin_render_pass(cb, &rp, vk::SubpassContents::INLINE);
            d.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::GRAPHICS, self.pipe_layout, 0, &[self.desc_set], &[]);
            let stages = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
            let push = |pc: &Push| {
                let bytes = std::slice::from_raw_parts(pc as *const Push as *const u8, std::mem::size_of::<Push>());
                d.cmd_push_constants(cb, self.pipe_layout, stages, 0, bytes);
            };
            let area = |y: f32, hh: f32| {
                d.cmd_set_viewport(cb, 0, &[vk::Viewport { x: 0.0, y, width: w, height: hh, min_depth: 0.0, max_depth: 1.0 }]);
                d.cmd_set_scissor(cb, 0, &[vk::Rect2D { offset: vk::Offset2D { x: 0, y: y as i32 }, extent: vk::Extent2D { width: s.extent.width, height: hh as u32 } }]);
            };

            // Waterfall (bottom)
            if wf_h >= 1.0 {
                area(spec_h, wf_h);
                pc.width = w;
                pc.height = wf_h;
                pc.off_y = spec_h;
                d.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, self.wf_pipe);
                push(&pc);
                d.cmd_draw(cb, 6, 1, 0, 0);
            }
            // Trace (top), then the peak-hold line: same geometry, no fill, dimmer
            area(0.0, spec_h);
            pc.width = w;
            pc.height = spec_h;
            pc.off_y = 0.0;
            d.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, self.spec_pipe);
            push(&pc);
            d.cmd_draw(cb, w as u32 * 2, 1, 0, 0);
            if self.peak_hold {
                let mut pk = pc;
                pk.line_offset = self.bins as i32;
                pk.fill[3] = 0.0;
                pk.line[3] = 0.55;
                pk.line_w = pc.line_w * 0.75;
                push(&pk);
                d.cmd_draw(cb, w as u32 * 2, 1, 0, 0);
            }
            d.cmd_end_render_pass(cb);
            d.end_command_buffer(cb).ck("end")?;

            let wait = [s.acquire[f]];
            let stage = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let cbs = [cb];
            let signal = [s.render_sem[idx]];
            let si = [vk::SubmitInfo::default().wait_semaphores(&wait).wait_dst_stage_mask(&stage).command_buffers(&cbs).signal_semaphores(&signal)];
            d.queue_submit(self.queue, &si, s.fences[f]).ck("submit")?;
            let scs = [s.swapchain];
            let ids = [idx as u32];
            let pi = vk::PresentInfoKHR::default().wait_semaphores(&signal).swapchains(&scs).image_indices(&ids);
            s.frame = (f + 1) % FRAMES;
            Ok(match self.swap_d.queue_present(self.queue, &pi) {
                Ok(false) => Draw::Ok,
                Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Draw::Recreate,
                Err(vk::Result::ERROR_SURFACE_LOST_KHR) => Draw::Lost,
                Err(e) => return Err(format!("present: {e:?}")),
            })
        }
    }
}
