//! What the graphics driver is asked to build, and the switches that tell
//! a driver's failure apart (docs/design/rendering.md, *Diagnostics of the
//! graphics driver*; docs/guide/troubleshooting.md). Each environment
//! variable is read once; while none is set, nothing here costs more than
//! one cached check:
//!
//! - [`TRACE_VAR`] (`CRAFT3D_GPU_TRACE=<file>`): a trace of what the
//!   graphics card is asked to build. Before every call of this crate that
//!   makes the driver compile or link something (a shader module, a render
//!   pipeline, a bind group layout: every one goes through [`Traced`]) a
//!   line is appended to `<file>` and synced to disk, and another one when
//!   the call returns, so the last line of the file names what the driver
//!   was building when the process died (a crash inside a driver is not a
//!   panic: nothing can catch it). The same lines go to standard error.
//!   Other parts of the program add lines of their own with [`note`] (the
//!   window: its version, the adapter, what the device was asked for, the
//!   surface, its tour).
//! - [`SAMPLES_VAR`] (`CRAFT3D_SAMPLES=1|4`): the samples per pixel of the
//!   viewport's renderer ([`crate::Onscreen::new`]) instead of
//!   [`crate::WANTED_SAMPLES`], where the adapter has them.
//! - [`PEEL_VARIANT_VAR`] (`CRAFT3D_PEEL_VARIANT=auto|today|persample`):
//!   the form of the see-through's and the section's multisampled stages
//!   ([`PeelSetting`], `variant.rs`).
//! - [`BACKEND_ORDER_VAR`] (`CRAFT3D_BACKEND_ORDER=dx12,vulkan,gl`): which
//!   adapter the window takes; read by the window (`apps/craft3d`),
//!   parsed here ([`parse_backend_order`]); the choice is the window's
//!   (`apps/craft3d/src/graphics_start.rs`).
//!
//! # The trace
//!
//! One line per event, every line numbered in the order it was written:
//!
//! ```text
//! #7 begin render_pipeline "craft3d bodies" t=12.5ms thread=main vs=vs_main@#3 ... samples=4 ...
//! #7 end render_pipeline "craft3d bodies" t=19.0ms took=6.5ms
//! #8 info onscreen samples 4 (the default) ...
//! ```
//!
//! A `begin` and its `end` share their number; whatever else happened in
//! between (another thread) has numbers of its own. A pipeline's line
//! names the modules its stages come from by their own number and label,
//! with the entry points, the sample count, the colour targets and the
//! depth state as the descriptor has them, so no label can misname them.
//! A module's line has the size and a hash of its source, its entry
//! points and the builtins and interpolation it uses.
//!
//! Not in the browser: there nothing is traced and nothing is forced.

#[cfg(not(target_arch = "wasm32"))]
use std::fmt::Write as _;

/// The environment variable naming the trace file (see the module).
pub const TRACE_VAR: &str = "CRAFT3D_GPU_TRACE";
/// The environment variable that forces the viewport's samples per pixel.
pub const SAMPLES_VAR: &str = "CRAFT3D_SAMPLES";
/// The environment variable that orders the backends the window takes its
/// adapter from.
pub const BACKEND_ORDER_VAR: &str = "CRAFT3D_BACKEND_ORDER";
/// The environment variable that chooses the form of the multisampled
/// see-through and section stages: `auto` (the default), `today` (the
/// masked stages) or `persample` ([`PeelSetting`], `variant.rs`).
pub const PEEL_VARIANT_VAR: &str = "CRAFT3D_PEEL_VARIANT";

pub use crate::variant::{PeelSetting, PeelVariant, Stages, auto_choice, stages};

/// The setting [`PEEL_VARIANT_VAR`] asks for, read once: `auto` when it is
/// not set, and a bad value is said once on standard error and ignored.
#[cfg(not(target_arch = "wasm32"))]
pub fn peel_setting() -> PeelSetting {
    static SETTING: std::sync::OnceLock<PeelSetting> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| {
        let Some(value) = std::env::var_os(PEEL_VARIANT_VAR) else {
            return PeelSetting::Auto;
        };
        match PeelSetting::parse(&value.to_string_lossy()) {
            Ok(setting) => setting,
            Err(why) => {
                let said = format!("{PEEL_VARIANT_VAR}: {why}; auto is used");
                say(&said);
                note("variant", &said);
                PeelSetting::Auto
            }
        }
    })
}

/// In the browser, `auto` (which there is always the masked stages).
#[cfg(target_arch = "wasm32")]
pub fn peel_setting() -> PeelSetting {
    PeelSetting::Auto
}

/// How many characters of a value an error message repeats.
const QUOTED_CHARS: usize = 80;
/// How many characters a field of a trace line keeps.
#[cfg(not(target_arch = "wasm32"))]
const FIELD_CHARS: usize = 4000;

/// `value` cut to `limit` characters, with `…` where it was cut.
fn clipped(value: &str, limit: usize) -> String {
    match value.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", value.get(..end).unwrap_or_default()),
        None => value.to_string(),
    }
}

/// Says `text` on standard error; a closed stream is no reason to fail.
pub fn say(text: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "craft3d: {text}");
}

// ---------------------------------------------------------------------
// Samples per pixel.

/// The samples per pixel `value` asks for: `None` for an empty value (as
/// if it were not set), an error for anything but 1 or 4 (the counts
/// WebGPU guarantees for the viewport's formats: 2 failed wgpu's
/// validation on Intel UHD, 2026-11-16).
pub fn parse_samples(value: &str) -> Result<Option<u32>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed {
        "1" => Ok(Some(1)),
        "4" => Ok(Some(4)),
        _ => Err(format!(
            "{SAMPLES_VAR}=`{}` is not 1 or 4; ignored",
            clipped(trimmed, QUOTED_CHARS)
        )),
    }
}

/// The samples per pixel [`SAMPLES_VAR`] forces on the viewport, read once;
/// a bad value is said once on standard error and ignored.
#[cfg(not(target_arch = "wasm32"))]
pub fn forced_samples() -> Option<u32> {
    static FORCED: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        let value = std::env::var_os(SAMPLES_VAR)?;
        let value = value.to_string_lossy();
        match parse_samples(&value) {
            Ok(samples) => samples,
            Err(why) => {
                say(&why);
                note("samples", &why);
                None
            }
        }
    })
}

/// In the browser nothing is forced.
#[cfg(target_arch = "wasm32")]
pub fn forced_samples() -> Option<u32> {
    None
}

// ---------------------------------------------------------------------
// The order of the backends.

/// The backend a name in [`BACKEND_ORDER_VAR`] stands for (ignoring case):
/// `vulkan`/`vk`, `dx12`/`d3d12`, `metal`/`mtl`, `gl`/`opengl`/`gles`.
fn backend_named(name: &str) -> Option<wgpu::Backend> {
    match name.to_ascii_lowercase().as_str() {
        "vulkan" | "vk" => Some(wgpu::Backend::Vulkan),
        "dx12" | "d3d12" => Some(wgpu::Backend::Dx12),
        "metal" | "mtl" => Some(wgpu::Backend::Metal),
        "gl" | "opengl" | "gles" => Some(wgpu::Backend::Gl),
        _ => None,
    }
}

/// The backends `value` lists, in its order, each once: names separated by
/// commas, blanks around them and empty items ignored. `None` for an empty
/// value (as if it were not set); an error, for the whole value, when a
/// name is not a backend or none is named.
pub fn parse_backend_order(value: &str) -> Result<Option<Vec<wgpu::Backend>>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let mut order = Vec::new();
    for item in value.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let Some(backend) = backend_named(item) else {
            return Err(format!(
                "{BACKEND_ORDER_VAR}: `{}` is not a backend (vulkan, dx12, metal, gl); the \
                 variable is ignored",
                clipped(item, QUOTED_CHARS)
            ));
        };
        if !order.contains(&backend) {
            order.push(backend);
        }
    }
    if order.is_empty() {
        return Err(format!(
            "{BACKEND_ORDER_VAR}=`{}` names no backend; ignored",
            clipped(value, QUOTED_CHARS)
        ));
    }
    Ok(Some(order))
}

/// The order the window takes its adapter by, from the values of
/// [`BACKEND_ORDER_VAR`] (`order`) and of `WGPU_BACKEND` (`wgpu_backend`):
/// `None` when the order is not set or empty, or when `WGPU_BACKEND` is set
/// at all (it chooses then, as it always did); the error says why a value
/// is ignored.
pub fn backend_order(
    order: Option<&str>,
    wgpu_backend: Option<&str>,
) -> Result<Option<Vec<wgpu::Backend>>, String> {
    let Some(order) = order else {
        return Ok(None);
    };
    let parsed = parse_backend_order(order)?;
    match (parsed, wgpu_backend) {
        (Some(_), Some(backend)) => Err(format!(
            "{BACKEND_ORDER_VAR} is ignored: WGPU_BACKEND=`{}` chooses",
            clipped(backend, QUOTED_CHARS)
        )),
        (parsed, _) => Ok(parsed),
    }
}

/// The backends as a list, for messages: `dx12,vulkan,gl`.
pub fn backend_list(order: &[wgpu::Backend]) -> String {
    order
        .iter()
        .map(|b| b.to_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// The [`wgpu::Backends`] of `order`: what the window's instance is made
/// with, so a backend that is not listed is not even started.
pub fn backends_of(order: &[wgpu::Backend]) -> wgpu::Backends {
    order.iter().fold(wgpu::Backends::empty(), |all, b| {
        all | wgpu::Backends::from(*b)
    })
}

/// What the window's choice of adapter needs to know of one.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub backend: wgpu::Backend,
    pub device_type: wgpu::DeviceType,
    /// It can present to the window's surface (true without a surface).
    pub presents: bool,
}

impl Candidate {
    pub fn of(info: &wgpu::AdapterInfo, presents: bool) -> Candidate {
        Candidate {
            name: info.name.clone(),
            backend: info.backend,
            device_type: info.device_type,
            presents,
        }
    }
}

/// wgpu's own preference among adapters (`wgpu-core` 30.0.1,
/// `Instance::request_adapter`, `get_order`): lower first.
pub fn preference(device_type: wgpu::DeviceType, power: wgpu::PowerPreference) -> u8 {
    let integrated_first = power == wgpu::PowerPreference::LowPower;
    match device_type {
        wgpu::DeviceType::DiscreteGpu if integrated_first => 2,
        wgpu::DeviceType::IntegratedGpu if integrated_first => 1,
        wgpu::DeviceType::DiscreteGpu => 1,
        wgpu::DeviceType::IntegratedGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::VirtualGpu => 4,
        wgpu::DeviceType::Cpu => 5,
    }
}

/// An adapter in one line: name, backend, device type, driver, vendor and
/// device ids.
pub fn describe_adapter(info: &wgpu::AdapterInfo) -> String {
    format!(
        "`{}` backend={} type={:?} driver=`{}` driver_info=`{}` vendor=0x{:04x} device=0x{:04x}",
        info.name,
        info.backend.to_str(),
        info.device_type,
        info.driver,
        info.driver_info,
        info.vendor,
        info.device
    )
}

// ---------------------------------------------------------------------
// The trace.

/// Whether [`TRACE_VAR`] is set: the trace is written.
pub fn enabled() -> bool {
    tracer().is_some()
}

/// Adds an `info` line about `topic` to the trace, when it is written.
pub fn note(topic: &str, text: &str) {
    if let Some(tracer) = tracer() {
        tracer.info(topic, text);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn tracer() -> Option<&'static Tracer> {
    static TRACER: std::sync::OnceLock<Option<Tracer>> = std::sync::OnceLock::new();
    TRACER
        .get_or_init(|| {
            let path = std::env::var_os(TRACE_VAR)?;
            if path.is_empty() {
                return None;
            }
            Some(Tracer::open(std::path::Path::new(&path), true))
        })
        .as_ref()
}

#[cfg(target_arch = "wasm32")]
fn tracer() -> Option<&'static Tracer> {
    None
}

/// The modules a trace remembers, to name them in the lines of the
/// pipelines made from them; beyond this they are named `?`.
#[cfg(not(target_arch = "wasm32"))]
const MAX_MODULES: usize = 512;

/// Where the trace goes: a file (when it could be opened) and standard
/// error (`echo`).
#[cfg(not(target_arch = "wasm32"))]
pub struct Tracer {
    file: std::sync::Mutex<Option<std::fs::File>>,
    echo: bool,
    next: std::sync::atomic::AtomicU64,
    start: std::time::Instant,
    /// The shader modules made so far, with their number, label and
    /// entry points.
    modules: std::sync::Mutex<Vec<Remembered>>,
}

/// A shader module a trace remembers.
#[cfg(not(target_arch = "wasm32"))]
struct Remembered {
    module: wgpu::ShaderModule,
    number: u64,
    label: String,
    entries: Vec<String>,
}

/// In the browser there is no trace.
#[cfg(target_arch = "wasm32")]
pub struct Tracer;

#[cfg(target_arch = "wasm32")]
impl Tracer {
    fn info(&self, _: &str, _: &str) {}
}

#[cfg(not(target_arch = "wasm32"))]
impl Tracer {
    /// A trace appended to the file at `path`, and with `echo` on standard
    /// error. A file that cannot be opened is said once on standard error,
    /// and the trace goes to standard error only.
    pub fn open(path: &std::path::Path, echo: bool) -> Tracer {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path);
        let file = match file {
            Ok(file) => Some(file),
            Err(e) => {
                say(&format!(
                    "{TRACE_VAR}: the trace file `{}` cannot be opened ({e}); the trace goes to \
                     standard error only",
                    clipped(&field(&path.display().to_string()), QUOTED_CHARS * 4)
                ));
                None
            }
        };
        Tracer {
            file: std::sync::Mutex::new(file),
            echo,
            next: std::sync::atomic::AtomicU64::new(1),
            // native clock: `Tracer` is cfg(not(target_arch = "wasm32")).
            start: std::time::Instant::now(),
            modules: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn number(&self) -> u64 {
        self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn millis(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    /// Writes `line`: appended to the file and synced to disk before this
    /// returns, and on standard error.
    fn line(&self, line: &str) {
        use std::io::Write as _;
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(open) = file.as_mut() {
            let written = writeln!(open, "{line}")
                .and_then(|()| open.flush())
                .and_then(|()| open.sync_data());
            if let Err(e) = written {
                say(&format!(
                    "{TRACE_VAR}: the trace file cannot be written ({e}); the trace goes to \
                     standard error only"
                ));
                *file = None;
            }
        }
        if self.echo {
            say(&format!("gpu trace: {line}"));
        }
    }

    /// An `info` line about `topic`.
    pub fn info(&self, topic: &str, text: &str) {
        let line = format!(
            "#{} info {} t={:.1}ms {}",
            self.number(),
            field(topic),
            self.millis(),
            field(text)
        );
        self.line(&line);
    }

    /// Runs `make` between a `begin` line (with `detail`) and an `end`
    /// line of the `kind` of thing labelled `label`; the `begin` line is on
    /// disk before `make` runs.
    pub fn around<T>(
        &self,
        kind: &str,
        label: Option<&str>,
        detail: impl FnOnce() -> String,
        make: impl FnOnce() -> T,
    ) -> (u64, T) {
        let number = self.number();
        let label = field(label.unwrap_or("(no label)")).replace('"', "'");
        let thread = std::thread::current();
        let begun = self.millis();
        self.line(&format!(
            "#{number} begin {kind} \"{label}\" t={begun:.1}ms thread={} {}",
            field(thread.name().unwrap_or("?")),
            field(&detail())
        ));
        let made = make();
        let ended = self.millis();
        self.line(&format!(
            "#{number} end {kind} \"{label}\" t={ended:.1}ms took={:.1}ms",
            ended - begun
        ));
        (number, made)
    }

    /// Remembers `module`, made as number `number` with `label` and the
    /// entry points `entries`.
    fn remember(
        &self,
        module: &wgpu::ShaderModule,
        number: u64,
        label: Option<&str>,
        entries: Vec<String>,
    ) {
        let mut modules = self
            .modules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if modules.len() < MAX_MODULES {
            modules.push(Remembered {
                module: module.clone(),
                number,
                label: field(label.unwrap_or("(no label)")).replace('"', "'"),
                entries,
            });
        }
    }

    /// `#number"label"` of `module`, whose entry point `entry` a pipeline
    /// takes, or `?` for one not remembered. Modules compare by their id,
    /// and the ids of two instances (the window's, and the one of the
    /// renderer that reads back, made when the window's fails) can be the
    /// same (seen in a trace of 2026-11-16): only a module that has the
    /// entry point counts, the newest first, and where several still do,
    /// all are named.
    fn module_name(&self, module: &wgpu::ShaderModule, entry: Option<&str>) -> String {
        let modules = self
            .modules
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let names: Vec<String> = modules
            .iter()
            .rev()
            .filter(|r| &r.module == module)
            .filter(|r| entry.is_none_or(|e| r.entries.iter().any(|x| x == e)))
            .map(|r| format!("#{}\"{}\"", r.number, r.label))
            .collect();
        if names.is_empty() {
            "?".to_string()
        } else {
            names.join("|")
        }
    }
}

/// `text` for a trace line: on one line, control characters made blanks,
/// at most [`FIELD_CHARS`] characters.
#[cfg(not(target_arch = "wasm32"))]
fn field(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clipped(&flat, FIELD_CHARS)
}

/// 64-bit FNV-1a of `bytes`: which variant of a filled-in template a
/// module is, to compare traces of two machines.
#[cfg(not(target_arch = "wasm32"))]
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The distinct texts between `start` and the next `)` in `source`.
#[cfg(not(target_arch = "wasm32"))]
fn distinct_after(source: &str, start: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (at, _) in source.match_indices(start) {
        let rest = source.get(at + start.len()..).unwrap_or_default();
        let inside: String = rest.chars().take_while(|c| *c != ')').take(64).collect();
        let inside = inside.split_whitespace().collect::<Vec<_>>().join("");
        if !found.contains(&inside) {
            found.push(inside);
        }
    }
    found
}

/// The entry points of `source`: the function after each `@vertex`,
/// `@fragment` and `@compute`.
#[cfg(not(target_arch = "wasm32"))]
fn entry_points(source: &str) -> Vec<String> {
    let mut entries = Vec::new();
    for stage in ["@vertex", "@fragment", "@compute"] {
        for (at, _) in source.match_indices(stage) {
            let rest = source.get(at + stage.len()..).unwrap_or_default();
            let Some(fn_at) = rest.find("fn ") else {
                continue;
            };
            let name: String = rest
                .get(fn_at + 3..)
                .unwrap_or_default()
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .take(64)
                .collect();
            if !name.is_empty() {
                entries.push(name);
            }
        }
    }
    entries
}

/// What a module's line says of its source.
#[cfg(not(target_arch = "wasm32"))]
fn describe_source(source: &wgpu::ShaderSource<'_>) -> String {
    #[allow(unreachable_patterns)]
    let text = match source {
        wgpu::ShaderSource::Wgsl(text) => text.as_ref(),
        _ => return "source=not WGSL".to_string(),
    };
    let mut out = format!(
        "wgsl bytes={} fnv={:016x} entries=[{}] builtins=[{}]",
        text.len(),
        fnv1a(text.as_bytes()),
        entry_points(text).join(","),
        distinct_after(text, "@builtin(").join(",")
    );
    let interpolate = distinct_after(text, "@interpolate(");
    if !interpolate.is_empty() {
        let _ = write!(out, " interpolate=[{}]", interpolate.join(";"));
    }
    if text.contains("@invariant") {
        out.push_str(" invariant");
    }
    let multisampled = text.matches("texture_multisampled_2d").count();
    if multisampled > 0 {
        let _ = write!(out, " multisampled_textures={multisampled}");
    }
    for marker in ["textureNumSamples", "discard"] {
        if text.contains(marker) {
            let _ = write!(out, " uses={marker}");
        }
    }
    out
}

/// What a pipeline's line says: its stages, samples, targets and depth.
#[cfg(not(target_arch = "wasm32"))]
fn describe_render_pipeline(tracer: &Tracer, desc: &wgpu::RenderPipelineDescriptor<'_>) -> String {
    let mut out = format!(
        "vs={}@{} buffers={}",
        desc.vertex.entry_point.unwrap_or("(default)"),
        tracer.module_name(desc.vertex.module, desc.vertex.entry_point),
        desc.vertex.buffers.len()
    );
    match &desc.fragment {
        Some(fragment) => {
            let targets = fragment
                .targets
                .iter()
                .map(|t| match t {
                    Some(t) => format!(
                        "{:?}{}{}",
                        t.format,
                        if t.blend.is_some() { "+blend" } else { "" },
                        if t.write_mask.is_empty() {
                            "+nowrite"
                        } else {
                            ""
                        }
                    ),
                    None => "none".to_string(),
                })
                .collect::<Vec<_>>()
                .join(",");
            let _ = write!(
                out,
                " fs={}@{} targets=[{targets}]",
                fragment.entry_point.unwrap_or("(default)"),
                tracer.module_name(fragment.module, fragment.entry_point)
            );
        }
        None => out.push_str(" fs=none"),
    }
    let _ = write!(out, " samples={}", desc.multisample.count);
    if desc.multisample.mask != !0 {
        let _ = write!(out, " mask=0x{:x}", desc.multisample.mask);
    }
    if desc.multisample.alpha_to_coverage_enabled {
        out.push_str(" alpha_to_coverage");
    }
    match &desc.depth_stencil {
        Some(depth) => {
            let _ = write!(
                out,
                " depth={:?}/{:?}/write={:?}{}",
                depth.format,
                depth.depth_compare,
                depth.depth_write_enabled,
                if depth.stencil.is_enabled() {
                    "/stencil"
                } else {
                    ""
                }
            );
        }
        None => out.push_str(" depth=none"),
    }
    let _ = write!(
        out,
        " topology={:?} cull={:?}",
        desc.primitive.topology, desc.primitive.cull_mode
    );
    out
}

/// What a bind group layout's line says: its entries.
#[cfg(not(target_arch = "wasm32"))]
fn describe_layout(desc: &wgpu::BindGroupLayoutDescriptor<'_>) -> String {
    let entries = desc
        .entries
        .iter()
        .map(|e| {
            #[allow(unreachable_patterns)]
            let what = match &e.ty {
                wgpu::BindingType::Buffer { ty, .. } => format!("{ty:?}"),
                wgpu::BindingType::Sampler(kind) => format!("sampler {kind:?}"),
                wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension,
                    multisampled,
                } => format!(
                    "texture {sample_type:?} {view_dimension:?}{}",
                    if *multisampled { " multisampled" } else { "" }
                ),
                wgpu::BindingType::StorageTexture { format, .. } => {
                    format!("storage texture {format:?}")
                }
                _ => "other".to_string(),
            };
            format!("{}:{what}", e.binding)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("entries=[{entries}]")
}

/// Everything in this crate that makes the driver compile or link
/// something goes through here (`tests::nothing_is_made_past_the_trace`
/// holds every source file of the crate to it), so that the trace misses
/// none of it. Without a trace each is the call of the device itself.
pub(crate) trait Traced {
    fn traced_shader_module(&self, desc: wgpu::ShaderModuleDescriptor<'_>) -> wgpu::ShaderModule;
    fn traced_render_pipeline(
        &self,
        desc: &wgpu::RenderPipelineDescriptor<'_>,
    ) -> wgpu::RenderPipeline;
    fn traced_bind_group_layout(
        &self,
        desc: &wgpu::BindGroupLayoutDescriptor<'_>,
    ) -> wgpu::BindGroupLayout;
}

#[cfg(not(target_arch = "wasm32"))]
impl Traced for wgpu::Device {
    fn traced_shader_module(&self, desc: wgpu::ShaderModuleDescriptor<'_>) -> wgpu::ShaderModule {
        let Some(tracer) = tracer() else {
            return self.create_shader_module(desc);
        };
        let label = desc.label;
        let detail = describe_source(&desc.source);
        #[allow(unreachable_patterns)]
        let entries = match &desc.source {
            wgpu::ShaderSource::Wgsl(text) => entry_points(text),
            _ => Vec::new(),
        };
        let (number, module) = tracer.around(
            "shader_module",
            label,
            || detail,
            || self.create_shader_module(desc),
        );
        tracer.remember(&module, number, label, entries);
        module
    }

    fn traced_render_pipeline(
        &self,
        desc: &wgpu::RenderPipelineDescriptor<'_>,
    ) -> wgpu::RenderPipeline {
        let Some(tracer) = tracer() else {
            return self.create_render_pipeline(desc);
        };
        tracer
            .around(
                "render_pipeline",
                desc.label,
                || describe_render_pipeline(tracer, desc),
                || self.create_render_pipeline(desc),
            )
            .1
    }

    fn traced_bind_group_layout(
        &self,
        desc: &wgpu::BindGroupLayoutDescriptor<'_>,
    ) -> wgpu::BindGroupLayout {
        let Some(tracer) = tracer() else {
            return self.create_bind_group_layout(desc);
        };
        tracer
            .around(
                "bind_group_layout",
                desc.label,
                || describe_layout(desc),
                || self.create_bind_group_layout(desc),
            )
            .1
    }
}

#[cfg(target_arch = "wasm32")]
impl Traced for wgpu::Device {
    fn traced_shader_module(&self, desc: wgpu::ShaderModuleDescriptor<'_>) -> wgpu::ShaderModule {
        self.create_shader_module(desc)
    }

    fn traced_render_pipeline(
        &self,
        desc: &wgpu::RenderPipelineDescriptor<'_>,
    ) -> wgpu::RenderPipeline {
        self.create_render_pipeline(desc)
    }

    fn traced_bind_group_layout(
        &self,
        desc: &wgpu::BindGroupLayoutDescriptor<'_>,
    ) -> wgpu::BindGroupLayout {
        self.create_bind_group_layout(desc)
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    /// A file of its own in the temporary directory, removed when dropped.
    struct TempFile(std::path::PathBuf);

    impl TempFile {
        fn new(name: &str) -> TempFile {
            // native clock: the tests are cfg(not(target_arch = "wasm32")).
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            TempFile(std::env::temp_dir().join(format!(
                "craft3d-gpu-trace-{name}-{}-{nanos}.txt",
                std::process::id()
            )))
        }

        fn read(&self) -> String {
            std::fs::read_to_string(&self.0).unwrap_or_default()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// The number, phase and kind of a trace line.
    fn head(line: &str) -> (u64, String, String) {
        let mut words = line.split_whitespace();
        let number = words
            .next()
            .and_then(|w| w.strip_prefix('#'))
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no number: {line}"));
        let phase = words.next().unwrap_or_default().to_string();
        let kind = words.next().unwrap_or_default().to_string();
        (number, phase, kind)
    }

    #[test]
    fn every_begin_is_on_disk_before_the_call_and_pairs_with_its_end() {
        let file = TempFile::new("pairs");
        let tracer = Tracer::open(&file.0, false);
        tracer.info(
            "version",
            "craft3d 0.1.2 (abc, format 9)\nwith a second line",
        );
        let (first, made) = tracer.around(
            "shader_module",
            Some("craft3d scene (scene.wgsl)"),
            || "wgsl bytes=10".to_string(),
            || {
                // What a crash in here would leave: the begin line, whole.
                let so_far = file.read();
                let last = so_far.lines().last().unwrap_or_default().to_string();
                assert!(so_far.ends_with('\n'), "{so_far}");
                assert!(last.contains(" begin shader_module "), "{last}");
                assert!(last.contains("\"craft3d scene (scene.wgsl)\""), "{last}");
                assert!(last.contains("wgsl bytes=10"), "{last}");
                7
            },
        );
        assert_eq!(made, 7);
        let (second, ()) = tracer.around(
            "render_pipeline",
            Some("lines \"flat\""),
            || "samples=4".to_string(),
            || (),
        );
        assert!(second > first);
        tracer.info("draw", "frame 1");
        let (_, ()) = tracer.around("bind_group_layout", None, String::new, || ());

        let text = file.read();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 8, "{text}");
        // Numbered in order, one line per event, nothing over two lines.
        let heads: Vec<_> = lines.iter().map(|l| head(l)).collect();
        assert_eq!(heads[0].1, "info");
        assert_eq!(heads[0].2, "version");
        assert!(
            lines[0].contains("format 9) with a second line"),
            "{}",
            lines[0]
        );
        assert_eq!(heads[5].1, "info");
        // Every begin has exactly one end of the same number, kind and
        // label, after it.
        let mut open = Vec::new();
        for (i, (number, phase, kind)) in heads.iter().enumerate() {
            match phase.as_str() {
                "begin" => open.push((*number, kind.clone(), i)),
                "end" => {
                    let at = open
                        .iter()
                        .position(|(n, ..)| n == number)
                        .unwrap_or_else(|| panic!("an end without a begin: {}", lines[i]));
                    let (_, begun_kind, begun_at) = open.remove(at);
                    assert_eq!(&begun_kind, kind);
                    let label = |l: &str| l.split('"').nth(1).map(str::to_string);
                    assert_eq!(label(lines[begun_at]), label(lines[i]));
                    assert!(lines[i].contains(" took="), "{}", lines[i]);
                }
                "info" => {}
                other => panic!("phase {other}: {}", lines[i]),
            }
        }
        assert!(open.is_empty(), "begins without an end: {open:?}");
        // A quote in a label cannot end it early.
        assert!(lines[3].contains(" \"lines 'flat'\" "), "{}", lines[3]);
        assert_eq!(lines[3].matches('"').count(), 2, "{}", lines[3]);
        assert!(lines[6].contains(" \"(no label)\" "), "{}", lines[6]);
    }

    #[test]
    fn a_trace_appends_to_what_is_there() {
        let file = TempFile::new("append");
        std::fs::write(&file.0, "an earlier run\n").unwrap();
        let tracer = Tracer::open(&file.0, false);
        tracer.info("version", "again");
        let text = file.read();
        assert!(
            text.starts_with("an earlier run\n#1 info version"),
            "{text}"
        );
    }

    #[test]
    fn a_file_that_cannot_be_opened_is_no_failure() {
        let missing = std::env::temp_dir()
            .join("craft3d-no-such-directory-for-a-trace")
            .join("x".repeat(5000))
            .join("trace.txt");
        let tracer = Tracer::open(&missing, false);
        tracer.info("version", "nowhere");
        let (_, made) = tracer.around("render_pipeline", Some("x"), String::new, || 3);
        assert_eq!(made, 3);
        assert!(!missing.exists());
        // A path with a NUL is refused by the system, not by a panic.
        let tracer = Tracer::open(std::path::Path::new("a\0b"), false);
        tracer.info("version", "nowhere");
    }

    #[test]
    fn fields_are_one_line_and_bounded() {
        assert_eq!(field("a\nb\r\tc"), "a b  c");
        let long = "é".repeat(FIELD_CHARS * 3);
        let kept = field(&long);
        assert_eq!(kept.chars().count(), FIELD_CHARS + 1);
        assert!(kept.ends_with('…'));
    }

    #[test]
    fn a_source_is_summed_up() {
        let source = "struct O { @builtin(sample_mask) mask: u32, @builtin(frag_depth) d: f32 }\n\
             @vertex\nfn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) @invariant \
             vec4<f32> {}\n@fragment fn fs_x(@interpolate(perspective, centroid) a: f32) {}\n\
             var t: texture_multisampled_2d<f32>; fn k() { textureNumSamples(t); }";
        let said = describe_source(&wgpu::ShaderSource::Wgsl(source.into()));
        assert!(said.contains("entries=[vs_main,fs_x]"), "{said}");
        assert!(
            said.contains("builtins=[sample_mask,frag_depth,vertex_index,position]"),
            "{said}"
        );
        assert!(
            said.contains("interpolate=[perspective,centroid]"),
            "{said}"
        );
        assert!(said.contains(" invariant"), "{said}");
        assert!(said.contains("multisampled_textures=1"), "{said}");
        assert!(said.contains("uses=textureNumSamples"), "{said}");
        assert!(said.contains(&format!("bytes={}", source.len())), "{said}");
        // Hostile sources: cut off anywhere, nothing but markers.
        for cut in 0..source.len() {
            if let Some(part) = source.get(..cut) {
                let _ = describe_source(&wgpu::ShaderSource::Wgsl(part.into()));
            }
        }
        let _ = describe_source(&wgpu::ShaderSource::Wgsl("@vertex@fragment fn".into()));
        let _ = describe_source(&wgpu::ShaderSource::Wgsl("@builtin(".repeat(1000).into()));
    }

    #[test]
    fn samples_take_one_or_four_and_nothing_else() {
        assert_eq!(parse_samples("1"), Ok(Some(1)));
        assert_eq!(parse_samples(" 4 "), Ok(Some(4)));
        assert_eq!(parse_samples(""), Ok(None));
        assert_eq!(parse_samples("  \t"), Ok(None));
        for bad in [
            "0",
            "2",
            "3",
            "8",
            "-1",
            "four",
            "4x",
            "+4",
            "04",
            "1e0",
            "18446744073709551617",
            "\u{0}",
            "４",
        ] {
            let said = parse_samples(bad).expect_err(bad);
            assert!(said.contains(SAMPLES_VAR), "{said}");
        }
        // A huge value is not repeated whole.
        let huge = "9".repeat(1 << 20);
        let said = parse_samples(&huge).expect_err("huge");
        assert!(said.len() < 400, "{}", said.len());
    }

    #[test]
    fn the_backend_order_is_a_list_of_names() {
        use wgpu::Backend::{Dx12, Gl, Metal, Vulkan};
        assert_eq!(
            parse_backend_order("dx12,vulkan,gl"),
            Ok(Some(vec![Dx12, Vulkan, Gl]))
        );
        assert_eq!(
            parse_backend_order(" GL , Vulkan,,dx12, gl "),
            Ok(Some(vec![Gl, Vulkan, Dx12]))
        );
        assert_eq!(parse_backend_order("metal"), Ok(Some(vec![Metal])));
        assert_eq!(
            parse_backend_order("d3d12,vk,opengl"),
            Ok(Some(vec![Dx12, Vulkan, Gl]))
        );
        assert_eq!(parse_backend_order(""), Ok(None));
        assert_eq!(parse_backend_order("   "), Ok(None));
        for bad in [
            ",",
            ", ,,",
            "dx11",
            "vulkan;gl",
            "dx12 vulkan",
            "webgpu",
            "noop",
            "\u{0}",
            "vulkan,garbage",
        ] {
            let said = parse_backend_order(bad).expect_err(bad);
            assert!(said.contains(BACKEND_ORDER_VAR), "{said}");
        }
        let huge = format!("{}x", "vulkan,".repeat(1 << 18));
        let said = parse_backend_order(&huge).expect_err("huge");
        assert!(said.len() < 400, "{}", said.len());
        let many = "gl,".repeat(1 << 18);
        assert_eq!(parse_backend_order(&many), Ok(Some(vec![Gl])));
        assert_eq!(
            backends_of(&[Dx12, Gl]),
            wgpu::Backends::DX12 | wgpu::Backends::GL
        );
        assert_eq!(backend_list(&[Dx12, Vulkan, Gl]), "dx12,vulkan,gl");
        // WGPU_BACKEND, set at all, chooses as it always did.
        assert_eq!(backend_order(None, None), Ok(None));
        assert_eq!(backend_order(None, Some("vulkan")), Ok(None));
        assert_eq!(backend_order(Some(""), Some("vulkan")), Ok(None));
        assert_eq!(backend_order(Some("dx12"), None), Ok(Some(vec![Dx12])));
        let said = backend_order(Some("dx12"), Some("")).unwrap_err();
        assert!(said.contains("WGPU_BACKEND"), "{said}");
        let said = backend_order(Some("dx12"), Some(&"x".repeat(1 << 20))).unwrap_err();
        assert!(said.len() < 400, "{}", said.len());
        assert!(backend_order(Some("dx13"), None).is_err());
    }

    /// wgpu's own preference: discrete before integrated before the rest
    /// for performance, integrated first for low power.
    #[test]
    fn the_preference_is_wgpus() {
        use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu, Other, VirtualGpu};
        use wgpu::PowerPreference::{HighPerformance, LowPower};
        let order = |power| {
            let mut kinds = [Cpu, VirtualGpu, Other, IntegratedGpu, DiscreteGpu];
            kinds.sort_by_key(|k| preference(*k, power));
            kinds
        };
        assert_eq!(
            order(HighPerformance),
            [DiscreteGpu, IntegratedGpu, Other, VirtualGpu, Cpu]
        );
        assert_eq!(
            order(LowPower),
            [IntegratedGpu, DiscreteGpu, Other, VirtualGpu, Cpu]
        );
    }

    /// Every source file of the crate makes its shader modules, pipelines
    /// and bind group layouts through [`Traced`], so the trace misses none.
    #[test]
    fn nothing_is_made_past_the_trace() {
        let sources = [
            ("cut.rs", include_str!("cut.rs")),
            ("lib.rs", include_str!("lib.rs")),
            ("mesh.rs", include_str!("mesh.rs")),
            ("offscreen.rs", include_str!("offscreen.rs")),
            ("onscreen.rs", include_str!("onscreen.rs")),
            ("peel.rs", include_str!("peel.rs")),
            ("readback.rs", include_str!("readback.rs")),
            ("renderer.rs", include_str!("renderer.rs")),
            ("studio.rs", include_str!("studio.rs")),
            ("variant.rs", include_str!("variant.rs")),
        ];
        let listed: Vec<&str> = sources.iter().map(|(name, _)| *name).collect();
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(&directory).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            if name.ends_with(".rs") && name != "diag.rs" {
                assert!(
                    listed.contains(&name.as_str()),
                    "{name} is not checked here"
                );
            }
        }
        for (name, source) in sources {
            for call in [
                ".create_shader_module(",
                ".create_shader_module_trusted(",
                ".create_render_pipeline(",
                ".create_compute_pipeline(",
                ".create_bind_group_layout(",
            ] {
                assert!(
                    !source.contains(call),
                    "{name} calls {call} directly: go through diag::Traced"
                );
            }
        }
    }
}
