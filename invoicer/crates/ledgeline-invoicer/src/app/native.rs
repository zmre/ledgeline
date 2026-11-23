//! What the API asks of the desktop (invoicer PLAN 3c): print a page to
//! PDF, show Share…, put a file on the pasteboard, open a URL in the
//! browser or an exported PDF in its viewer. All but the last two need the app's main thread and its tao event
//! loop, which the API (on tokio's pool) doesn't have; a [`Native`] carries
//! each request there and its answer back.
//!
//! Implementations:
//!
//! - the `gui` feature's event-loop host ([`super::desktop`]): the window
//!   (everything), `--server` in a desktop build (PDF only: no window to
//!   share from, and no browser to open), and the CLI's `pdf`;
//! - none: a server-only build, or `--server` where the platform can't
//!   print (Linux with no display). The routes answer `503` with why.
//! - a mock in the router tests, which "prints" a stub PDF.
//!
//! A server-only build has no implementation, so the requests' fields are
//! only ever built there, never read.
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

use std::path::PathBuf;

use mbr_core::pdf_config::PdfConfig;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// The answer to a request, sent back from the event loop. A dropped
/// sender means the app closed first.
pub(crate) type Reply<T> = oneshot::Sender<Result<T, String>>;

/// One page to print.
#[derive(Debug, Clone)]
pub(crate) struct PrintJob {
    /// The page, on this process's server (a one-time export link).
    pub url: String,
    /// The file to print to. Its folder must exist.
    pub output: PathBuf,
    /// The books' MBR `[pdf]` settings (paper and margin defaults; a
    /// template's own `@page` wins).
    pub pdf: PdfConfig,
    /// Print on `load` plus fonts, not the page's own ready signal.
    pub ready_on_load: bool,
}

/// What a print produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Printed {
    pub bytes: u64,
    pub warnings: Vec<String>,
}

/// Where the Share… picker points from: the button, in the page's CSS
/// pixels from the top left of the window's content.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(crate) struct Anchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A request for the main thread.
pub(crate) enum NativeRequest {
    Print(PrintJob, Reply<Printed>),
    Share {
        file: PathBuf,
        anchor: Option<Anchor>,
        reply: Reply<()>,
    },
    CopyFile {
        file: PathBuf,
        reply: Reply<()>,
    },
}

impl std::fmt::Debug for NativeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Print(job, _) => f.debug_tuple("Print").field(&job.url).finish(),
            Self::Share { file, .. } => f.debug_tuple("Share").field(file).finish(),
            Self::CopyFile { file, .. } => f.debug_tuple("CopyFile").field(file).finish(),
        }
    }
}

/// What this process can do natively. The SPA hides what it can't.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Capabilities {
    /// Print a page to PDF.
    pub pdf: bool,
    /// Share… (`NSSharingServicePicker`).
    pub share: bool,
    /// Copy file (a file URL on the pasteboard).
    pub copy_file: bool,
    /// Open a URL in the user's browser. Without it the SPA opens it itself
    /// (a browser tab under `--server`).
    pub open_url: bool,
    /// Open an exported PDF in the default viewer (`open`, macOS).
    pub open_file: bool,
}

/// The desktop, as the API sees it.
pub(crate) trait Native: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    /// Hand `request` to the main thread. Its reply channel answers; if the
    /// app is closing, the request (and so the channel) is dropped.
    fn submit(&self, request: NativeRequest);

    /// Open `url` (already vetted: plain `https`) in the user's browser.
    ///
    /// # Errors
    /// When this process can't.
    fn open_url(&self, url: &str) -> Result<(), String>;

    /// Open `file` (already vetted: an exported PDF) in the default viewer.
    ///
    /// # Errors
    /// When this process can't.
    fn open_file(&self, file: &std::path::Path) -> Result<(), String>;
}

/// Wait for a reply; a dropped sender is the app closing.
pub(crate) async fn answer<T>(reply: oneshot::Receiver<Result<T, String>>) -> Result<T, String> {
    reply
        .await
        .unwrap_or_else(|_| Err("the app closed before it finished".to_string()))
}

#[cfg(test)]
pub(crate) mod mock {
    //! A [`Native`] for the router tests: "prints" a stub PDF holding the
    //! URL it was asked for, and records shares and copies.

    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub(crate) struct MockNative {
        pub caps: Capabilities,
        pub printed: Mutex<Vec<PrintJob>>,
        pub shared: Mutex<Vec<PathBuf>>,
        pub copied: Mutex<Vec<PathBuf>>,
        pub opened: Mutex<Vec<String>>,
        pub opened_files: Mutex<Vec<PathBuf>>,
        /// Fail every print with this.
        pub fail: Option<String>,
    }

    impl MockNative {
        pub(crate) fn everything() -> Self {
            Self {
                caps: Capabilities {
                    pdf: true,
                    share: true,
                    copy_file: true,
                    open_url: true,
                    open_file: true,
                },
                ..Self::default()
            }
        }
    }

    impl Native for MockNative {
        fn capabilities(&self) -> Capabilities {
            self.caps
        }

        fn submit(&self, request: NativeRequest) {
            match request {
                NativeRequest::Print(job, reply) => {
                    let result = match &self.fail {
                        Some(message) => Err(message.clone()),
                        None => {
                            let body = format!("%PDF-1.7 stub for {}\n", job.url);
                            std::fs::write(&job.output, &body)
                                .map(|()| Printed {
                                    bytes: body.len() as u64,
                                    warnings: Vec::new(),
                                })
                                .map_err(|e| e.to_string())
                        }
                    };
                    self.printed.lock().unwrap().push(job);
                    let _ = reply.send(result);
                }
                NativeRequest::Share { file, reply, .. } => {
                    self.shared.lock().unwrap().push(file);
                    let _ = reply.send(Ok(()));
                }
                NativeRequest::CopyFile { file, reply } => {
                    self.copied.lock().unwrap().push(file);
                    let _ = reply.send(Ok(()));
                }
            }
        }

        fn open_url(&self, url: &str) -> Result<(), String> {
            self.opened.lock().unwrap().push(url.to_string());
            Ok(())
        }

        fn open_file(&self, file: &std::path::Path) -> Result<(), String> {
            self.opened_files.lock().unwrap().push(file.to_path_buf());
            Ok(())
        }
    }
}
