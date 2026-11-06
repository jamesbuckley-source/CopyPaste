//! The onscreen ruler.
//!
//! A topmost, transparent Win32 popup that measures the desktop in physical
//! pixels: distances, boxes, angles and guides. Like the capture overlays it owns
//! the screen from a dedicated thread and paints with GDI; unlike them it stays up
//! until closed and can be locked click-through so other applications remain
//! usable underneath.
//!
//! The geometry and formatting helpers at the top are free of FFI so they can be
//! tested like the capture module's selection rules.

use crate::error::Result;
use crate::settings::Settings;

/// How a measurement is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Physical,
    Dip,
    Both,
}

impl Units {
    fn from_setting(value: &str) -> Self {
        match value {
            "dip" => Units::Dip,
            "both" => Units::Both,
            _ => Units::Physical,
        }
    }

    fn next(self) -> Self {
        match self {
            Units::Physical => Units::Dip,
            Units::Dip => Units::Both,
            Units::Both => Units::Physical,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Units::Physical => "px",
            Units::Dip => "dip",
            Units::Both => "both",
        }
    }
}

/// What the Copy button writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFormat {
    Px,
    Pair,
    Css,
    Json,
}

impl CopyFormat {
    fn from_setting(value: &str) -> Self {
        match value {
            "pair" => CopyFormat::Pair,
            "css" => CopyFormat::Css,
            "json" => CopyFormat::Json,
            _ => CopyFormat::Px,
        }
    }
}

/// One thing the ruler is measuring, in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measurement {
    Distance {
        a: (i32, i32),
        b: (i32, i32),
    },
    Box {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
    Angle {
        vertex: (i32, i32),
        a: (i32, i32),
        b: (i32, i32),
    },
}

impl Measurement {
    /// Width and height of the shape's bounding box.
    pub fn size(self) -> (i32, i32) {
        match self {
            Measurement::Distance { a, b } => ((b.0 - a.0).abs(), (b.1 - a.1).abs()),
            Measurement::Box {
                left,
                top,
                right,
                bottom,
            } => ((right - left).abs(), (bottom - top).abs()),
            Measurement::Angle { vertex, a, b } => (
                (a.0.max(b.0) - vertex.0).abs(),
                (a.1.max(b.1) - vertex.1).abs(),
            ),
        }
    }

    /// The primary number, matching what the toolbar shows.
    pub fn primary(self, units: Units, dpi: u32) -> String {
        match self {
            Measurement::Distance { a, b } => signed_length(f64::from(b.0 - a.0), units, dpi),
            Measurement::Box { .. } => {
                let (width, height) = self.size();
                pair_text(width, height, units, dpi)
            }
            Measurement::Angle { .. } => format!("{}°", number(self.degrees())),
        }
    }

    /// A point inside the shape, used to pick the monitor's DPI for a readout.
    pub fn centre(self) -> (i32, i32) {
        match self {
            Measurement::Distance { a, b } => ((a.0 + b.0) / 2, (a.1 + b.1) / 2),
            Measurement::Box {
                left,
                top,
                right,
                bottom,
            } => ((left + right) / 2, (top + bottom) / 2),
            Measurement::Angle { vertex, .. } => vertex,
        }
    }

    /// The supporting line under the primary value.
    pub fn secondary(self) -> String {
        match self {
            Measurement::Distance { a, b } => {
                let (width, height) = self.size();
                format!("Δ {width} × {height} · {} px", number(distance(a, b)))
            }
            Measurement::Box { .. } => {
                let (width, height) = self.size();
                format!(
                    "area {} px² · perimeter {} px",
                    grouped(i64::from(width) * i64::from(height)),
                    grouped(i64::from(width) * 2 + i64::from(height) * 2)
                )
            }
            Measurement::Angle { vertex, a, b } => {
                format!(
                    "rays {} px · {} px",
                    number(distance(vertex, a)),
                    number(distance(vertex, b))
                )
            }
        }
    }

    /// The angle between the two rays, in degrees; zero for other shapes.
    pub fn degrees(self) -> f64 {
        match self {
            Measurement::Angle { vertex, a, b } => angle_degrees(vertex, a, b),
            _ => 0.0,
        }
    }

    /// What the Copy button writes, in the chosen format.
    pub fn formatted(self, units: Units, format: CopyFormat, dpi: u32) -> String {
        let (width, height) = self.size();
        match format {
            CopyFormat::Px => match self {
                Measurement::Distance { a, b } => signed_length(f64::from(b.0 - a.0), units, dpi),
                Measurement::Box { .. } => pair_text(width, height, units, dpi),
                Measurement::Angle { .. } => format!("{}°", number(self.degrees())),
            },
            CopyFormat::Pair => match self {
                Measurement::Angle { .. } => format!("{}°", number(self.degrees())),
                _ => pair_text(width, height, units, dpi),
            },
            CopyFormat::Css => match self {
                Measurement::Angle { .. } => {
                    format!("transform: rotate({}deg);", number(self.degrees()))
                }
                Measurement::Distance { .. } | Measurement::Box { .. } => format!(
                    "width: {}; height: {};",
                    css_length(width, units, dpi),
                    css_length(height, units, dpi)
                ),
            },
            CopyFormat::Json => match self {
                Measurement::Angle { .. } => {
                    format!("{{\"degrees\":{}}}", number(self.degrees()))
                }
                _ => {
                    let (w, h, unit) = numeric(width, height, units, dpi);
                    format!("{{\"width\":{w},\"height\":{h},\"unit\":\"{unit}\"}}")
                }
            },
        }
    }
}

/// A guide dragged out of an edge ruler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guide {
    Vertical(i32),
    Horizontal(i32),
}

/// Why a point moved: the source the ruler snapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    Point,
    Guide,
    Window,
    Edge,
    Grid,
}

impl SnapKind {
    fn label(self) -> &'static str {
        match self {
            SnapKind::Point => "point",
            SnapKind::Guide => "guide",
            SnapKind::Window => "window",
            SnapKind::Edge => "edge",
            SnapKind::Grid => "grid",
        }
    }
}

/// Straight-line length between two points.
pub fn distance(a: (i32, i32), b: (i32, i32)) -> f64 {
    let dx = f64::from(b.0 - a.0);
    let dy = f64::from(b.1 - a.1);
    (dx * dx + dy * dy).sqrt()
}

/// The angle at `vertex` between the rays to `a` and `b`, in degrees.
pub fn angle_degrees(vertex: (i32, i32), a: (i32, i32), b: (i32, i32)) -> f64 {
    let u = (f64::from(a.0 - vertex.0), f64::from(a.1 - vertex.1));
    let v = (f64::from(b.0 - vertex.0), f64::from(b.1 - vertex.1));
    let lengths = (u.0 * u.0 + u.1 * u.1).sqrt() * (v.0 * v.0 + v.1 * v.1).sqrt();
    if lengths == 0.0 {
        return 0.0;
    }
    let cosine = ((u.0 * v.0 + u.1 * v.1) / lengths).clamp(-1.0, 1.0);
    cosine.acos().to_degrees()
}

/// The nearest candidate within `threshold`, preferring the smallest distance.
pub fn nearest(
    candidates: impl IntoIterator<Item = i32>,
    target: i32,
    threshold: i32,
) -> Option<i32> {
    candidates
        .into_iter()
        .filter(|candidate| (candidate - target).abs() <= threshold)
        .min_by_key(|candidate| (candidate - target).abs())
}

/// Rounds to the nearest multiple of `step`; `step` of zero leaves the value alone.
pub fn grid_value(value: i32, step: i32) -> i32 {
    if step <= 0 {
        return value;
    }
    (value + step / 2).div_euclid(step) * step
}

/// Spacing, in physical pixels, between numbered ruler ticks.
///
/// Labels must stay far enough apart to be read at the monitor's scale, so a
/// 200% screen gets a coarser step than a 100% one.
pub fn tick_step(scale: f64) -> i32 {
    for step in [10, 25, 50, 100, 200, 500, 1_000] {
        if f64::from(step) * scale >= 48.0 {
            return step;
        }
    }
    1_000
}

/// A physical pixel length as device-independent pixels.
pub fn dip(px: f64, dpi: u32) -> f64 {
    if dpi == 0 {
        return px;
    }
    px * 96.0 / f64::from(dpi)
}

fn number(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded - rounded.round()).abs() < 0.05 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded:.1}")
    }
}

fn grouped(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0 { format!("-{out}") } else { out }
}

fn unit_text(px: f64, units: Units, dpi: u32) -> String {
    match units {
        Units::Physical => format!("{} px", number(px)),
        Units::Dip => format!("{} dip", number(dip(px, dpi))),
        Units::Both => format!("{} px · {} dip", number(px), number(dip(px, dpi))),
    }
}

fn signed_length(value: f64, units: Units, dpi: u32) -> String {
    unit_text(value, units, dpi)
}

fn pair_text(width: i32, height: i32, units: Units, dpi: u32) -> String {
    match units {
        Units::Both => format!(
            "{width} × {height} px · {} × {} dip",
            number(dip(f64::from(width), dpi)),
            number(dip(f64::from(height), dpi))
        ),
        _ => {
            let (w, h, unit) = numeric(width, height, units, dpi);
            format!("{w} × {h} {unit}")
        }
    }
}

fn numeric(width: i32, height: i32, units: Units, dpi: u32) -> (String, String, &'static str) {
    match units {
        Units::Dip => (
            number(dip(f64::from(width), dpi)),
            number(dip(f64::from(height), dpi)),
            "dip",
        ),
        Units::Physical | Units::Both => (width.to_string(), height.to_string(), "px"),
    }
}

fn css_length(value: i32, units: Units, dpi: u32) -> String {
    match units {
        Units::Dip => format!("{}dip", number(dip(f64::from(value), dpi))),
        Units::Physical | Units::Both => format!("{value}px"),
    }
}

/// Opens the ruler, or unlocks/closes the one already showing.
pub fn open(app: tauri::AppHandle, settings: Settings) -> Result<()> {
    #[cfg(windows)]
    {
        platform::open(app, settings)
    }
    #[cfg(not(windows))]
    {
        let _ = (app, settings);
        Err(crate::error::AppError::msg(
            "the onscreen ruler is only available on Windows",
        ))
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::time::Instant;

    use tauri::{AppHandle, Manager};
    use windows::Win32::Foundation::{
        COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen,
        CreateSolidBrush, DEFAULT_GUI_FONT, DeleteDC, EndPaint, FillRect, FrameRect,
        GetStockObject, HDC, InvalidateRect, LineTo, MONITOR_DEFAULTTONEAREST, MonitorFromPoint,
        MoveToEx, PS_SOLID, SRCCOPY, SelectObject, SetBkMode, SetTextColor, SetViewportOrgEx,
        TRANSPARENT, TextOutW,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, HOT_KEY_MODIFIERS, RegisterHotKey, ReleaseCapture, SetCapture,
        UnregisterHotKey, VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_F5, VK_LEFT,
        VK_MENU, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GWL_EXSTYLE,
        GetClientRect, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
        GetWindowRect, GetWindowThreadProcessId, HWND_TOPMOST, IDC_CROSS, IDC_SIZEALL, IDC_SIZENS,
        IDC_SIZEWE, IsIconic, IsWindowVisible, KillTimer, LWA_ALPHA, LWA_COLORKEY, LoadCursorW,
        MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SetCursor, SetForegroundWindow, SetLayeredWindowAttributes,
        SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, WM_APP,
        WM_DESTROY, WM_DISPLAYCHANGE, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WM_TIMER, WNDCLASSW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
        WS_POPUP,
    };
    use windows::core::PCWSTR;

    use crate::AppState;
    use crate::capture::{self, Rect};
    use crate::settings::Settings;

    use super::{CopyFormat, Guide, Measurement, SnapKind, Units, grid_value, nearest, tick_step};

    const CLASS_NAME: windows::core::PCWSTR = windows::core::w!("KeptRulerSurface");
    const INPUT_CLASS: windows::core::PCWSTR = windows::core::w!("KeptRulerInput");
    /// Sent by a second trigger to unlock or close the overlay it already found.
    const WM_RULER_TOGGLE: u32 = WM_APP + 1;
    /// One-shot timer that clears the "Copied" confirmation.
    const TIMER_COPY: usize = 1;
    /// Registered only while the ruler is locked, so Esc can give the screen back.
    const ESC_HOTKEY_ID: i32 = 0x0E5C;
    /// The colour key: this exact colour is painted as the window background and
    /// becomes transparent, letting the desktop show through.
    const KEY: COLORREF = COLORREF(0x00ff_00ff);
    /// Stands in for "opening" while the worker thread creates its window.
    const OPENING: isize = -1;
    /// A snapshot larger than this is skipped; pixel-edge snapping is a nicety,
    /// not worth a hundred megabytes of RAM on a wall of monitors.
    const MAX_SNAPSHOT_PIXELS: usize = 24_000_000;

    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    static SURFACE: AtomicIsize = AtomicIsize::new(0);
    static STATE: Mutex<Option<Ruler>> = Mutex::new(None);

    fn surface_window() -> Option<HWND> {
        let handle = SURFACE.load(Ordering::Relaxed);
        (handle != 0).then_some(HWND(handle as *mut core::ffi::c_void))
    }

    fn state() -> std::sync::MutexGuard<'static, Option<Ruler>> {
        STATE.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Tool {
        Select,
        Distance,
        Box,
        Angle,
    }

    impl Tool {
        fn label(self) -> &'static str {
            match self {
                Tool::Select => "Select",
                Tool::Distance => "Distance",
                Tool::Box => "Box",
                Tool::Angle => "Angle",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Button {
        Select,
        Distance,
        Box,
        Angle,
        Guides,
        Snap,
        Units,
        Copy,
        Lock,
        Close,
    }

    const BUTTONS: [Button; 10] = [
        Button::Select,
        Button::Distance,
        Button::Box,
        Button::Angle,
        Button::Guides,
        Button::Snap,
        Button::Units,
        Button::Copy,
        Button::Lock,
        Button::Close,
    ];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Axis {
        X,
        Y,
    }

    #[derive(Debug, Clone, Copy)]
    enum Pending {
        Distance { a: (i32, i32), b: (i32, i32) },
        Box { a: (i32, i32), b: (i32, i32) },
    }

    #[derive(Debug, Clone, Copy)]
    enum Drag {
        None,
        New,
        Move { index: usize, last: (i32, i32) },
        Handle { index: usize, handle: usize },
        Guide { index: usize },
        Toolbar { offset: (i32, i32) },
    }

    struct Snapshot {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    }

    impl Snapshot {
        fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
            let at = ((y * self.width + x) * 4) as usize;
            [
                self.rgba[at],
                self.rgba[at + 1],
                self.rgba[at + 2],
                self.rgba[at + 3],
            ]
        }
    }

    struct Ruler {
        app: AppHandle,
        settings: Settings,
        units: Units,
        copy_format: CopyFormat,
        tool: Tool,
        snap: bool,
        snap_grid: i32,
        snap_threshold: i32,
        edge_rulers: bool,
        guides_enabled: bool,
        bounds: Rect,
        scale: f64,
        measurements: Vec<Measurement>,
        active: Option<usize>,
        guides: Vec<Guide>,
        selected_guide: Option<usize>,
        pending: Option<Pending>,
        angle_points: Vec<(i32, i32)>,
        pointer: (i32, i32),
        snap_hint: Option<SnapKind>,
        hover: Option<Button>,
        drag: Drag,
        toolbar: Rect,
        copied_at: Option<Instant>,
        locked: bool,
        snapshot: Option<Snapshot>,
        windows: Vec<Rect>,
        previous: isize,
    }

    impl Ruler {
        fn new(
            app: AppHandle,
            settings: Settings,
            bounds: Rect,
            snapshot: Option<Snapshot>,
            windows: Vec<Rect>,
            scale: f64,
            previous: isize,
        ) -> Self {
            let (width, height) = toolbar_size(scale);
            let toolbar = match settings.ruler_toolbar_pos {
                Some(pos) => Rect::new(pos.x, pos.y, pos.x + width, pos.y + height),
                None => Rect::new(
                    bounds.left + (bounds.width() - width) / 2,
                    bounds.top + (40.0 * scale) as i32,
                    bounds.left + (bounds.width() - width) / 2 + width,
                    bounds.top + (40.0 * scale) as i32 + height,
                ),
            };
            let toolbar = clamp_toolbar(toolbar, bounds);
            Ruler {
                units: Units::from_setting(&settings.ruler_units),
                copy_format: CopyFormat::from_setting(&settings.ruler_copy_format),
                tool: Tool::Distance,
                snap: settings.ruler_snap,
                snap_grid: settings.ruler_snap_grid as i32,
                snap_threshold: settings.ruler_snap_threshold.min(32) as i32,
                edge_rulers: settings.ruler_edge_rulers,
                guides_enabled: settings.ruler_guides,
                app,
                settings,
                bounds,
                scale,
                measurements: Vec::new(),
                active: None,
                guides: Vec::new(),
                selected_guide: None,
                pending: None,
                angle_points: Vec::new(),
                pointer: (
                    bounds.left + bounds.width() / 2,
                    bounds.top + bounds.height() / 2,
                ),
                snap_hint: None,
                hover: None,
                drag: Drag::None,
                toolbar,
                copied_at: None,
                locked: false,
                snapshot,
                windows,
                previous,
            }
        }

        fn active_measurement(&self) -> Option<Measurement> {
            self.active
                .and_then(|index| self.measurements.get(index).copied())
        }

        fn shown(&self) -> Option<Measurement> {
            match self.pending {
                Some(Pending::Distance { a, b }) => Some(Measurement::Distance { a, b }),
                Some(Pending::Box { a, b }) => Some(Measurement::Box {
                    left: a.0.min(b.0),
                    top: a.1.min(b.1),
                    right: a.0.max(b.0),
                    bottom: a.1.max(b.1),
                }),
                _ => self.active_measurement(),
            }
        }

        fn dpi_at(&self, point: (i32, i32)) -> u32 {
            dpi_at(point)
        }

        /// All the points another measurement already occupies, for point snapping.
        fn anchors(&self) -> Vec<(i32, i32)> {
            let mut out = Vec::new();
            for measurement in &self.measurements {
                match *measurement {
                    Measurement::Distance { a, b } => out.extend([a, b]),
                    Measurement::Box {
                        left,
                        top,
                        right,
                        bottom,
                    } => out.extend([(left, top), (right, top), (left, bottom), (right, bottom)]),
                    Measurement::Angle { vertex, a, b } => out.extend([vertex, a, b]),
                }
            }
            out
        }

        fn snap_point(&self, point: (i32, i32)) -> ((i32, i32), Option<SnapKind>) {
            if !self.snap || key_down(VK_MENU) {
                return (point, None);
            }
            let threshold = self.snap_threshold;
            // A corner on an existing measurement wins over an axis line, because
            // it pins both coordinates at once.
            if let Some(anchor) = self
                .anchors()
                .into_iter()
                .filter(|anchor| {
                    (anchor.0 - point.0).abs() <= threshold
                        && (anchor.1 - point.1).abs() <= threshold
                })
                .min_by_key(|anchor| (anchor.0 - point.0).abs().max((anchor.1 - point.1).abs()))
            {
                return (anchor, Some(SnapKind::Point));
            }

            let mut out = point;
            let mut kind: Option<SnapKind> = None;

            let (x, x_kind) = self.snap_axis(point.0, Axis::X, threshold);
            if let Some(value) = x {
                out.0 = value;
                kind = x_kind;
            }
            let (y, y_kind) = self.snap_axis(point.1, Axis::Y, threshold);
            if let Some(value) = y {
                out.1 = value;
                kind = match (kind, y_kind) {
                    (Some(current), Some(next)) => Some(current.min(next)),
                    (None, next) => next,
                    (current, None) => current,
                };
            }
            out.0 = out.0.clamp(self.bounds.left, self.bounds.right - 1);
            out.1 = out.1.clamp(self.bounds.top, self.bounds.bottom - 1);
            (out, kind)
        }

        fn snap_axis(
            &self,
            target: i32,
            axis: Axis,
            threshold: i32,
        ) -> (Option<i32>, Option<SnapKind>) {
            let guide = nearest(
                self.guides.iter().filter_map(|guide| match (axis, guide) {
                    (Axis::X, Guide::Vertical(value)) => Some(*value),
                    (Axis::Y, Guide::Horizontal(value)) => Some(*value),
                    _ => None,
                }),
                target,
                threshold,
            );
            if let Some(value) = guide {
                return (Some(value), Some(SnapKind::Guide));
            }

            let window = nearest(
                self.windows.iter().flat_map(|rect| match axis {
                    Axis::X => [rect.left, rect.right],
                    Axis::Y => [rect.top, rect.bottom],
                }),
                target,
                threshold,
            );
            if let Some(value) = window {
                return (Some(value), Some(SnapKind::Window));
            }

            if let Some(value) = self.pixel_edge(target, axis, threshold) {
                return (Some(value), Some(SnapKind::Edge));
            }

            if self.snap_grid > 0 {
                let value = grid_value(target, self.snap_grid);
                if (value - target).abs() <= threshold {
                    return (Some(value), Some(SnapKind::Grid));
                }
            }
            (None, None)
        }

        /// The strongest colour edge within reach of `target` on the snapshot.
        fn pixel_edge(&self, target: i32, axis: Axis, threshold: i32) -> Option<i32> {
            let snapshot = self.snapshot.as_ref()?;
            let start = match axis {
                Axis::X => target - self.bounds.left,
                Axis::Y => target - self.bounds.top,
            };
            let limit = match axis {
                Axis::X => snapshot.width as i32,
                Axis::Y => snapshot.height as i32,
            };
            if start < 0 || start >= limit {
                return None;
            }
            let from = (start - threshold).max(0);
            let to = (start + threshold).min(limit - 2);
            if to <= from {
                return None;
            }
            let fixed = match axis {
                Axis::X => (self.pointer.1 - self.bounds.top).clamp(0, limit - 1) as u32,
                Axis::Y => (self.pointer.0 - self.bounds.left).clamp(0, limit - 1) as u32,
            };
            let mut best = 0i32;
            let mut at = None;
            for index in from..=to {
                let (first, second) = match axis {
                    Axis::X => (
                        snapshot.pixel(index as u32, fixed),
                        snapshot.pixel(index as u32 + 1, fixed),
                    ),
                    Axis::Y => (
                        snapshot.pixel(fixed, index as u32),
                        snapshot.pixel(fixed, index as u32 + 1),
                    ),
                };
                let contrast = (0..3)
                    .map(|channel| (i32::from(first[channel]) - i32::from(second[channel])).abs())
                    .sum::<i32>();
                if contrast > best {
                    best = contrast;
                    at = Some(index + 1);
                }
            }
            (best >= 48).then(|| match axis {
                Axis::X => self.bounds.left + at.unwrap(),
                Axis::Y => self.bounds.top + at.unwrap(),
            })
        }

        fn measurement_hit(&self, point: (i32, i32)) -> Option<(usize, Option<usize>)> {
            for (index, measurement) in self.measurements.iter().enumerate().rev() {
                match *measurement {
                    Measurement::Distance { a, b } => {
                        if near(point, a, 8) {
                            return Some((index, Some(0)));
                        }
                        if near(point, b, 8) {
                            return Some((index, Some(1)));
                        }
                        if to_segment(point, a, b) <= 6.0 {
                            return Some((index, None));
                        }
                    }
                    Measurement::Box {
                        left,
                        top,
                        right,
                        bottom,
                    } => {
                        for (handle, at) in box_handles(left, top, right, bottom).iter().enumerate()
                        {
                            if near(point, *at, 8) {
                                return Some((index, Some(handle)));
                            }
                        }
                        if point.0 >= left
                            && point.0 <= right
                            && point.1 >= top
                            && point.1 <= bottom
                        {
                            return Some((index, None));
                        }
                    }
                    Measurement::Angle { vertex, a, b } => {
                        for (handle, at) in [vertex, a, b].iter().enumerate() {
                            if near(point, *at, 8) {
                                return Some((index, Some(handle)));
                            }
                        }
                        if to_segment(point, vertex, a) <= 6.0
                            || to_segment(point, vertex, b) <= 6.0
                        {
                            return Some((index, None));
                        }
                    }
                }
            }
            None
        }

        fn guide_hit(&self, point: (i32, i32)) -> Option<usize> {
            self.guides.iter().position(|guide| match guide {
                Guide::Vertical(x) => (point.0 - x).abs() <= 4,
                Guide::Horizontal(y) => (point.1 - y).abs() <= 4,
            })
        }

        fn button_at(&self, point: (i32, i32)) -> Option<Button> {
            if !self.toolbar.contains(point) {
                return None;
            }
            let pad = pad(self.scale);
            let grip = grip_width(self.scale);
            let button = button_width(self.scale);
            let left = self.toolbar.left + pad + grip;
            if point.0 < left {
                return None;
            }
            let index = (point.0 - left) / button;
            if index < 0 || index as usize >= BUTTONS.len() {
                return None;
            }
            Some(BUTTONS[index as usize])
        }

        fn cursor_for(&self, point: (i32, i32)) -> PCWSTR {
            if self.toolbar.contains(point) {
                return IDC_CROSS;
            }
            if let Some((index, handle)) = self.measurement_hit(point) {
                if let Some(handle) = handle {
                    if matches!(self.measurements.get(index), Some(Measurement::Box { .. })) {
                        return match handle {
                            1 | 5 => IDC_SIZENS,
                            3 | 7 => IDC_SIZEWE,
                            _ => IDC_SIZEALL,
                        };
                    }
                    return IDC_SIZEALL;
                }
                return IDC_SIZEALL;
            }
            if self.guide_hit(point).is_some() {
                return IDC_SIZEWE;
            }
            IDC_CROSS
        }

        fn commit(&mut self) {
            match self.pending.take() {
                Some(Pending::Distance { a, b }) => {
                    if (a.0 - b.0).abs() + (a.1 - b.1).abs() > 2 {
                        self.measurements.push(Measurement::Distance { a, b });
                        self.active = Some(self.measurements.len() - 1);
                    }
                }
                Some(Pending::Box { a, b }) => {
                    let (left, right) = (a.0.min(b.0), a.0.max(b.0));
                    let (top, bottom) = (a.1.min(b.1), a.1.max(b.1));
                    if right - left > 1 && bottom - top > 1 {
                        self.measurements.push(Measurement::Box {
                            left,
                            top,
                            right,
                            bottom,
                        });
                        self.active = Some(self.measurements.len() - 1);
                    }
                }
                _ => {}
            }
        }

        fn copy(&mut self) {
            let Some(measurement) = self.active_measurement() else {
                return;
            };
            let centre = measurement.centre();
            let text = measurement.formatted(self.units, self.copy_format, self.dpi_at(centre));
            match crate::commands::record_text_clip(&self.app, &text, "Screen ruler") {
                Ok(_) => self.copied_at = Some(Instant::now()),
                Err(e) => log::warn!("could not copy the ruler measurement: {e}"),
            }
        }

        fn persist(&self, patch: serde_json::Value) {
            let store = self.app.state::<AppState>().store.clone();
            if let Err(e) = store.update_settings(&patch) {
                log::warn!("could not save the ruler setting: {e}");
            }
        }
    }

    fn pad(scale: f64) -> i32 {
        (6.0 * scale).round() as i32
    }

    fn grip_width(scale: f64) -> i32 {
        (16.0 * scale).round() as i32
    }

    fn button_width(scale: f64) -> i32 {
        (32.0 * scale).round() as i32
    }

    fn readout_width(scale: f64) -> i32 {
        (196.0 * scale).round() as i32
    }

    fn toolbar_size(scale: f64) -> (i32, i32) {
        let width = pad(scale) * 2
            + grip_width(scale)
            + button_width(scale) * BUTTONS.len() as i32
            + readout_width(scale);
        (width, (34.0 * scale).round() as i32)
    }

    fn clamp_toolbar(mut toolbar: Rect, bounds: Rect) -> Rect {
        let visible = 40;
        let (width, height) = (toolbar.width(), toolbar.height());
        let low = bounds.left - width + visible;
        let high = bounds.right - visible;
        toolbar.left = if low <= high {
            toolbar.left.clamp(low, high)
        } else {
            low
        };
        let low = bounds.top;
        let high = (bounds.bottom - visible).max(low);
        toolbar.top = toolbar.top.clamp(low, high);
        toolbar.right = toolbar.left + width;
        toolbar.bottom = toolbar.top + height;
        toolbar
    }

    fn near(point: (i32, i32), target: (i32, i32), reach: i32) -> bool {
        (point.0 - target.0).abs() <= reach && (point.1 - target.1).abs() <= reach
    }

    fn to_segment(point: (i32, i32), a: (i32, i32), b: (i32, i32)) -> f64 {
        let (px, py) = (f64::from(point.0), f64::from(point.1));
        let (ax, ay) = (f64::from(a.0), f64::from(a.1));
        let (bx, by) = (f64::from(b.0), f64::from(b.1));
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx * dx + dy * dy;
        if length == 0.0 {
            return super::distance(point, a);
        }
        let t = (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, 1.0);
        super::distance(
            point,
            ((ax + dx * t).round() as i32, (ay + dy * t).round() as i32),
        )
    }

    fn box_handles(left: i32, top: i32, right: i32, bottom: i32) -> [(i32, i32); 8] {
        let mid_x = left + (right - left) / 2;
        let mid_y = top + (bottom - top) / 2;
        [
            (left, top),
            (mid_x, top),
            (right, top),
            (right, mid_y),
            (right, bottom),
            (mid_x, bottom),
            (left, bottom),
            (left, mid_y),
        ]
    }

    fn key_down(key: VIRTUAL_KEY) -> bool {
        unsafe { GetKeyState(i32::from(key.0)) < 0 }
    }

    /// Projects `point` onto the nearest of the eight compass directions.
    fn constrained(anchor: (i32, i32), point: (i32, i32)) -> (i32, i32) {
        let dx = point.0 - anchor.0;
        let dy = point.1 - anchor.1;
        if dx.abs() >= dy.abs() * 2 {
            (point.0, anchor.1)
        } else if dy.abs() >= dx.abs() * 2 {
            (anchor.0, point.1)
        } else {
            let reach = dx.abs().max(dy.abs());
            (
                anchor.0 + reach * if dx < 0 { -1 } else { 1 },
                anchor.1 + reach * if dy < 0 { -1 } else { 1 },
            )
        }
    }

    /// The corner that makes the drag from `anchor` a square.
    fn square_point(anchor: (i32, i32), point: (i32, i32)) -> (i32, i32) {
        let dx = point.0 - anchor.0;
        let dy = point.1 - anchor.1;
        let reach = dx.abs().max(dy.abs());
        (
            anchor.0 + reach * if dx < 0 { -1 } else { 1 },
            anchor.1 + reach * if dy < 0 { -1 } else { 1 },
        )
    }

    /// Rounds the angle between the two rays to `step` degrees, keeping the ray.
    fn snap_angle(vertex: (i32, i32), a: (i32, i32), b: (i32, i32), step: f64) -> (i32, i32) {
        let length = super::distance(vertex, b);
        if length == 0.0 {
            return b;
        }
        let base = f64::from(a.1 - vertex.1).atan2(f64::from(a.0 - vertex.0));
        let current = f64::from(b.1 - vertex.1).atan2(f64::from(b.0 - vertex.0));
        let delta = (current - base).to_degrees();
        let angle = base + (delta / step).round() * step * (std::f64::consts::PI / 180.0);
        (
            vertex.0 + (length * angle.cos()).round() as i32,
            vertex.1 + (length * angle.sin()).round() as i32,
        )
    }

    fn dpi_at(point: (i32, i32)) -> u32 {
        unsafe {
            let monitor = MonitorFromPoint(
                POINT {
                    x: point.0,
                    y: point.1,
                },
                MONITOR_DEFAULTTONEAREST,
            );
            let mut x = 0u32;
            let mut y = 0u32;
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() && x > 0 {
                x
            } else {
                96
            }
        }
    }

    fn monitor_scale(bounds: Rect) -> f64 {
        let centre = (
            bounds.left + bounds.width() / 2,
            bounds.top + bounds.height() / 2,
        );
        (f64::from(dpi_at(centre)) / 96.0).clamp(1.0, 2.0)
    }

    /// Top-level windows that are worth snapping to, in screen coordinates.
    fn collect_windows(bounds: Rect) -> Vec<Rect> {
        static FOUND: Mutex<Vec<Rect>> = Mutex::new(Vec::new());
        FOUND.lock().unwrap_or_else(|e| e.into_inner()).clear();

        unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> windows::core::BOOL {
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                    return windows::core::BOOL(1);
                }
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_err() {
                    return windows::core::BOOL(1);
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == std::process::id() {
                    return windows::core::BOOL(1);
                }
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                if style & WS_EX_TOOLWINDOW.0 != 0 {
                    return windows::core::BOOL(1);
                }
                let candidate = Rect::new(rect.left, rect.top, rect.right, rect.bottom);
                if candidate.is_empty() || candidate.width() < 8 || candidate.height() < 8 {
                    return windows::core::BOOL(1);
                }
                FOUND
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(candidate);
            }
            windows::core::BOOL(1)
        }

        let _ = unsafe { EnumWindows(Some(each), LPARAM(0)) };
        let mut seen = FOUND.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // The shell's desktop window covers everything and would make every edge
        // snap to a monitor boundary; it is already covered by the virtual bounds.
        seen.retain(|rect| *rect != bounds);
        seen
    }

    fn take_snapshot(bounds: Rect) -> Option<Snapshot> {
        let pixels = (bounds.width() as usize).checked_mul(bounds.height() as usize)?;
        if pixels == 0 || pixels > MAX_SNAPSHOT_PIXELS {
            log::info!("ruler pixel snapping disabled for a {pixels} pixel desktop");
            return None;
        }
        match capture::screen_pixels(bounds) {
            Ok((width, height, rgba)) => Some(Snapshot {
                width,
                height,
                rgba,
            }),
            Err(e) => {
                log::warn!("could not snapshot the desktop for ruler snapping: {e}");
                None
            }
        }
    }

    pub fn open(app: AppHandle, settings: Settings) -> crate::error::Result<()> {
        let current = WINDOW.load(Ordering::Relaxed);
        if current != 0 {
            if current > 0 {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(current as *mut core::ffi::c_void)),
                        WM_RULER_TOGGLE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            return Ok(());
        }
        if WINDOW
            .compare_exchange(0, OPENING, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(());
        }

        // The window must not measure itself, so it goes away before the overlay
        // appears, exactly as the capture tools hide it.
        if let Some(window) = crate::window::window(&app) {
            let _ = window.hide();
        }
        if let Some(settings_window) = app.get_webview_window(crate::window::SETTINGS_LABEL) {
            let _ = settings_window.hide();
        }

        std::thread::Builder::new()
            .name("onscreen-ruler".into())
            .spawn(move || {
                // Let the hidden window finish disappearing before the overlay
                // snapshots the desktop, as the capture tools do.
                std::thread::sleep(std::time::Duration::from_millis(180));
                // A panic in the message loop must not leave the "open" flag set,
                // or every later trigger would think a ruler is still showing.
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(app, settings)));
                match outcome {
                    Ok(Err(e)) => log::warn!("the onscreen ruler stopped: {e}"),
                    Err(_) => log::error!("the onscreen ruler panicked"),
                    Ok(Ok(())) => {}
                }
                WINDOW.store(0, Ordering::SeqCst);
            })
            .map_err(|e| {
                WINDOW.store(0, Ordering::SeqCst);
                crate::error::AppError::msg(format!("could not start the onscreen ruler: {e}"))
            })?;
        Ok(())
    }

    fn run(app: AppHandle, settings: Settings) -> crate::error::Result<()> {
        // The primary monitor, not the whole desktop: the ruler is one window,
        // and stretched across two screens of different sizes and scales its
        // guides, snapping and toolbar all land in the wrong place.
        let bounds = capture::primary_bounds();
        if bounds.is_empty() {
            return Err(crate::error::AppError::msg("there is no screen to measure"));
        }
        let snapshot = take_snapshot(bounds);
        let windows = collect_windows(bounds);
        // Keep the toolbar usable on a small or heavily scaled screen: its full
        // 200% width would otherwise run off both edges.
        let scale = monitor_scale(bounds).min((f64::from(bounds.width()) / 620.0).max(0.75));
        let previous = unsafe { GetForegroundWindow() };

        unsafe {
            let module = GetModuleHandleW(None)
                .map_err(|e| crate::error::AppError::msg(format!("no module handle: {e}")))?;
            let instance = HINSTANCE(module.0);
            let surface_class = WNDCLASSW {
                style: Default::default(),
                lpfnWndProc: Some(surface_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: Default::default(),
                hCursor: Default::default(),
                hbrBackground: Default::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: CLASS_NAME,
            };
            let input_class = WNDCLASSW {
                lpfnWndProc: Some(input_proc),
                lpszClassName: INPUT_CLASS,
                ..surface_class
            };
            // Registration fails when the class already exists, which is the
            // normal case for a second ruler; that is not an error.
            RegisterClassW(&surface_class);
            RegisterClassW(&input_class);

            // Two windows, because a color-keyed layered window lets mouse
            // messages through its transparent pixels. The surface paints the
            // chrome and is always click-through; the input window is an almost
            // invisible (1/255 alpha) sheet underneath that owns the mouse and
            // keyboard everywhere on the screen.
            let input = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                INPUT_CLASS,
                INPUT_CLASS,
                WS_POPUP,
                bounds.left,
                bounds.top,
                bounds.width(),
                bounds.height(),
                None,
                None,
                Some(instance),
                None,
            )
            .map_err(|e| crate::error::AppError::msg(format!("could not open the ruler: {e}")))?;

            let surface = match CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE,
                CLASS_NAME,
                CLASS_NAME,
                WS_POPUP,
                bounds.left,
                bounds.top,
                bounds.width(),
                bounds.height(),
                None,
                None,
                Some(instance),
                None,
            ) {
                Ok(surface) => surface,
                Err(e) => {
                    let _ = DestroyWindow(input);
                    return Err(crate::error::AppError::msg(format!(
                        "could not open the ruler: {e}"
                    )));
                }
            };

            let _ = SetLayeredWindowAttributes(input, COLORREF(0), 1, LWA_ALPHA);
            let _ = SetLayeredWindowAttributes(surface, KEY, 0, LWA_COLORKEY);
            SURFACE.store(surface.0 as isize, Ordering::SeqCst);
            *state() = Some(Ruler::new(
                app,
                settings,
                bounds,
                snapshot,
                windows,
                scale,
                previous.0 as isize,
            ));

            let _ = ShowWindow(surface, SW_SHOW);
            let _ = SetWindowPos(
                surface,
                Some(HWND_TOPMOST),
                bounds.left,
                bounds.top,
                bounds.width(),
                bounds.height(),
                SWP_NOACTIVATE,
            );
            let _ = ShowWindow(input, SW_SHOW);
            // The input has to sit directly below the surface so clicks in the
            // surface's transparent pixels land on it rather than on the desktop.
            let _ = SetWindowPos(
                input,
                Some(surface),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            WINDOW.store(input.0 as isize, Ordering::SeqCst);

            // Interactive mode owns the keyboard so Esc and the tool keys work.
            // Locked mode removes this focus again; see `toggle_lock`.
            let _ = SetForegroundWindow(input);

            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            if let Some(surface) = surface_window() {
                let _ = DestroyWindow(surface);
            }
            SURFACE.store(0, Ordering::SeqCst);
            *state() = None;
            if !previous.is_invalid() {
                let _ = SetForegroundWindow(previous);
            }
        }
        Ok(())
    }

    fn invalidate(_hwnd: HWND, bounds: Rect, area: Rect) {
        let Some(surface) = surface_window() else {
            return;
        };
        if let Some(area) = area.intersect(bounds) {
            let client = RECT {
                left: area.left - bounds.left,
                top: area.top - bounds.top,
                right: area.right - bounds.left,
                bottom: area.bottom - bounds.top,
            };
            unsafe {
                let _ = InvalidateRect(Some(surface), Some(&client), false);
            }
        }
    }

    fn invalidate_measurement(hwnd: HWND, bounds: Rect, measurement: Measurement) {
        invalidate(hwnd, bounds, measurement_bounds(measurement).inflate(60));
    }

    fn measurement_bounds(measurement: Measurement) -> Rect {
        match measurement {
            Measurement::Distance { a, b } => {
                Rect::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
            }
            Measurement::Box {
                left,
                top,
                right,
                bottom,
            } => Rect::new(
                left.min(right),
                top.min(bottom),
                left.max(right),
                top.max(bottom),
            ),
            Measurement::Angle { vertex, a, b } => Rect::new(
                vertex.0.min(a.0).min(b.0),
                vertex.1.min(a.1).min(b.1),
                vertex.0.max(a.0).max(b.0),
                vertex.1.max(a.1).max(b.1),
            ),
        }
        .inflate(8)
    }

    unsafe extern "system" fn surface_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_PAINT => {
                unsafe { paint(hwnd) };
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    /// The near-invisible sheet that owns input for the whole screen.
    unsafe extern "system" fn input_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_PAINT => {
                unsafe { paint_input(hwnd) };
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SETCURSOR => {
                unsafe {
                    let mut cursor = POINT::default();
                    let _ = GetCursorPos(&mut cursor);
                    let point = (cursor.x, cursor.y);
                    let wanted = state()
                        .as_ref()
                        .map(|ruler| ruler.cursor_for(point))
                        .unwrap_or(IDC_CROSS);
                    if let Ok(handle) = LoadCursorW(None, wanted) {
                        SetCursor(Some(handle));
                    }
                }
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                unsafe { on_move(hwnd, lparam) };
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                unsafe { on_down(hwnd, lparam) };
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                // A quick drag can release without a final move; fold the release
                // point in first so the measurement ends where the pointer did.
                unsafe {
                    on_move(hwnd, lparam);
                    on_up(hwnd);
                }
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                unsafe { on_key(hwnd, wparam.0 as u32) };
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == TIMER_COPY {
                    let bounds = {
                        let mut guard = state();
                        if let Some(ruler) = guard.as_mut() {
                            ruler.copied_at = None;
                            let area = ruler.toolbar;
                            Some((ruler.bounds, area))
                        } else {
                            None
                        }
                    };
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_COPY);
                    }
                    if let Some((bounds, area)) = bounds {
                        invalidate(hwnd, bounds, area);
                    }
                }
                LRESULT(0)
            }
            WM_RULER_TOGGLE => {
                unsafe { hotkey_toggle(hwnd) };
                LRESULT(0)
            }
            WM_HOTKEY => {
                if wparam.0 as i32 == ESC_HOTKEY_ID {
                    unsafe { unlock(hwnd) };
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                // The overlay geometry would be stale after a resolution or
                // monitor change; closing is safer than painting over nothing.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = unsafe { UnregisterHotKey(Some(hwnd), ESC_HOTKEY_ID) };
                if let Some(surface) = surface_window() {
                    let _ = unsafe { DestroyWindow(surface) };
                }
                SURFACE.store(0, Ordering::SeqCst);
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    fn point_of(lparam: LPARAM) -> (i32, i32) {
        let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
        (x, y)
    }

    fn screen_point(lparam: LPARAM) -> ((i32, i32), Rect) {
        let client = point_of(lparam);
        let bounds = {
            let guard = state();
            guard
                .as_ref()
                .map(|ruler| ruler.bounds)
                .unwrap_or(Rect::new(0, 0, 0, 0))
        };
        ((bounds.left + client.0, bounds.top + client.1), bounds)
    }

    unsafe fn on_move(hwnd: HWND, lparam: LPARAM) {
        let (point, bounds) = screen_point(lparam);
        let mut dirty: Vec<Rect> = Vec::new();
        {
            let mut guard = state();
            let Some(ruler) = guard.as_mut() else {
                return;
            };
            let previous = ruler.pointer;
            ruler.pointer = point;
            let tracking = ruler.pending.is_some()
                || !matches!(ruler.drag, Drag::None)
                || !ruler.angle_points.is_empty();
            if tracking {
                dirty.push(Rect::new(
                    previous.0 - 200,
                    previous.1 - 40,
                    previous.0 + 200,
                    previous.1 + 60,
                ));
                dirty.push(Rect::new(
                    point.0 - 200,
                    point.1 - 40,
                    point.0 + 200,
                    point.1 + 60,
                ));
            }
            if ruler.pending.is_some() || !matches!(ruler.drag, Drag::None) {
                // The toolbar's readout follows a measurement while it is dragged.
                dirty.push(ruler.toolbar);
            }

            // The pointer cross-highlight lives in the edge strips, which the
            // pointer box above does not necessarily touch when the cursor is
            // mid-screen. Clear the old marks explicitly.
            let thickness = strip(ruler);
            for (x, _) in [previous, point] {
                dirty.push(Rect::new(x - 2, bounds.top, x + 2, bounds.top + thickness));
                dirty.push(Rect::new(
                    x - 2,
                    bounds.bottom - thickness,
                    x + 2,
                    bounds.bottom,
                ));
            }
            for (_, y) in [previous, point] {
                dirty.push(Rect::new(
                    bounds.left,
                    y - 2,
                    bounds.left + thickness,
                    y + 2,
                ));
                dirty.push(Rect::new(
                    bounds.right - thickness,
                    y - 2,
                    bounds.right,
                    y + 2,
                ));
            }

            if matches!(ruler.drag, Drag::New) {
                if let Some(old) = ruler.shown() {
                    dirty.push(measurement_bounds(old).inflate(60));
                }
            }
            if !ruler.angle_points.is_empty() {
                let vertex = ruler.angle_points[0];
                let a = *ruler.angle_points.get(1).unwrap_or(&previous);
                dirty.push(
                    measurement_bounds(Measurement::Angle {
                        vertex,
                        a,
                        b: previous,
                    })
                    .inflate(60),
                );
            }

            match ruler.drag {
                Drag::New => match ruler.pending.as_ref() {
                    Some(Pending::Distance { a, .. }) => {
                        let (snapped, kind) = ruler.snap_point(point);
                        ruler.snap_hint = kind;
                        let a = *a;
                        let b = if key_down(VK_SHIFT) {
                            constrained(a, snapped)
                        } else {
                            snapped
                        };
                        ruler.pending = Some(Pending::Distance { a, b });
                        dirty.push(measurement_bounds(Measurement::Distance { a, b }).inflate(60));
                    }
                    Some(Pending::Box { a, .. }) => {
                        let (snapped, kind) = ruler.snap_point(point);
                        ruler.snap_hint = kind;
                        let a = *a;
                        let b = if key_down(VK_SHIFT) {
                            square_point(a, snapped)
                        } else {
                            snapped
                        };
                        ruler.pending = Some(Pending::Box { a, b });
                        dirty.push(
                            measurement_bounds(Measurement::Box {
                                left: a.0.min(snapped.0),
                                top: a.1.min(snapped.1),
                                right: a.0.max(snapped.0),
                                bottom: a.1.max(snapped.1),
                            })
                            .inflate(60),
                        );
                    }
                    _ => {}
                },
                Drag::Handle { index, handle } => {
                    let (snapped, kind) = ruler.snap_point(point);
                    ruler.snap_hint = kind;
                    if let Some(measurement) = ruler.measurements.get(index).copied() {
                        dirty.push(measurement_bounds(measurement).inflate(60));
                        let next = resized(measurement, handle, snapped, key_down(VK_SHIFT));
                        ruler.measurements[index] = next;
                        ruler.active = Some(index);
                        dirty.push(measurement_bounds(next).inflate(60));
                    }
                }
                Drag::Move { index, last } => {
                    let delta = (point.0 - last.0, point.1 - last.1);
                    if let Some(measurement) = ruler.measurements.get(index).copied() {
                        dirty.push(measurement_bounds(measurement).inflate(60));
                        let next = moved(measurement, delta, ruler.bounds);
                        ruler.measurements[index] = next;
                        ruler.active = Some(index);
                        ruler.drag = Drag::Move { index, last: point };
                        dirty.push(measurement_bounds(next).inflate(60));
                    }
                }
                Drag::Guide { index } => {
                    if let Some(guide) = ruler.guides.get(index).copied() {
                        dirty.push(guide_bounds(guide, ruler.bounds));
                        let next = match guide {
                            Guide::Vertical(_) => Guide::Vertical(point.0),
                            Guide::Horizontal(_) => Guide::Horizontal(point.1),
                        };
                        ruler.guides[index] = next;
                        dirty.push(guide_bounds(next, ruler.bounds));
                    }
                }
                Drag::Toolbar { offset } => {
                    let old = ruler.toolbar;
                    let (width, height) = (old.width(), old.height());
                    let left = point.0 - offset.0;
                    let top = point.1 - offset.1;
                    ruler.toolbar = clamp_toolbar(
                        Rect::new(left, top, left + width, top + height),
                        ruler.bounds,
                    );
                    dirty.push(old);
                    dirty.push(ruler.toolbar);
                }
                Drag::None => {}
            }
            if ruler.angle_points.len() == 1 || ruler.angle_points.len() == 2 {
                let vertex = ruler.angle_points[0];
                let a = *ruler.angle_points.get(1).unwrap_or(&point);
                dirty.push(
                    measurement_bounds(Measurement::Angle {
                        vertex,
                        a,
                        b: point,
                    })
                    .inflate(60),
                );
            }
            if matches!(ruler.drag, Drag::None) && ruler.pending.is_none() {
                let next = ruler.button_at(point);
                if next != ruler.hover {
                    ruler.hover = next;
                    dirty.push(hover_area(ruler));
                    dirty.push(ruler.toolbar);
                }
            } else if ruler.hover.take().is_some() {
                dirty.push(hover_area(ruler));
                dirty.push(ruler.toolbar);
            }
            for area in dirty {
                invalidate(hwnd, bounds, area);
            }
        }
    }

    unsafe fn on_down(hwnd: HWND, lparam: LPARAM) {
        let (point, bounds) = screen_point(lparam);
        let mut dirty: Vec<Rect> = Vec::new();
        {
            let mut guard = state();
            let Some(ruler) = guard.as_mut() else {
                return;
            };
            ruler.pointer = point;
            if ruler.hover.take().is_some() {
                dirty.push(hover_area(ruler));
                dirty.push(ruler.toolbar);
            }
            let previous_active = ruler.active;

            if let Some(button) = ruler.button_at(point) {
                handle_button(hwnd, ruler, button);
                dirty.push(ruler.toolbar.inflate(2));
            } else if ruler.guides_enabled && ruler.edge_rulers && in_edge_strip(ruler, point) {
                let horizontal = point.1 - ruler.bounds.top < strip(ruler)
                    || ruler.bounds.bottom - point.1 < strip(ruler);
                let guide = if horizontal {
                    Guide::Horizontal(point.1)
                } else {
                    Guide::Vertical(point.0)
                };
                ruler.guides.push(guide);
                ruler.selected_guide = Some(ruler.guides.len() - 1);
                ruler.drag = Drag::Guide {
                    index: ruler.guides.len() - 1,
                };
                dirty.push(guide_bounds(guide, ruler.bounds));
            } else if ruler.toolbar.contains(point) {
                // The grip and empty toolbar space move the panel.
                ruler.drag = Drag::Toolbar {
                    offset: (point.0 - ruler.toolbar.left, point.1 - ruler.toolbar.top),
                };
            } else if let Some((index, handle)) = ruler.measurement_hit(point) {
                ruler.active = Some(index);
                ruler.selected_guide = None;
                match handle {
                    Some(handle) => ruler.drag = Drag::Handle { index, handle },
                    None => ruler.drag = Drag::Move { index, last: point },
                }
                if let Some(measurement) = ruler.measurements.get(index).copied() {
                    dirty.push(measurement_bounds(measurement).inflate(60));
                }
            } else if let Some(index) = ruler.guide_hit(point) {
                ruler.selected_guide = Some(index);
                ruler.active = None;
                ruler.drag = Drag::Guide { index };
                if let Some(guide) = ruler.guides.get(index).copied() {
                    dirty.push(guide_bounds(guide, ruler.bounds));
                }
            } else {
                ruler.active = None;
                ruler.selected_guide = None;
                match ruler.tool {
                    Tool::Select => {}
                    Tool::Distance => {
                        let (snapped, kind) = ruler.snap_point(point);
                        ruler.snap_hint = kind;
                        ruler.pending = Some(Pending::Distance {
                            a: snapped,
                            b: snapped,
                        });
                        ruler.drag = Drag::New;
                    }
                    Tool::Box => {
                        let (snapped, kind) = ruler.snap_point(point);
                        ruler.snap_hint = kind;
                        ruler.pending = Some(Pending::Box {
                            a: snapped,
                            b: snapped,
                        });
                        ruler.drag = Drag::New;
                    }
                    Tool::Angle => {
                        let (snapped, kind) = ruler.snap_point(point);
                        ruler.snap_hint = kind;
                        ruler.angle_points.push(snapped);
                        if ruler.angle_points.len() == 3 {
                            let vertex = ruler.angle_points[0];
                            let a = ruler.angle_points[1];
                            let mut b = ruler.angle_points[2];
                            if key_down(VK_SHIFT) {
                                b = snap_angle(vertex, a, b, 15.0);
                            }
                            ruler.measurements.push(Measurement::Angle { vertex, a, b });
                            ruler.active = Some(ruler.measurements.len() - 1);
                            ruler.angle_points.clear();
                            dirty.push(
                                measurement_bounds(Measurement::Angle { vertex, a, b }).inflate(60),
                            );
                        }
                    }
                }
                dirty.push(Rect::new(
                    point.0 - 200,
                    point.1 - 40,
                    point.0 + 200,
                    point.1 + 60,
                ));
            }
            if previous_active != ruler.active {
                if let Some(index) = previous_active {
                    if let Some(measurement) = ruler.measurements.get(index).copied() {
                        dirty.push(measurement_bounds(measurement).inflate(60));
                    }
                }
            }
            // The readout follows the active measurement, so the toolbar always
            // needs a repaint after a click.
            dirty.push(ruler.toolbar);
            unsafe {
                let _ = SetCapture(hwnd);
            }
            for area in dirty {
                invalidate(hwnd, bounds, area);
            }
        }
    }

    unsafe fn on_up(hwnd: HWND) {
        {
            let mut guard = state();
            let Some(ruler) = guard.as_mut() else {
                return;
            };
            let bounds = ruler.bounds;
            let area = match ruler.pending {
                Some(Pending::Distance { a, b }) => {
                    measurement_bounds(Measurement::Distance { a, b })
                }
                Some(Pending::Box { a, b }) => measurement_bounds(Measurement::Box {
                    left: a.0.min(b.0),
                    top: a.1.min(b.1),
                    right: a.0.max(b.0),
                    bottom: a.1.max(b.1),
                }),
                _ => Rect::new(
                    ruler.pointer.0 - 200,
                    ruler.pointer.1 - 40,
                    ruler.pointer.0 + 200,
                    ruler.pointer.1 + 60,
                ),
            };
            match ruler.drag {
                Drag::New => ruler.commit(),
                Drag::Toolbar { .. } => {
                    let position = ruler.toolbar;
                    ruler.persist(serde_json::json!({
                        "rulerToolbarPos": { "x": position.left, "y": position.top }
                    }));
                }
                Drag::Guide { index } => {
                    if let Some(guide) = ruler.guides.get(index).copied() {
                        let inside = match guide {
                            Guide::Vertical(x) => {
                                x - bounds.left < strip(ruler) || bounds.right - x < strip(ruler)
                            }
                            Guide::Horizontal(y) => {
                                y - bounds.top < strip(ruler) || bounds.bottom - y < strip(ruler)
                            }
                        };
                        if inside {
                            ruler.guides.remove(index);
                            ruler.selected_guide = None;
                        }
                    }
                }
                _ => {}
            }
            ruler.drag = Drag::None;
            ruler.snap_hint = None;
            invalidate(hwnd, bounds, area.inflate(60));
            invalidate(hwnd, bounds, ruler.toolbar);
        }
        unsafe {
            let _ = ReleaseCapture();
        }
    }

    fn strip(ruler: &Ruler) -> i32 {
        (18.0 * ruler.scale).round() as i32
    }

    fn in_edge_strip(ruler: &Ruler, point: (i32, i32)) -> bool {
        let edge = strip(ruler);
        point.1 - ruler.bounds.top < edge
            || ruler.bounds.bottom - point.1 < edge
            || point.0 - ruler.bounds.left < edge
            || ruler.bounds.right - point.0 < edge
    }

    fn guide_bounds(guide: Guide, bounds: Rect) -> Rect {
        match guide {
            Guide::Vertical(x) => Rect::new(x - 3, bounds.top, x + 3, bounds.bottom),
            Guide::Horizontal(y) => Rect::new(bounds.left, y - 3, bounds.right, y + 3),
        }
    }

    fn resized(
        measurement: Measurement,
        handle: usize,
        point: (i32, i32),
        square: bool,
    ) -> Measurement {
        match measurement {
            Measurement::Distance { a, b } => {
                if handle == 0 {
                    Measurement::Distance { a: point, b }
                } else {
                    Measurement::Distance { a, b: point }
                }
            }
            Measurement::Box {
                left,
                top,
                right,
                bottom,
            } => {
                let (mut left, mut top, mut right, mut bottom) = (left, top, right, bottom);
                match handle {
                    0 => {
                        left = point.0;
                        top = point.1;
                    }
                    1 => top = point.1,
                    2 => {
                        right = point.0;
                        top = point.1;
                    }
                    3 => right = point.0,
                    4 => {
                        right = point.0;
                        bottom = point.1;
                    }
                    5 => bottom = point.1,
                    6 => {
                        left = point.0;
                        bottom = point.1;
                    }
                    _ => left = point.0,
                }
                if square {
                    let side = (right - left).abs().max((bottom - top).abs());
                    if handle == 0 || handle == 6 || handle == 7 {
                        left = right - side;
                    } else {
                        right = left + side;
                    }
                    if handle == 0 || handle == 1 || handle == 2 {
                        top = bottom - side;
                    } else {
                        bottom = top + side;
                    }
                }
                Measurement::Box {
                    left: left.min(right),
                    top: top.min(bottom),
                    right: left.max(right),
                    bottom: top.max(bottom),
                }
            }
            Measurement::Angle { vertex, a, b } => match handle {
                0 => Measurement::Angle {
                    vertex: point,
                    a,
                    b,
                },
                1 => Measurement::Angle {
                    vertex,
                    a: point,
                    b,
                },
                _ => Measurement::Angle {
                    vertex,
                    a,
                    b: point,
                },
            },
        }
    }

    fn moved(measurement: Measurement, delta: (i32, i32), bounds: Rect) -> Measurement {
        let clamp = |point: (i32, i32)| {
            (
                (point.0 + delta.0).clamp(bounds.left, bounds.right),
                (point.1 + delta.1).clamp(bounds.top, bounds.bottom),
            )
        };
        match measurement {
            Measurement::Distance { a, b } => Measurement::Distance {
                a: clamp(a),
                b: clamp(b),
            },
            Measurement::Box {
                left,
                top,
                right,
                bottom,
            } => {
                let width = right - left;
                let height = bottom - top;
                let left = (left + delta.0).clamp(bounds.left, bounds.right - width);
                let top = (top + delta.1).clamp(bounds.top, bounds.bottom - height);
                Measurement::Box {
                    left,
                    top,
                    right: left + width,
                    bottom: top + height,
                }
            }
            Measurement::Angle { vertex, a, b } => Measurement::Angle {
                vertex: clamp(vertex),
                a: clamp(a),
                b: clamp(b),
            },
        }
    }

    fn handle_button(hwnd: HWND, ruler: &mut Ruler, button: Button) {
        match button {
            Button::Select => ruler.tool = Tool::Select,
            Button::Distance => ruler.tool = Tool::Distance,
            Button::Box => ruler.tool = Tool::Box,
            Button::Angle => {
                ruler.tool = Tool::Angle;
                ruler.angle_points.clear();
            }
            Button::Guides => {
                ruler.guides_enabled = !ruler.guides_enabled;
                ruler.persist(serde_json::json!({
                    "rulerGuides": ruler.guides_enabled
                }));
            }
            Button::Snap => {
                ruler.snap = !ruler.snap;
                ruler.persist(serde_json::json!({ "rulerSnap": ruler.snap }));
            }
            Button::Units => {
                ruler.units = ruler.units.next();
                ruler.persist(serde_json::json!({
                    "rulerUnits": ruler.units.label()
                }));
            }
            Button::Copy => {
                ruler.copy();
                if ruler.copied_at.is_some() {
                    unsafe {
                        let _ = SetTimer(Some(hwnd), TIMER_COPY, 1_200, None);
                    }
                }
            }
            Button::Lock => {
                ruler.locked = !ruler.locked;
                ruler.hover = None;
                let locked = ruler.locked;
                unsafe { apply_lock(hwnd, locked, ruler.bounds, ruler.toolbar, ruler.previous) };
            }
            Button::Close => unsafe {
                let _ = DestroyWindow(hwnd);
            },
        }
    }

    /// The hotkey and tray item unlock a locked ruler, and close a live one.
    ///
    /// Locking is the toolbar's job; from the keyboard the useful pair is
    /// "give me my screen back" and "put the ruler away".
    unsafe fn hotkey_toggle(hwnd: HWND) {
        let locked = {
            let guard = state();
            guard.as_ref().map(|ruler| ruler.locked)
        };
        match locked {
            Some(true) => unsafe { toggle_lock(hwnd) },
            Some(false) => unsafe {
                let _ = DestroyWindow(hwnd);
            },
            None => {}
        }
    }

    /// Esc while locked gives the screen back without closing the ruler; the
    /// next Esc reaches the now-focused input window and closes it normally.
    unsafe fn unlock(hwnd: HWND) {
        let target = {
            let mut guard = state();
            let Some(ruler) = guard.as_mut() else {
                return;
            };
            if !ruler.locked {
                return;
            }
            ruler.locked = false;
            ruler.hover = None;
            (ruler.bounds, ruler.toolbar, ruler.previous)
        };
        unsafe { apply_lock(hwnd, false, target.0, target.1, target.2) };
    }

    unsafe fn toggle_lock(hwnd: HWND) {
        let target = {
            let mut guard = state();
            let Some(ruler) = guard.as_mut() else {
                return;
            };
            ruler.locked = !ruler.locked;
            ruler.hover = None;
            (ruler.locked, ruler.bounds, ruler.toolbar, ruler.previous)
        };
        unsafe { apply_lock(hwnd, target.0, target.1, target.2, target.3) };
    }

    /// Applies the window styles for `locked` without touching the shared state.
    ///
    /// Kept apart from [`toggle_lock`] because a toolbar click already holds the
    /// state lock; taking it again here would deadlock the overlay thread.
    unsafe fn apply_lock(hwnd: HWND, locked: bool, bounds: Rect, area: Rect, previous: isize) {
        unsafe {
            // While locked the input window is click-through and unfocused, so
            // Esc has to be caught system-wide. Registering only Esc keeps every
            // other key going to the application underneath.
            if locked {
                if let Err(e) = RegisterHotKey(
                    Some(hwnd),
                    ESC_HOTKEY_ID,
                    HOT_KEY_MODIFIERS(0),
                    u32::from(VK_ESCAPE.0),
                ) {
                    log::warn!("could not register Esc to unlock the ruler: {e}");
                }
            } else {
                let _ = UnregisterHotKey(Some(hwnd), ESC_HOTKEY_ID);
            }
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let transparent = (WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0) as isize;
            let next = if locked {
                ex | transparent
            } else {
                ex & !transparent
            };
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
            // The input sheet always sits directly below the painting surface.
            if let Some(surface) = surface_window() {
                let _ = SetWindowPos(
                    hwnd,
                    Some(surface),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            // Locking hands the screen back to the application underneath;
            // unlocking takes it again so the next measurement works.
            if locked {
                if previous != 0 {
                    let _ = SetForegroundWindow(HWND(previous as *mut core::ffi::c_void));
                }
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            } else {
                let _ = SetForegroundWindow(hwnd);
            }
        }
        invalidate(
            hwnd,
            bounds,
            Rect::new(
                area.left - 200,
                area.top - 30,
                area.right + 200,
                area.bottom + 30,
            ),
        );
    }

    unsafe fn on_key(hwnd: HWND, key: u32) {
        let shift = key_down(VK_SHIFT);
        let step = if shift { 10 } else { 1 };
        match key {
            k if k == u32::from(VK_ESCAPE.0) => unsafe {
                let _ = DestroyWindow(hwnd);
            },
            k if k == u32::from(VK_F5.0) => unsafe {
                let surface = surface_window();
                if let Some(surface) = surface {
                    let _ = ShowWindow(surface, SW_HIDE);
                }
                let _ = ShowWindow(hwnd, SW_HIDE);
                std::thread::sleep(std::time::Duration::from_millis(90));
                let refreshed = {
                    let mut guard = state();
                    guard.as_mut().and_then(|ruler| {
                        take_snapshot(ruler.bounds).map(|snapshot| {
                            ruler.snapshot = Some(snapshot);
                            ruler.windows = collect_windows(ruler.bounds);
                            ruler.bounds
                        })
                    })
                };
                if let Some(surface) = surface {
                    let _ = ShowWindow(surface, SW_SHOW);
                }
                let _ = ShowWindow(hwnd, SW_SHOW);
                if let Some(surface) = surface {
                    let _ = SetWindowPos(
                        hwnd,
                        Some(surface),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                let _ = SetForegroundWindow(hwnd);
                if let Some(bounds) = refreshed {
                    invalidate(hwnd, bounds, bounds);
                }
            },
            k if k == u32::from(VK_BACK.0) || k == u32::from(VK_DELETE.0) => {
                let mut guard = state();
                if let Some(ruler) = guard.as_mut() {
                    if let Some(index) = ruler.selected_guide.take() {
                        ruler.guides.remove(index);
                        invalidate(hwnd, ruler.bounds, ruler.bounds);
                    } else if let Some(index) = ruler.active.take() {
                        if index < ruler.measurements.len() {
                            let removed = ruler.measurements.remove(index);
                            invalidate_measurement(hwnd, ruler.bounds, removed);
                            invalidate(hwnd, ruler.bounds, ruler.toolbar);
                        }
                    }
                }
            }
            k if k == u32::from(VK_TAB.0) => {
                let mut guard = state();
                if let Some(ruler) = guard.as_mut() {
                    let count = ruler.measurements.len();
                    if count > 0 {
                        let next = match ruler.active {
                            Some(index) => {
                                if shift {
                                    (index + count - 1) % count
                                } else {
                                    (index + 1) % count
                                }
                            }
                            None => 0,
                        };
                        let previous = ruler.active;
                        ruler.active = Some(next);
                        let bounds = ruler.bounds;
                        if let Some(index) = previous {
                            if let Some(measurement) = ruler.measurements.get(index).copied() {
                                invalidate_measurement(hwnd, bounds, measurement);
                            }
                        }
                        if let Some(measurement) = ruler.measurements.get(next).copied() {
                            invalidate_measurement(hwnd, bounds, measurement);
                        }
                    }
                }
            }
            k if k == u32::from(VK_LEFT.0)
                || k == u32::from(VK_RIGHT.0)
                || k == u32::from(VK_UP.0)
                || k == u32::from(VK_DOWN.0) =>
            {
                let delta = match key {
                    k if k == u32::from(VK_LEFT.0) => (-step, 0),
                    k if k == u32::from(VK_RIGHT.0) => (step, 0),
                    k if k == u32::from(VK_UP.0) => (0, -step),
                    _ => (0, step),
                };
                let mut guard = state();
                if let Some(ruler) = guard.as_mut() {
                    if let Some(index) = ruler.selected_guide {
                        if let Some(guide) = ruler.guides.get(index).copied() {
                            let next = match guide {
                                Guide::Vertical(x) => Guide::Vertical(x + delta.0),
                                Guide::Horizontal(y) => Guide::Horizontal(y + delta.1),
                            };
                            let bounds = ruler.bounds;
                            ruler.guides[index] = next;
                            invalidate(hwnd, bounds, guide_bounds(next, bounds));
                        }
                    } else if let Some(index) = ruler.active {
                        if let Some(measurement) = ruler.measurements.get(index).copied() {
                            let next = moved(measurement, delta, ruler.bounds);
                            let bounds = ruler.bounds;
                            ruler.measurements[index] = next;
                            invalidate_measurement(hwnd, bounds, measurement);
                            invalidate_measurement(hwnd, bounds, next);
                        }
                    }
                }
            }
            0x56 => set_tool(hwnd, Tool::Select),
            0x44 => set_tool(hwnd, Tool::Distance),
            0x42 => set_tool(hwnd, Tool::Box),
            0x41 => set_tool(hwnd, Tool::Angle),
            0x47 => {
                let mut guard = state();
                if let Some(ruler) = guard.as_mut() {
                    ruler.guides_enabled = !ruler.guides_enabled;
                    let area = ruler.toolbar;
                    let bounds = ruler.bounds;
                    invalidate(hwnd, bounds, area);
                }
            }
            0x53 => {
                let mut guard = state();
                if let Some(ruler) = guard.as_mut() {
                    ruler.snap = !ruler.snap;
                    let area = ruler.toolbar;
                    let bounds = ruler.bounds;
                    invalidate(hwnd, bounds, area);
                }
            }
            0x4c => unsafe { toggle_lock(hwnd) },
            0x43 => {
                let mut guard = state();
                if let Some(ruler) = guard.as_mut() {
                    ruler.copy();
                    let area = ruler.toolbar;
                    let bounds = ruler.bounds;
                    invalidate(hwnd, bounds, area);
                    if ruler.copied_at.is_some() {
                        unsafe {
                            let _ = SetTimer(Some(hwnd), TIMER_COPY, 1_200, None);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn set_tool(hwnd: HWND, tool: Tool) {
        let mut guard = state();
        if let Some(ruler) = guard.as_mut() {
            ruler.tool = tool;
            ruler.angle_points.clear();
            let area = ruler.toolbar;
            let bounds = ruler.bounds;
            invalidate(hwnd, bounds, area);
        }
    }

    // ---------------------------------------------------------------- painting

    struct Palette {
        panel: COLORREF,
        panel2: COLORREF,
        panel3: COLORREF,
        edge: COLORREF,
        ink: COLORREF,
        ink_dim: COLORREF,
        ink_faint: COLORREF,
        accent: COLORREF,
        accent_hi: COLORREF,
    }

    impl Palette {
        fn new(settings: &Settings) -> Self {
            let theme = &settings.theme.colors;
            Palette {
                panel: colorref(&theme.panel),
                panel2: colorref(&theme.panel2),
                panel3: colorref(&theme.panel3),
                edge: colorref(&theme.edge),
                ink: colorref(&theme.ink),
                ink_dim: colorref(&theme.ink_dim),
                ink_faint: colorref(&theme.ink_faint),
                accent: colorref(&theme.accent),
                accent_hi: colorref(&theme.accent_hi),
            }
        }
    }

    fn colorref(hex: &str) -> COLORREF {
        let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x808080);
        // COLORREF stores blue in the high byte.
        COLORREF(((value & 0xff) << 16) | (value & 0x00ff_00) | ((value >> 16) & 0xff))
    }

    /// Paints the input sheet. It is only 1/255 opaque, so its colour is
    /// immaterial visually; a black fill just keeps the surface deterministic.
    unsafe fn paint_input(hwnd: HWND) {
        let mut paint = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(hwnd, &mut paint) };
        if dc.is_invalid() {
            return;
        }
        let mut client = RECT::default();
        let _ = unsafe { GetClientRect(hwnd, &mut client) };
        let update = if paint.rcPaint.right <= paint.rcPaint.left
            || paint.rcPaint.bottom <= paint.rcPaint.top
        {
            client
        } else {
            paint.rcPaint
        };
        unsafe {
            let black = CreateSolidBrush(COLORREF(0));
            FillRect(dc, &update, black);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(black.into());
            let _ = EndPaint(hwnd, &paint);
        }
    }

    unsafe fn paint(hwnd: HWND) {
        let mut paint = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(hwnd, &mut paint) };
        if dc.is_invalid() {
            return;
        }
        let mut client = RECT::default();
        let _ = unsafe { GetClientRect(hwnd, &mut client) };
        // A paint with no update rectangle can happen while the window is being
        // shown or restyled; paint everything in that case rather than leave the
        // window at whatever happened to be in it.
        let update = if paint.rcPaint.right <= paint.rcPaint.left
            || paint.rcPaint.bottom <= paint.rcPaint.top
        {
            client
        } else {
            paint.rcPaint
        };
        let width = (update.right - update.left).max(1);
        let height = (update.bottom - update.top).max(1);

        // Compose off-screen and blit once. Clearing the colour-keyed background
        // and then drawing directly made the toolbar and its buttons flash for a
        // frame on every invalidation; a single blit removes that intermediate.
        let memory = unsafe { CreateCompatibleDC(Some(dc)) };
        let bitmap = unsafe { CreateCompatibleBitmap(dc, width, height) };
        if memory.is_invalid() || bitmap.is_invalid() {
            if !memory.is_invalid() {
                let _ = unsafe { DeleteDC(memory) };
            }
            let _ = unsafe { EndPaint(hwnd, &paint) };
            return;
        }
        let old_bitmap = unsafe { SelectObject(memory, bitmap.into()) };
        let _ = unsafe { SetViewportOrgEx(memory, -update.left, -update.top, None) };

        unsafe {
            let key = CreateSolidBrush(KEY);
            FillRect(memory, &update, key);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(key.into());
        }
        if let Some(ruler) = state().as_ref() {
            let palette = Palette::new(&ruler.settings);
            unsafe {
                paint_rulers(memory, ruler, &palette, update);
                paint_guides(memory, ruler, &palette);
                for (index, measurement) in ruler.measurements.iter().enumerate() {
                    let selected = ruler.active == Some(index);
                    paint_measurement(memory, ruler, *measurement, selected, &palette);
                }
                if let Some(measurement) = ruler.shown() {
                    if ruler.pending.is_some() {
                        paint_measurement(memory, ruler, measurement, true, &palette);
                    }
                }
                paint_angle_preview(memory, ruler, &palette);
                paint_toolbar(memory, ruler, &palette);
                paint_snap_hint(memory, ruler, &palette);
            }
        }

        unsafe {
            // Drop the drawing offset before reading the bitmap back: BitBlt
            // source coordinates go through the memory DC's transform too.
            let _ = SetViewportOrgEx(memory, 0, 0, None);
            let _ = BitBlt(
                dc,
                update.left,
                update.top,
                width,
                height,
                Some(memory),
                0,
                0,
                SRCCOPY,
            );
            SelectObject(memory, old_bitmap);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            let _ = EndPaint(hwnd, &paint);
        }
    }

    unsafe fn fill(dc: HDC, rect: RECT, color: COLORREF) {
        unsafe {
            let brush = CreateSolidBrush(color);
            FillRect(dc, &rect, brush);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush.into());
        }
    }

    unsafe fn frame(dc: HDC, rect: RECT, color: COLORREF) {
        unsafe {
            let brush = CreateSolidBrush(color);
            FrameRect(dc, &rect, brush);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush.into());
        }
    }

    unsafe fn line(dc: HDC, a: (i32, i32), b: (i32, i32), color: COLORREF, width: i32) {
        unsafe {
            let pen = CreatePen(PS_SOLID, width, color);
            let old = SelectObject(dc, pen.into());
            let _ = MoveToEx(dc, a.0, a.1, None);
            let _ = LineTo(dc, b.0, b.1);
            SelectObject(dc, old);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(pen.into());
        }
    }

    unsafe fn write(dc: HDC, x: i32, y: i32, text: &str, color: COLORREF) {
        unsafe {
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, color);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            let old = SelectObject(dc, font);
            let _ = TextOutW(dc, x, y, &wide);
            SelectObject(dc, old);
            wide.clear();
        }
    }

    fn local(bounds: Rect, point: (i32, i32)) -> (i32, i32) {
        (point.0 - bounds.left, point.1 - bounds.top)
    }

    fn rect_of(bounds: Rect, rect: Rect) -> RECT {
        RECT {
            left: rect.left - bounds.left,
            top: rect.top - bounds.top,
            right: rect.right - bounds.left,
            bottom: rect.bottom - bounds.top,
        }
    }

    unsafe fn paint_rulers(dc: HDC, ruler: &Ruler, palette: &Palette, update: RECT) {
        if !ruler.edge_rulers {
            return;
        }
        let bounds = ruler.bounds;
        let (width, height) = (bounds.width(), bounds.height());
        let thickness = strip(ruler);
        let step = tick_step(ruler.scale);
        let mid = (step / 2).max(5);

        let strips = [
            Rect::new(
                bounds.left,
                bounds.top,
                bounds.right,
                bounds.top + thickness,
            ),
            Rect::new(
                bounds.left,
                bounds.bottom - thickness,
                bounds.right,
                bounds.bottom,
            ),
            Rect::new(
                bounds.left,
                bounds.top,
                bounds.left + thickness,
                bounds.bottom,
            ),
            Rect::new(
                bounds.right - thickness,
                bounds.top,
                bounds.right,
                bounds.bottom,
            ),
        ];
        for strip_rect in strips {
            unsafe {
                fill(dc, rect_of(bounds, strip_rect), palette.panel);
            }
        }
        unsafe {
            frame(
                dc,
                rect_of(
                    bounds,
                    Rect::new(bounds.left, bounds.top, bounds.right, bounds.bottom),
                ),
                palette.edge,
            );
            let top_edge = RECT {
                left: 0,
                top: thickness - 1,
                right: width,
                bottom: thickness,
            };
            let bottom_edge = RECT {
                left: 0,
                top: height - thickness,
                right: width,
                bottom: height - thickness + 1,
            };
            let left_edge = RECT {
                left: thickness - 1,
                top: 0,
                right: thickness,
                bottom: height,
            };
            let right_edge = RECT {
                left: width - thickness,
                top: 0,
                right: width - thickness + 1,
                bottom: height,
            };
            for edge in [top_edge, bottom_edge, left_edge, right_edge] {
                fill(dc, edge, palette.edge);
            }
        }

        // Horizontal rulers.
        let ticks = unsafe { CreateSolidBrush(palette.ink_faint) };
        let draw_horizontal = update.top < thickness || update.bottom > height - thickness;
        if draw_horizontal {
            let mut x = (update.left.max(0) / 5) * 5;
            while x < update.right.min(width) {
                let value = bounds.left + x;
                let major = value.rem_euclid(step) == 0;
                let medium = value.rem_euclid(mid) == 0;
                let length = if major {
                    thickness / 2
                } else if medium {
                    thickness / 3
                } else {
                    thickness / 5
                };
                let top = RECT {
                    left: x,
                    top: thickness - length,
                    right: x + 1,
                    bottom: thickness,
                };
                let bottom = RECT {
                    left: x,
                    top: height - thickness,
                    right: x + 1,
                    bottom: height - thickness + length,
                };
                unsafe {
                    FillRect(dc, &top, ticks);
                    FillRect(dc, &bottom, ticks);
                    if major {
                        write(dc, x + 2, 1, &value.to_string(), palette.ink_dim);
                        write(
                            dc,
                            x + 2,
                            height - thickness + 2,
                            &value.to_string(),
                            palette.ink_dim,
                        );
                    }
                }
                x += 5;
            }
        }

        // Vertical rulers.
        let draw_vertical = update.left < thickness || update.right > width - thickness;
        if draw_vertical {
            let mut y = (update.top.max(0) / 5) * 5;
            while y < update.bottom.min(height) {
                let value = bounds.top + y;
                let major = value.rem_euclid(step) == 0;
                let medium = value.rem_euclid(mid) == 0;
                let length = if major {
                    thickness / 2
                } else if medium {
                    thickness / 3
                } else {
                    thickness / 5
                };
                let left = RECT {
                    left: thickness - length,
                    top: y,
                    right: thickness,
                    bottom: y + 1,
                };
                let right = RECT {
                    left: width - thickness,
                    top: y,
                    right: width - thickness + length,
                    bottom: y + 1,
                };
                unsafe {
                    FillRect(dc, &left, ticks);
                    FillRect(dc, &right, ticks);
                    if major {
                        write(dc, 2, y + 1, &value.to_string(), palette.ink_dim);
                        write(
                            dc,
                            width - thickness + 2,
                            y + 1,
                            &value.to_string(),
                            palette.ink_dim,
                        );
                    }
                }
                y += 5;
            }
        }
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(ticks.into());
        }

        // Cross-highlight where the pointer sits.
        let (px, py) = local(bounds, ruler.pointer);
        unsafe {
            fill(
                dc,
                RECT {
                    left: px,
                    top: 0,
                    right: px + 1,
                    bottom: thickness,
                },
                palette.accent,
            );
            fill(
                dc,
                RECT {
                    left: px,
                    top: height - thickness,
                    right: px + 1,
                    bottom: height,
                },
                palette.accent,
            );
            fill(
                dc,
                RECT {
                    left: 0,
                    top: py,
                    right: thickness,
                    bottom: py + 1,
                },
                palette.accent,
            );
            fill(
                dc,
                RECT {
                    left: width - thickness,
                    top: py,
                    right: width,
                    bottom: py + 1,
                },
                palette.accent,
            );
        }
    }

    unsafe fn paint_guides(dc: HDC, ruler: &Ruler, palette: &Palette) {
        let bounds = ruler.bounds;
        for (index, guide) in ruler.guides.iter().enumerate() {
            let selected = ruler.selected_guide == Some(index);
            let color = if selected {
                palette.accent_hi
            } else {
                palette.accent
            };
            let width = if selected { 2 } else { 1 };
            match *guide {
                Guide::Vertical(x) => unsafe {
                    let local = x - bounds.left;
                    line(dc, (local, 0), (local, bounds.height()), color, width);
                    chip_near(dc, bounds, (local + 4, 4), &x.to_string(), palette);
                },
                Guide::Horizontal(y) => unsafe {
                    let local = y - bounds.top;
                    line(dc, (0, local), (bounds.width(), local), color, width);
                    chip_near(dc, bounds, (4, local + 4), &y.to_string(), palette);
                },
            }
        }
    }

    unsafe fn paint_measurement(
        dc: HDC,
        ruler: &Ruler,
        measurement: Measurement,
        selected: bool,
        palette: &Palette,
    ) {
        let bounds = ruler.bounds;
        let color = if selected {
            palette.accent_hi
        } else {
            palette.accent
        };
        match measurement {
            Measurement::Distance { a, b } => {
                let (la, lb) = (local(bounds, a), local(bounds, b));
                unsafe {
                    line(dc, la, lb, color, if selected { 2 } else { 1 });
                    handle(dc, la, color, ruler.scale);
                    handle(dc, lb, color, ruler.scale);
                    chip_near(
                        dc,
                        bounds,
                        ((la.0 + lb.0) / 2, (la.1 + lb.1) / 2 + 12),
                        &measurement.primary(ruler.units, ruler.dpi_at(a)),
                        palette,
                    );
                }
            }
            Measurement::Box {
                left,
                top,
                right,
                bottom,
            } => {
                let rect = Rect::new(left, top, right, bottom);
                unsafe {
                    let client = rect_of(bounds, rect);
                    frame(dc, client, color);
                    for at in box_handles(left, top, right, bottom) {
                        handle(dc, local(bounds, at), color, ruler.scale);
                    }
                    let (width, height) = ((right - left).abs(), (bottom - top).abs());
                    let label = measurement.primary(ruler.units, ruler.dpi_at((left, top)));
                    let _ = (width, height);
                    chip_near(
                        dc,
                        bounds,
                        (client.left + 4, client.top - 22),
                        &label,
                        palette,
                    );
                }
            }
            Measurement::Angle { vertex, a, b } => {
                let (lv, la, lb) = (local(bounds, vertex), local(bounds, a), local(bounds, b));
                unsafe {
                    line(dc, lv, la, color, if selected { 2 } else { 1 });
                    line(dc, lv, lb, color, if selected { 2 } else { 1 });
                    arc(dc, lv, la, lb, color, ruler.scale);
                    handle(dc, lv, color, ruler.scale);
                    handle(dc, la, color, ruler.scale);
                    handle(dc, lb, color, ruler.scale);
                    chip_near(
                        dc,
                        bounds,
                        (lv.0 + 16, lv.1 + 16),
                        &measurement.primary(ruler.units, ruler.dpi_at(vertex)),
                        palette,
                    );
                }
            }
        }
    }

    unsafe fn paint_angle_preview(dc: HDC, ruler: &Ruler, palette: &Palette) {
        if ruler.angle_points.is_empty() {
            return;
        }
        let bounds = ruler.bounds;
        let vertex = local(bounds, ruler.angle_points[0]);
        let pointer = local(bounds, ruler.pointer);
        unsafe {
            if let Some(a) = ruler.angle_points.get(1) {
                line(dc, vertex, local(bounds, *a), palette.accent_hi, 1);
            }
            line(dc, vertex, pointer, palette.ink_dim, 1);
        }
    }

    unsafe fn arc(
        dc: HDC,
        vertex: (i32, i32),
        a: (i32, i32),
        b: (i32, i32),
        color: COLORREF,
        scale: f64,
    ) {
        let radius = 24.0 * scale;
        let start = f64::from(a.1 - vertex.1).atan2(f64::from(a.0 - vertex.0));
        let end = f64::from(b.1 - vertex.1).atan2(f64::from(b.0 - vertex.0));
        let mut delta = end - start;
        while delta > std::f64::consts::PI {
            delta -= std::f64::consts::TAU;
        }
        while delta < -std::f64::consts::PI {
            delta += std::f64::consts::TAU;
        }
        let mut previous = vertex;
        for step in 0..=20 {
            let angle = start + delta * f64::from(step) / 20.0;
            let point = (
                vertex.0 + (radius * angle.cos()).round() as i32,
                vertex.1 + (radius * angle.sin()).round() as i32,
            );
            if step > 0 {
                unsafe { line(dc, previous, point, color, 1) };
            }
            previous = point;
        }
    }

    unsafe fn handle(dc: HDC, at: (i32, i32), color: COLORREF, scale: f64) {
        let half = (3.0 * scale).round() as i32;
        unsafe {
            fill(
                dc,
                RECT {
                    left: at.0 - half,
                    top: at.1 - half,
                    right: at.0 + half + 1,
                    bottom: at.1 + half + 1,
                },
                color,
            );
        }
    }

    unsafe fn chip_near(dc: HDC, bounds: Rect, at: (i32, i32), text: &str, palette: &Palette) {
        let width = (text.chars().count() as i32 * 7 + 10).min(bounds.width());
        let height = 18;
        let left = at.0.clamp(0, (bounds.width() - width).max(0));
        let top = at.1.clamp(0, (bounds.height() - height).max(0));
        let rect = RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        let fit = ((width - 10).max(7) / 7) as usize;
        unsafe {
            fill(dc, rect, palette.panel2);
            frame(dc, rect, palette.edge);
            write(dc, left + 5, top + 3, &clip(text, fit), palette.ink);
        }
    }

    fn clip(text: &str, chars: usize) -> String {
        if text.chars().count() <= chars {
            return text.to_string();
        }
        let mut out: String = text.chars().take(chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }

    fn hover_area(ruler: &Ruler) -> Rect {
        Rect::new(
            ruler.toolbar.left - 200,
            ruler.toolbar.top - 30,
            ruler.toolbar.right + 200,
            ruler.toolbar.bottom + 30,
        )
    }

    fn full_label(button: Button, ruler: &Ruler) -> String {
        match button {
            Button::Select => "Select / move (V)".into(),
            Button::Distance => "Distance (D)".into(),
            Button::Box => "Box / area (B)".into(),
            Button::Angle => "Angle (A)".into(),
            Button::Guides => {
                if ruler.guides_enabled {
                    "Guides on (G)".into()
                } else {
                    "Guides off (G)".into()
                }
            }
            Button::Snap => {
                if ruler.snap {
                    "Snap to edges on (S)".into()
                } else {
                    "Snap to edges off (S)".into()
                }
            }
            Button::Units => format!("Units: {} (click to cycle)", ruler.units.label()),
            Button::Copy => "Copy measurement (C)".into(),
            Button::Lock => "Lock: click-through; Esc unlocks (L)".into(),
            Button::Close => "Close (Esc)".into(),
        }
    }

    unsafe fn paint_icon(
        dc: HDC,
        item: Button,
        rect: RECT,
        color: COLORREF,
        locked: bool,
        scale: f64,
    ) {
        let side = (16.0 * scale).round() as i32;
        let ox = rect.left + (rect.right - rect.left - side) / 2;
        let oy = rect.top + (rect.bottom - rect.top - side) / 2;
        let s = |value: i32| (f64::from(value) * scale).round() as i32;
        let p = |x: i32, y: i32| (ox + s(x), oy + s(y));
        let width = if scale >= 1.5 { 2 } else { 1 };
        let shape = |a: (i32, i32), b: (i32, i32)| RECT {
            left: a.0,
            top: a.1,
            right: b.0,
            bottom: b.1,
        };
        unsafe {
            match item {
                Button::Select => {
                    let points = [
                        p(4, 2),
                        p(11, 9),
                        p(7, 9),
                        p(10, 13),
                        p(8, 14),
                        p(6, 10),
                        p(4, 12),
                        p(4, 2),
                    ];
                    for pair in points.windows(2) {
                        line(dc, pair[0], pair[1], color, width);
                    }
                }
                Button::Distance => {
                    line(dc, p(2, 8), p(14, 8), color, width);
                    line(dc, p(2, 5), p(2, 11), color, width);
                    line(dc, p(14, 5), p(14, 11), color, width);
                    line(dc, p(5, 5), p(2, 8), color, width);
                    line(dc, p(2, 8), p(5, 11), color, width);
                    line(dc, p(11, 5), p(14, 8), color, width);
                    line(dc, p(14, 8), p(11, 11), color, width);
                }
                Button::Box => {
                    frame(dc, shape(p(3, 4), p(13, 12)), color);
                    for corner in [p(2, 3), p(12, 3), p(2, 11), p(12, 11)] {
                        fill(
                            dc,
                            RECT {
                                left: corner.0,
                                top: corner.1,
                                right: corner.0 + s(2),
                                bottom: corner.1 + s(2),
                            },
                            color,
                        );
                    }
                }
                Button::Angle => {
                    line(dc, p(2, 2), p(2, 14), color, width);
                    line(dc, p(2, 14), p(14, 14), color, width);
                    let mut previous = p(2, 7);
                    for step in 1..=6 {
                        let angle = -std::f64::consts::FRAC_PI_2
                            + std::f64::consts::FRAC_PI_2 * f64::from(step) / 6.0;
                        let point = (
                            2 + (7.0 * angle.cos()).round() as i32,
                            14 + (7.0 * angle.sin()).round() as i32,
                        );
                        let point = p(point.0, point.1);
                        line(dc, previous, point, color, width);
                        previous = point;
                    }
                }
                Button::Guides => {
                    for y in [2, 6, 10] {
                        line(dc, p(6, y), p(6, y + 2), color, width);
                    }
                    for x in [2, 6, 10] {
                        line(dc, p(x, 11), p(x + 2, 11), color, width);
                    }
                    frame(dc, shape(p(4, 9), p(8, 13)), color);
                }
                Button::Snap => {
                    line(dc, p(4, 2), p(4, 9), color, width);
                    line(dc, p(12, 2), p(12, 9), color, width);
                    let points = [p(4, 9), p(5, 12), p(8, 13), p(11, 12), p(12, 9)];
                    for pair in points.windows(2) {
                        line(dc, pair[0], pair[1], color, width);
                    }
                    line(dc, p(3, 2), p(6, 2), color, width);
                    line(dc, p(10, 2), p(13, 2), color, width);
                }
                Button::Units => {
                    frame(dc, shape(p(2, 5), p(14, 11)), color);
                    for x in [5, 8, 11] {
                        line(dc, p(x, 5), p(x, 8), color, width);
                    }
                }
                Button::Copy => {
                    frame(dc, shape(p(3, 3), p(10, 10)), color);
                    frame(dc, shape(p(6, 6), p(13, 13)), color);
                }
                Button::Lock => {
                    frame(dc, shape(p(4, 7), p(12, 13)), color);
                    if locked {
                        let shackle = [p(6, 7), p(6, 5), p(7, 4), p(9, 4), p(10, 5), p(10, 7)];
                        for pair in shackle.windows(2) {
                            line(dc, pair[0], pair[1], color, width);
                        }
                    } else {
                        let shackle = [p(5, 7), p(5, 5), p(6, 4), p(9, 4)];
                        for pair in shackle.windows(2) {
                            line(dc, pair[0], pair[1], color, width);
                        }
                    }
                }
                Button::Close => {
                    line(dc, p(4, 4), p(12, 12), color, width);
                    line(dc, p(12, 4), p(4, 12), color, width);
                }
            }
        }
    }

    unsafe fn paint_toolbar(dc: HDC, ruler: &Ruler, palette: &Palette) {
        let toolbar = rrect(ruler);
        unsafe {
            fill(dc, toolbar, palette.panel);
            frame(dc, toolbar, palette.edge);
        }
        let pad = pad(ruler.scale);
        let grip = grip_width(ruler.scale);
        let button = button_width(ruler.scale);
        let left = toolbar.left + pad + grip;
        // Grip dots.
        for index in 0..3 {
            let x = toolbar.left + pad + 3 + index * 3;
            unsafe {
                fill(
                    dc,
                    RECT {
                        left: x,
                        top: toolbar.top + toolbar.bottom.saturating_sub(toolbar.top) / 2 - 5,
                        right: x + 1,
                        bottom: toolbar.top + (toolbar.bottom - toolbar.top) / 2 + 5,
                    },
                    palette.ink_faint,
                );
            }
        }
        for (index, item) in BUTTONS.iter().enumerate() {
            let rect = RECT {
                left: left + index as i32 * button,
                top: toolbar.top + 3,
                right: left + (index as i32 + 1) * button - 2,
                bottom: toolbar.bottom - 3,
            };
            let active = match item {
                Button::Select => ruler.tool == Tool::Select,
                Button::Distance => ruler.tool == Tool::Distance,
                Button::Box => ruler.tool == Tool::Box,
                Button::Angle => ruler.tool == Tool::Angle,
                Button::Guides => ruler.guides_enabled,
                Button::Snap => ruler.snap,
                Button::Lock => ruler.locked,
                _ => false,
            };
            if active {
                unsafe { fill(dc, rect, palette.panel3) };
            } else if ruler.hover == Some(*item) {
                unsafe { fill(dc, rect, palette.panel2) };
            }
            unsafe {
                paint_icon(
                    dc,
                    *item,
                    rect,
                    if active {
                        palette.accent_hi
                    } else if ruler.hover == Some(*item) {
                        palette.ink
                    } else {
                        palette.ink_dim
                    },
                    ruler.locked,
                    ruler.scale,
                );
            }
        }

        let readout_left = left + BUTTONS.len() as i32 * button;
        let readout = RECT {
            left: readout_left + 2,
            top: toolbar.top + 3,
            right: toolbar.right - pad,
            bottom: toolbar.bottom - 3,
        };
        unsafe { fill(dc, readout, palette.panel2) };
        let (primary, secondary) = match ruler.shown() {
            Some(measurement) => (
                measurement.primary(ruler.units, ruler.dpi_at(measurement.centre())),
                measurement.secondary(),
            ),
            None => (
                format!("{} tool", ruler.tool.label()),
                if ruler.locked {
                    "Locked · Esc unlocks".to_string()
                } else if ruler.guides_enabled {
                    "Drag to measure · L locks · Esc closes".to_string()
                } else {
                    "Drag to measure · Esc closes".to_string()
                },
            ),
        };
        let secondary = if ruler.copied_at.is_some() {
            "Copied to the clipboard".to_string()
        } else {
            secondary
        };
        unsafe {
            let fit = (readout_width(ruler.scale) / 7) as usize;
            write(
                dc,
                readout.left + 7,
                readout.top + 3,
                &clip(&primary, fit),
                palette.ink,
            );
            write(
                dc,
                readout.left + 7,
                readout.top + 16,
                &clip(&secondary, fit),
                palette.ink_faint,
            );
        }
        if ruler.locked {
            unsafe {
                chip_near(
                    dc,
                    ruler.bounds,
                    (
                        ruler.toolbar.left - ruler.bounds.left,
                        ruler.toolbar.bottom - ruler.bounds.top + 4,
                    ),
                    "Locked · Esc unlocks, Esc again closes",
                    palette,
                );
            }
        }
        if let Some(hovered) = ruler.hover {
            let index = BUTTONS
                .iter()
                .position(|candidate| *candidate == hovered)
                .unwrap_or(0) as i32;
            let label = full_label(hovered, ruler);
            let width = (label.chars().count() as i32 * 7 + 12).min(ruler.bounds.width());
            let centre = left + index * button + button / 2;
            let left = (centre - width / 2).clamp(0, (ruler.bounds.width() - width).max(0));
            let top = if toolbar.top - 24 >= 0 {
                toolbar.top - 24
            } else {
                toolbar.bottom + 4
            };
            let rect = RECT {
                left,
                top,
                right: left + width,
                bottom: top + 18,
            };
            unsafe {
                fill(dc, rect, palette.panel2);
                frame(dc, rect, palette.accent);
                write(dc, left + 6, top + 3, &label, palette.ink);
            }
        }
    }

    fn rrect(ruler: &Ruler) -> RECT {
        let toolbar = ruler.toolbar;
        RECT {
            left: toolbar.left - ruler.bounds.left,
            top: toolbar.top - ruler.bounds.top,
            right: toolbar.right - ruler.bounds.left,
            bottom: toolbar.bottom - ruler.bounds.top,
        }
    }

    unsafe fn paint_snap_hint(dc: HDC, ruler: &Ruler, palette: &Palette) {
        if let Some(kind) = ruler.snap_hint {
            let (x, y) = local(ruler.bounds, ruler.pointer);
            unsafe {
                chip_near(dc, ruler.bounds, (x + 14, y + 14), kind.label(), palette);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{constrained, snap_angle, square_point};

        #[test]
        fn shift_pulls_a_distance_onto_an_axis_or_a_diagonal() {
            assert_eq!(constrained((0, 0), (100, 8)), (100, 0));
            assert_eq!(constrained((0, 0), (8, 100)), (0, 100));
            assert_eq!(constrained((0, 0), (100, 92)), (100, 100));
            assert_eq!(constrained((0, 0), (-60, -40)), (-60, -60));
        }

        #[test]
        fn shift_makes_a_box_square_in_the_drag_direction() {
            assert_eq!(square_point((10, 10), (110, 40)), (110, 110));
            assert_eq!(square_point((10, 10), (-40, 10)), (-40, 60));
        }

        #[test]
        fn shift_rounds_an_angle_to_fifteen_degrees() {
            let vertex = (0, 0);
            let a = (100, 0);
            // 40° above the first ray rounds to 45°.
            let b = (
                (100.0 * 40.0_f64.to_radians().cos()).round() as i32,
                (100.0 * 40.0_f64.to_radians().sin()).round() as i32,
            );
            let snapped = snap_angle(vertex, a, b, 15.0);
            let dx = f64::from(snapped.0);
            let dy = f64::from(snapped.1);
            let degrees = dy.atan2(dx).to_degrees();
            assert!((degrees - 45.0).abs() < 0.5, "got {degrees}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CopyFormat, Measurement, SnapKind, Units, angle_degrees, dip, distance, grid_value,
        nearest, tick_step,
    };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn distance_is_the_straight_line_between_two_points() {
        assert!(close(distance((0, 0), (3, 4)), 5.0));
        assert!(close(distance((10, 10), (10, 10)), 0.0));
    }

    #[test]
    fn angle_uses_the_rays_and_handles_degenerate_input() {
        assert!(close(angle_degrees((0, 0), (10, 0), (0, 10)), 90.0));
        assert!(close(angle_degrees((0, 0), (10, 0), (-10, 0)), 180.0));
        assert!(close(angle_degrees((0, 0), (0, 0), (10, 0)), 0.0));
    }

    #[test]
    fn snapping_prefers_the_closest_candidate_within_reach() {
        assert_eq!(nearest([4, 30, 2], 0, 8), Some(2));
        assert_eq!(nearest([12], 0, 8), None);
    }

    #[test]
    fn grid_rounding_and_zero_step() {
        assert_eq!(grid_value(17, 10), 20);
        assert_eq!(grid_value(12, 10), 10);
        assert_eq!(grid_value(12, 0), 12);
    }

    #[test]
    fn tick_steps_get_coarser_as_the_screen_scale_grows() {
        assert_eq!(tick_step(1.0), 50);
        assert_eq!(tick_step(2.0), 25);
    }

    #[test]
    fn dip_converts_with_the_monitor_dpi() {
        assert!(close(dip(100.0, 96), 100.0));
        assert!(close(dip(150.0, 144), 100.0));
    }

    #[test]
    fn a_distance_prints_its_signed_primary_and_secondary_metrics() {
        let measurement = Measurement::Distance {
            a: (10, 10),
            b: (607, 352),
        };
        assert_eq!(measurement.primary(Units::Physical, 96), "597 px");
        assert_eq!(measurement.secondary(), "Δ 597 × 342 · 688 px");
        assert_eq!(
            measurement.formatted(Units::Physical, CopyFormat::Pair, 96),
            "597 × 342 px"
        );
        assert_eq!(
            measurement.formatted(Units::Physical, CopyFormat::Css, 96),
            "width: 597px; height: 342px;"
        );
    }

    #[test]
    fn a_box_reports_area_perimeter_and_json() {
        let measurement = Measurement::Box {
            left: 0,
            top: 0,
            right: 100,
            bottom: 50,
        };
        assert_eq!(measurement.primary(Units::Physical, 96), "100 × 50 px");
        assert_eq!(measurement.secondary(), "area 5,000 px² · perimeter 300 px");
        assert_eq!(
            measurement.formatted(Units::Physical, CopyFormat::Json, 96),
            "{\"width\":100,\"height\":50,\"unit\":\"px\"}"
        );
    }

    #[test]
    fn units_can_be_printed_as_dip_or_both() {
        let measurement = Measurement::Box {
            left: 0,
            top: 0,
            right: 150,
            bottom: 75,
        };
        assert_eq!(measurement.primary(Units::Dip, 144), "100 × 50 dip");
        assert_eq!(
            measurement.primary(Units::Both, 144),
            "150 × 75 px · 100 × 50 dip"
        );
    }

    #[test]
    fn snap_kinds_order_by_strength() {
        assert!(SnapKind::Point < SnapKind::Guide);
        assert!(SnapKind::Window < SnapKind::Edge);
        assert!(SnapKind::Edge < SnapKind::Grid);
    }
}
