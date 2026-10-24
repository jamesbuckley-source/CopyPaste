//! Element lookup for the capture picker, on top of UI Automation.
//!
//! The picker has always asked which window is under the pointer; this asks what
//! inside it, so a browser's page can be captured without its toolbar and a
//! dialog can be captured on its own. Providers are other people's code: one may
//! take milliseconds to answer, another may describe a rectangle that is not on
//! screen at all. Every query therefore reports "nothing" rather than failing, and
//! the picker falls back to the whole window, which is what it has always done.
//!
//! The lookup is deliberately independent of the picker: a plain window capture
//! that wants the page rectangle of a window can ask the same questions.

use windows::Win32::Foundation::{POINT, RECT, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    UIA_CustomControlTypeId, UIA_DataGridControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_GroupControlTypeId, UIA_ListControlTypeId, UIA_MenuBarControlTypeId,
    UIA_PaneControlTypeId, UIA_ScrollBarControlTypeId, UIA_StatusBarControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TableControlTypeId,
    UIA_TitleBarControlTypeId, UIA_ToolBarControlTypeId, UIA_TreeControlTypeId,
    UIA_WindowControlTypeId,
};

use crate::capture::{Element, Rect};

/// How far up the tree a lookup walks before giving up. Providers can report
/// cycles, and a deep tree is not worth stalling the picker for.
const MAX_ANCESTORS: usize = 32;

/// How far outside its window a provider's rectangle may reach and still count as
/// part of it. DWM draws an invisible frame a few pixels wide around every window,
/// and providers include it or not depending on the app.
const SLACK: i32 = 8;

/// A rectangle smaller than this share of the window is a sub-window — a dialog —
/// rather than the window itself.
const SUB_WINDOW_SHARE: i64 = 95;

/// UI Automation, started on the calling thread.
///
/// One lookup is meant to serve a whole pick: creating the client per call would
/// pay for COM activation on every pointer movement.
pub struct Lookup {
    automation: IUIAutomation,
    walker: IUIAutomationTreeWalker,
    /// Declared last so it drops last: COM must be uninitialised after the objects
    /// that were created in it.
    _com: Com,
}

impl Lookup {
    /// Starts UI Automation on this thread. `None` when COM or the client cannot
    /// be started, which the picker takes as "only whole windows can be captured".
    pub fn new() -> Option<Self> {
        // SAFETY: a plain COM initialisation of the calling thread; the apartment
        // is released by `Com` when the lookup is dropped.
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            // RPC_E_CHANGED_MODE means the thread already joined COM in a
            // single-threaded apartment, which serves UI Automation just as well.
            if hr.is_err() && hr != RPC_E_CHANGED_MODE {
                log::debug!("UI Automation unavailable: COM would not start ({hr:?})");
                return None;
            }
            let com = Com(hr.is_ok());
            match create() {
                Ok((automation, walker)) => Some(Self {
                    automation,
                    walker,
                    _com: com,
                }),
                Err(e) => {
                    log::debug!("UI Automation unavailable: {e}");
                    None
                }
            }
        }
    }

    /// The element at a screen point, described against the window that contains
    /// it. `None` when automation has nothing usable to say about the point.
    pub fn element_at(&self, point: (i32, i32), window: Rect) -> Option<Element> {
        // SAFETY: every call is a read on objects this struct owns.
        unsafe {
            let hit = self
                .automation
                .ElementFromPoint(POINT {
                    x: point.0,
                    y: point.1,
                })
                .ok()?;
            // The highlight overlay is transparent to the mouse but still a
            // window, and automation may well report it. Describing our own
            // windows would make the picker measure its own frame.
            if is_ours(&hit) {
                return None;
            }
            let mut described = describe(&hit)?;
            let rect = described.rect;
            absorb(&mut described, &hit, rect, window);
            // A page is the whole answer, so there is nothing to walk up to.
            let mut current = hit;
            if described.document.is_none() {
                for _ in 0..MAX_ANCESTORS {
                    let Ok(parent) = self.walker.GetParentElement(&current) else {
                        break;
                    };
                    let Some(rect) = bounds(&parent) else {
                        break;
                    };
                    // Above the whole window is the desktop or another window
                    // that covers it, which is where this walk belongs to
                    // someone else.
                    if above(rect, window) {
                        break;
                    }
                    // Providers describe scrollable content as one tall
                    // rectangle that reaches outside the window, so an ancestor
                    // is skipped rather than treated as the end of the tree.
                    if belongs_to(rect, window) {
                        absorb(&mut described, &parent, rect, window);
                        if described.document.is_some() {
                            // Nothing above a page changes what a click takes.
                            break;
                        }
                    }
                    current = parent;
                }
            }
            Some(described)
        }
    }
}

/// The COM apartment opened by [`Lookup::new`], released with the lookup.
struct Com(bool);

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balanced by the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
    }
}

fn create() -> windows::core::Result<(IUIAutomation, IUIAutomationTreeWalker)> {
    // SAFETY: the client is a local, in-process COM object.
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let walker = automation.ControlViewWalker()?;
        Ok((automation, walker))
    }
}

/// The element under the pointer, as the first thing to consider.
fn describe(element: &IUIAutomationElement) -> Option<Element> {
    let rect = bounds(element)?;
    let control_type = control_type(element);
    Some(Element {
        rect,
        name: name(element),
        container: is_container(control_type),
        document: (control_type == UIA_DocumentControlTypeId.0).then_some(rect),
        dialog: None,
        chrome: is_chrome(control_type),
    })
}

/// Folds one ancestor into what is known about the point.
fn absorb(described: &mut Element, element: &IUIAutomationElement, rect: Rect, window: Rect) {
    let control_type = control_type(element);
    if described.document.is_none() && control_type == UIA_DocumentControlTypeId.0 {
        described.document = Some(rect);
    }
    if described.dialog.is_none()
        && control_type == UIA_WindowControlTypeId.0
        && sub_window(rect, window)
    {
        described.dialog = Some((rect, name(element)));
    }
    described.chrome |= is_chrome(control_type);
}

fn bounds(element: &IUIAutomationElement) -> Option<Rect> {
    // SAFETY: a property read on a live element.
    let rect: RECT = unsafe { element.CurrentBoundingRectangle() }.ok()?;
    Some(Rect::new(rect.left, rect.top, rect.right, rect.bottom))
}

fn control_type(element: &IUIAutomationElement) -> i32 {
    // SAFETY: a property read on a live element.
    unsafe { element.CurrentControlType() }
        .map(|kind| kind.0)
        .unwrap_or_default()
}

fn name(element: &IUIAutomationElement) -> String {
    // SAFETY: a property read on a live element.
    unsafe { element.CurrentName() }
        .map(|name| name.to_string())
        .unwrap_or_default()
}

/// Whether the element is one of this app's own windows: the capture overlay
/// itself, or the app window behind it. Providers rarely describe a transparent
/// popup, but the picker must never measure its own frame if one does.
fn is_ours(element: &IUIAutomationElement) -> bool {
    // SAFETY: a property read plus the owner lookup for the handle it returns.
    unsafe {
        let Ok(hwnd) = element.CurrentNativeWindowHandle() else {
            return false;
        };
        if hwnd.is_invalid() {
            return false;
        }
        let mut process = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut process));
        process == GetCurrentProcessId()
    }
}

/// Whether an ancestor encloses the whole window, which means the walk has left
/// it: the desktop, or another window the pointer could be inside.
fn above(rect: Rect, window: Rect) -> bool {
    rect.left <= window.left
        && rect.top <= window.top
        && rect.right >= window.right
        && rect.bottom >= window.bottom
        && (rect.width() > window.width() || rect.height() > window.height())
}

/// Whether an ancestor still sits in the window being highlighted.
fn belongs_to(rect: Rect, window: Rect) -> bool {
    rect.left >= window.left - SLACK
        && rect.top >= window.top - SLACK
        && rect.right <= window.right + SLACK
        && rect.bottom <= window.bottom + SLACK
}

/// Whether a `Window` element is a dialog inside the window rather than the
/// window's own element, which covers all of it.
fn sub_window(rect: Rect, window: Rect) -> bool {
    let area = |r: Rect| i64::from(r.width().max(0)) * i64::from(r.height().max(0));
    area(rect) * 100 < area(window) * SUB_WINDOW_SHARE
}

/// Elements that gather other things, so capturing one on its own is a sensible
/// thing to offer rather than a random sliver of the window.
fn is_container(control_type: i32) -> bool {
    [
        UIA_ListControlTypeId,
        UIA_TableControlTypeId,
        UIA_TreeControlTypeId,
        UIA_DataGridControlTypeId,
        UIA_GroupControlTypeId,
        UIA_PaneControlTypeId,
        UIA_EditControlTypeId,
        UIA_DocumentControlTypeId,
        UIA_CustomControlTypeId,
    ]
    .iter()
    .any(|id| id.0 == control_type)
}

/// The window's own frame: hovering any of it means "the whole window".
fn is_chrome(control_type: i32) -> bool {
    [
        UIA_TitleBarControlTypeId,
        UIA_MenuBarControlTypeId,
        UIA_ToolBarControlTypeId,
        UIA_TabControlTypeId,
        UIA_TabItemControlTypeId,
        UIA_ScrollBarControlTypeId,
        UIA_StatusBarControlTypeId,
    ]
    .iter()
    .any(|id| id.0 == control_type)
}
