//! The [`Native`] requests, served on the tao event loop's thread (the
//! `gui` feature; invoicer PLAN 3c).
//!
//! Three loops host it: the window's ([`super::gui`]), `--server`'s in a
//! desktop build ([`super::serve`]: PDF only), and the CLI's `pdf`
//! ([`super::pdf::run_cli`]). Each wraps a [`NativeEvent`] in its own event
//! type, hands every one to [`DesktopHost::handle`], and gives the API a
//! [`LoopNative`] that posts requests through the loop's proxy.
//!
//! **PDF** is MBR's `pdf_export::ExportSession` (MBR-CHANGES M4): a hidden
//! window and webview on the page, its readiness signal (or `load` plus
//! fonts), then the system's print to a verified file. Its progress events
//! come back through the same proxy.
//!
//! **Share…** is `NSSharingServicePicker` anchored at the SPA's button;
//! **Copy file** puts the PDF's file URL on the general pasteboard (macOS;
//! elsewhere they are not offered).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use ::mbr::errors::PdfError as MbrPdfError;
use ::mbr::pdf_export::{
    self, ExportEvent, ExportSession, ExportSink, JobId, OutputTarget, PdfFlags, PdfOptions,
    PendingOutput, ReadySignal,
};
use ledgeline_shell::gui as shell;
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::window::Window;

use super::APP;
use super::native::{Anchor, Capabilities, Native, NativeRequest, PrintJob, Printed, Reply};

/// What the loop is asked to do: a request from the API, or progress of an
/// export in flight.
pub(crate) enum NativeEvent {
    Request(NativeRequest),
    Export(ExportEvent),
}

/// What each kind of loop can do.
pub(crate) fn capabilities(window: bool) -> Capabilities {
    Capabilities {
        pdf: true,
        share: window && cfg!(target_os = "macos"),
        copy_file: window && cfg!(target_os = "macos"),
        open_url: window,
        open_file: window && cfg!(target_os = "macos"),
        pick_file: window && cfg!(target_os = "macos"),
        open_ledgeline: window && cfg!(target_os = "macos"),
    }
}

/// The API's [`Native`]: posts each request to the loop.
pub(crate) struct LoopNative<E: 'static> {
    proxy: Mutex<EventLoopProxy<E>>,
    wrap: fn(NativeEvent) -> E,
    caps: Capabilities,
}

impl<E: 'static> LoopNative<E> {
    pub(crate) fn new(
        proxy: EventLoopProxy<E>,
        wrap: fn(NativeEvent) -> E,
        caps: Capabilities,
    ) -> Self {
        Self {
            proxy: Mutex::new(proxy),
            wrap,
            caps,
        }
    }
}

impl<E: Send + 'static> Native for LoopNative<E> {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn submit(&self, request: NativeRequest) {
        let proxy = self.proxy.lock().unwrap_or_else(PoisonError::into_inner);
        // Fails only once the loop is gone: the request is dropped with it,
        // and its caller hears that the app closed.
        let _ = proxy.send_event((self.wrap)(NativeEvent::Request(request)));
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        if !self.caps.open_url {
            return Err("this process has no window to open a browser from".to_string());
        }
        shell::open_in_browser(&APP, url);
        Ok(())
    }

    fn open_file(&self, file: &std::path::Path) -> Result<(), String> {
        if !self.caps.open_file {
            return Err("this process has no window to open a PDF from".to_string());
        }
        // An absolute path (never an option), vetted by the caller: an
        // exported PDF in `pdf-dir` or the temporary folder, or a document
        // in the books folder.
        shell::spawn_reaped(std::process::Command::new("/usr/bin/open").arg(file))
            .map(drop)
            .map_err(|e| format!("couldn't open {}: {e}", file.display()))
    }

    fn reveal_file(&self, file: &std::path::Path) -> Result<(), String> {
        if !self.caps.open_file {
            return Err("this process has no window to show a file from".to_string());
        }
        // `open -R`: selected in a Finder window, never launched.
        shell::spawn_reaped(
            std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(file),
        )
        .map(drop)
        .map_err(|e| format!("couldn't show {}: {e}", file.display()))
    }

    fn pick_file(&self, start: &Path, title: &str) -> Result<Option<std::path::PathBuf>, String> {
        if !self.caps.pick_file {
            return Err("this process has no window to choose a file from".to_string());
        }
        // rfd's dialog runs on the main thread for us (as File → Open's
        // picker does, from its own thread) and blocks this one.
        Ok(rfd::FileDialog::new()
            .set_title(title)
            .set_directory(start)
            .pick_file())
    }

    fn open_in_ledgeline(&self, journal: &Path) -> Result<(), String> {
        if !self.caps.open_ledgeline {
            return Err("this process has no window to open Ledgeline from".to_string());
        }
        // Launch Services by bundle id: Ledgeline opens a document handed to
        // it (tao's `Opened`), so it opens these books, or switches to them.
        // `open` waits only for the hand-off, and fails when no app has the
        // id: Ledgeline isn't installed.
        let output = std::process::Command::new("/usr/bin/open")
            .args(["-b", LEDGELINE_BUNDLE_ID])
            .arg(journal)
            .output()
            .map_err(|e| format!("couldn't open Ledgeline: {e}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err("Ledgeline isn't installed on this Mac".to_string())
        }
    }
}

/// Ledgeline's bundle id (`assets/Info.plist.in`).
const LEDGELINE_BUNDLE_ID: &str = "com.zmre.ledgeline";

/// The sink MBR's exports report through: back into the loop.
pub(crate) fn export_sink<E: Send + 'static>(
    proxy: EventLoopProxy<E>,
    wrap: fn(NativeEvent) -> E,
) -> ExportSink {
    let proxy = Mutex::new(proxy);
    Arc::new(move |event| {
        let proxy = proxy.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = proxy.send_event(wrap(NativeEvent::Export(event)));
    })
}

/// The loop thread's half: exports in flight, and the Share… picker.
pub(crate) struct DesktopHost {
    sink: ExportSink,
    exports: HashMap<JobId, (ExportSession, Reply<Printed>)>,
    /// The picker shown last, kept alive while it is on screen.
    #[cfg(target_os = "macos")]
    picker: Option<objc2::rc::Retained<objc2_app_kit::NSSharingServicePicker>>,
}

impl DesktopHost {
    pub(crate) fn new(sink: ExportSink) -> Self {
        Self {
            sink,
            exports: HashMap::new(),
            #[cfg(target_os = "macos")]
            picker: None,
        }
    }

    /// Serve `event`. `window` is the app's window, when there is one
    /// (Share… anchors to it).
    pub(crate) fn handle<T: 'static>(
        &mut self,
        target: &EventLoopWindowTarget<T>,
        event: NativeEvent,
        window: Option<&Window>,
    ) {
        match event {
            NativeEvent::Request(NativeRequest::Print(job, reply)) => {
                self.print(target, job, reply);
            }
            NativeEvent::Request(NativeRequest::Share {
                file,
                anchor,
                reply,
            }) => {
                let _ = reply.send(self.share(window, &file, anchor));
            }
            NativeEvent::Request(NativeRequest::CopyFile { file, reply }) => {
                let _ = reply.send(copy_file(&file));
            }
            NativeEvent::Export(event) => {
                let job = event.job;
                let Some((session, _)) = self.exports.get_mut(&job) else {
                    return;
                };
                if let Some(result) = session.handle(event, &mut std::io::sink())
                    && let Some((session, reply)) = self.exports.remove(&job)
                {
                    // The hidden window closes here, on the loop's thread.
                    drop(session);
                    let _ = reply.send(
                        result
                            .map(|done| Printed {
                                bytes: done.bytes,
                                warnings: done.warnings,
                            })
                            .map_err(|e| report(&e)),
                    );
                }
            }
        }
    }

    fn print<T: 'static>(
        &mut self,
        target: &EventLoopWindowTarget<T>,
        job: PrintJob,
        reply: Reply<Printed>,
    ) {
        let flags = PdfFlags {
            ready: job.ready_on_load.then_some(ReadySignal::Load),
            ..PdfFlags::default()
        };
        // Here, on the main thread: the macOS default paper is read from
        // AppKit. A template's own `@page` wins over it.
        let options = PdfOptions::resolve(&job.pdf, &flags, pdf_export::default_paper);
        let started = PendingOutput::prepare(OutputTarget::File(job.output)).and_then(|output| {
            ExportSession::start(target, &job.url, options, output, Arc::clone(&self.sink))
        });
        match started {
            Ok(session) => {
                self.exports.insert(session.id(), (session, reply));
            }
            Err(error) => {
                let _ = reply.send(Err(report(&error)));
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn share(
        &mut self,
        window: Option<&Window>,
        file: &Path,
        anchor: Option<Anchor>,
    ) -> Result<(), String> {
        let window = window.ok_or("Share… needs the app's window")?;
        let picker = macos::share(window, file, anchor)?;
        if let Some(previous) = self.picker.replace(picker) {
            previous.close();
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    #[allow(clippy::unused_self)]
    fn share(
        &mut self,
        _window: Option<&Window>,
        _file: &Path,
        _anchor: Option<Anchor>,
    ) -> Result<(), String> {
        Err("Share… is macOS only".to_string())
    }
}

#[cfg(target_os = "macos")]
fn copy_file(file: &Path) -> Result<(), String> {
    macos::copy_file(file)
}

#[cfg(not(target_os = "macos"))]
fn copy_file(_file: &Path) -> Result<(), String> {
    Err("Copy file is macOS only".to_string())
}

/// An MBR export error and its causes, one sentence each.
pub(crate) fn report(error: &MbrPdfError) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// Where Share… points from, in the content view's coordinates: the
/// anchor (CSS pixels from the top left, the page at 100 %) flipped into
/// AppKit's bottom-left origin unless the view is flipped; with no anchor,
/// a point at the top centre.
///
/// Pure geometry, so it is compiled and unit-tested everywhere; only the
/// macOS Share… path calls it.
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        dead_code,
        reason = "only the macOS share picker calls it; tests run everywhere"
    )
)]
pub(crate) fn picker_rect(
    anchor: Option<Anchor>,
    view_width: f64,
    view_height: f64,
    flipped: bool,
) -> (f64, f64, f64, f64) {
    let a = anchor.unwrap_or(Anchor {
        x: view_width / 2.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    });
    let clamp = |v: f64, max: f64| {
        if v.is_finite() {
            v.clamp(0.0, max)
        } else {
            0.0
        }
    };
    let x = clamp(a.x, view_width);
    let top = clamp(a.y, view_height);
    let width = clamp(a.width, view_width - x).max(1.0);
    let height = clamp(a.height, view_height - top).max(1.0);
    let y = if flipped {
        top
    } else {
        (view_height - top - height).max(0.0)
    };
    (x, y, width, height)
}

#[cfg(target_os = "macos")]
mod macos {
    //! The AppKit calls. Every `unsafe` of the feature is here.

    use std::path::Path;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2::{AllocAnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSPasteboard, NSPasteboardWriting, NSSharingServicePicker, NSView};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
    use tao::platform::macos::WindowExtMacOS as _;
    use tao::window::Window;

    use super::{Anchor, picker_rect};

    fn file_url(file: &Path) -> Result<Retained<NSURL>, String> {
        let path = file
            .to_str()
            .ok_or_else(|| format!("{} isn't a UTF-8 path", file.display()))?;
        Ok(NSURL::fileURLWithPath(&NSString::from_str(path)))
    }

    /// Show the picker for `file` from `window`'s content view.
    pub(super) fn share(
        window: &Window,
        file: &Path,
        anchor: Option<Anchor>,
    ) -> Result<Retained<NSSharingServicePicker>, String> {
        MainThreadMarker::new().ok_or("Share… must run on the main thread")?;
        let url = file_url(file)?;
        let item: &AnyObject = &url;
        let items = NSArray::<AnyObject>::from_slice(&[item]);
        // SAFETY: `ns_view` is the live window's content view (an NSView),
        // borrowed for this call on the main thread, while `window` is.
        let view: &NSView = unsafe { window.ns_view().cast::<NSView>().as_ref() }
            .ok_or("the window has no content view")?;
        // SAFETY: every item is an NSURL, which conforms to
        // NSPasteboardWriting, as `initWithItems:` requires.
        let picker = unsafe {
            NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items)
        };
        let bounds = view.bounds();
        let (x, y, width, height) = picker_rect(
            anchor,
            bounds.size.width,
            bounds.size.height,
            view.isFlipped(),
        );
        picker.showRelativeToRect_ofView_preferredEdge(
            NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)),
            view,
            NSRectEdge::MinY,
        );
        Ok(picker)
    }

    /// Put `file`'s URL on the general pasteboard: what Finder's Copy puts
    /// there, so Paste in Finder, Mail or a chat attaches the file.
    pub(super) fn copy_file(file: &Path) -> Result<(), String> {
        MainThreadMarker::new().ok_or("Copy file must run on the main thread")?;
        let url = file_url(file)?;
        let item = ProtocolObject::<dyn NSPasteboardWriting>::from_retained(url);
        let items = NSArray::from_retained_slice(&[item]);
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        if pasteboard.writeObjects(&items) {
            Ok(())
        } else {
            Err("the pasteboard refused the file".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_points_from_the_button() {
        let button = Anchor {
            x: 100.0,
            y: 40.0,
            width: 80.0,
            height: 24.0,
        };
        // AppKit's origin is the bottom left.
        assert_eq!(
            picker_rect(Some(button), 1280.0, 800.0, false),
            (100.0, 736.0, 80.0, 24.0)
        );
        assert_eq!(
            picker_rect(Some(button), 1280.0, 800.0, true),
            (100.0, 40.0, 80.0, 24.0)
        );
        // No anchor: the top centre.
        assert_eq!(
            picker_rect(None, 1000.0, 600.0, false),
            (500.0, 599.0, 1.0, 1.0)
        );
        // Off the view, or nonsense: kept inside it.
        let wild = Anchor {
            x: f64::NAN,
            y: 5000.0,
            width: -3.0,
            height: f64::INFINITY,
        };
        let (x, y, w, h) = picker_rect(Some(wild), 1000.0, 600.0, false);
        assert!(x == 0.0 && (0.0..=600.0).contains(&y) && w >= 1.0 && h >= 1.0);
    }

    #[test]
    fn capabilities_follow_the_window() {
        let windowless = capabilities(false);
        assert!(windowless.pdf);
        assert!(
            !windowless.share
                && !windowless.copy_file
                && !windowless.open_url
                && !windowless.open_file
                && !windowless.pick_file
        );
        let window = capabilities(true);
        assert!(window.pdf && window.open_url);
        assert_eq!(window.share, cfg!(target_os = "macos"));
        assert_eq!(window.open_file, cfg!(target_os = "macos"));
        assert_eq!(window.pick_file, cfg!(target_os = "macos"));
    }
}
