//! Custom-scheme requests in, streamed responses out.
//!
//! The app's `WKURLSchemeHandler` hands each request to
//! [`super::LedgelineApp::handle`], which runs it through Ledgeline's own axum
//! router in-process (`Router::oneshot`): no socket, no port. The adapter
//! (modelled on mbr's `mobile/transport.rs`) smooths over what a socket would
//! have supplied and what WebKit does differently for a custom scheme:
//!
//! | Concern | What the adapter does |
//! |---------|-----------------------|
//! | Routing | the URL is reduced to origin-form (`/path?query`), as a server receives it |
//! | `Host` | derived from the URL's authority when the request has none (nothing checks it in-process — see [`ledgeline::Security::in_process`] — but handlers see what an HTTP client would send) |
//! | `Authorization` | forwarded verbatim: the bearer token is still required in-process |
//! | `Content-Encoding` | `Accept-Encoding` is dropped: WebKit hands a scheme handler's body to the page undecoded, so the compression layer must not apply one |
//! | Large bodies | streamed: [`LedgelineResponse::next_chunk`] pulls the body a frame at a time through a small bounded buffer |
//!
//! Ledgeline answers no route with a redirect, so mbr's redirect-as-document
//! replay is not needed here.

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{self, HeaderName, HeaderValue, Method, Request, Response, Uri, header};
use futures::StreamExt;
use tower::ServiceExt;

use super::MobileError;

/// Frames buffered ahead of the app's reads of one body. A body frame is at
/// most a few hundred KiB, so a response holds well under a few megabytes
/// however large the payload.
const BUFFERED_FRAMES: usize = 4;

/// One HTTP header. A list rather than a map so repeated headers survive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Record))]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl Header {
    #[must_use]
    pub fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }
}

/// A response whose body is read in chunks.
///
/// `status` and `headers` are final when [`super::LedgelineApp::handle`]
/// returns; the body follows through [`Self::next_chunk`] until it returns
/// `None`. Dropping the response early cancels the rest of the body.
#[cfg_attr(target_vendor = "apple", derive(uniffi::Object))]
pub struct LedgelineResponse {
    status: u16,
    headers: Vec<Header>,
    body: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Result<Bytes, String>>>,
}

#[cfg_attr(target_vendor = "apple", uniffi::export)]
impl LedgelineResponse {
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub fn headers(&self) -> Vec<Header> {
        self.headers.clone()
    }

    /// The next piece of the body, or `None` once it is complete. An error
    /// means the body broke off: the app should fail the load rather than
    /// show a truncated resource.
    pub async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MobileError> {
        match self.body.lock().await.recv().await {
            None => Ok(None),
            Some(Ok(bytes)) => Ok(Some(bytes.to_vec())),
            Some(Err(message)) => Err(MobileError::Internal { message }),
        }
    }

    /// The rest of the body at once — for small responses (JSON) where
    /// streaming buys nothing.
    pub async fn read_all(&self) -> Result<Vec<u8>, MobileError> {
        let mut body = Vec::new();
        while let Some(chunk) = self.next_chunk().await? {
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

impl LedgelineResponse {
    /// Take a router response apart and start pumping its body on `runtime`,
    /// a few frames ahead of the reader.
    pub(crate) fn from_response(
        response: Response<Body>,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let (parts, body) = response.into_parts();
        let headers = parts
            .headers
            .iter()
            .map(|(name, value)| Header {
                name: name.as_str().to_owned(),
                value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
            })
            .collect();
        let (tx, rx) = tokio::sync::mpsc::channel(BUFFERED_FRAMES);
        runtime.spawn(async move {
            let mut stream = body.into_data_stream();
            while let Some(frame) = stream.next().await {
                let item = frame.map_err(|e| e.to_string());
                let failed = item.is_err();
                // The reader went away: stop reading (and drop the body).
                if tx.send(item).await.is_err() || failed {
                    break;
                }
            }
        });
        Self {
            status: parts.status.as_u16(),
            headers,
            body: tokio::sync::Mutex::new(rx),
        }
    }
}

/// Turn the scheme handler's view of a request into an axum request.
pub(crate) fn build_request(
    method: &str,
    url: &str,
    headers: &[Header],
    body: Vec<u8>,
) -> Result<Request<Body>, MobileError> {
    let invalid = |message: String| MobileError::InvalidRequest { message };

    let method = Method::from_bytes(method.as_bytes())
        .map_err(|e| invalid(format!("method {method:?}: {e}")))?;
    let uri: Uri = url
        .parse()
        .map_err(|e| invalid(format!("url {url:?}: {e}")))?;
    // Route on origin-form (`/path?query`), as a server would receive it.
    let path_and_query = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let origin_form: Uri = path_and_query
        .parse()
        .map_err(|e| invalid(format!("path {path_and_query:?}: {e}")))?;

    let mut request = Request::builder()
        .method(method)
        .uri(origin_form)
        .body(Body::from(body))
        .map_err(|e| invalid(e.to_string()))?;

    let request_headers = request.headers_mut();
    for Header { name, value } in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| invalid(format!("header name {name:?}: {e}")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| invalid(format!("header value for {name}: {e}")))?;
        request_headers.append(name, value);
    }
    // WebKit hands a custom scheme's response body to the page as-is; it does
    // not undo a `Content-Encoding`.
    request_headers.remove(header::ACCEPT_ENCODING);
    if !request_headers.contains_key(header::HOST)
        && let Some(authority) = uri.authority()
    {
        let host = HeaderValue::from_str(authority.as_str())
            .map_err(|e| invalid(format!("host {authority}: {e}")))?;
        request_headers.insert(header::HOST, host);
    }
    Ok(request)
}

/// Run one request through the router.
pub(crate) async fn dispatch(router: Router, request: Request<Body>) -> Response<Body> {
    match router.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_reduces_to_origin_form_and_derives_host() {
        let request = build_request(
            "GET",
            "ledgeline://localhost/api/journal?x=1",
            &[
                Header::new("Accept-Encoding", "gzip"),
                Header::new("Authorization", "Bearer abc"),
            ],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(request.uri(), "/api/journal?x=1");
        assert_eq!(request.headers()[header::HOST], "localhost");
        assert_eq!(request.headers()[header::AUTHORIZATION], "Bearer abc");
        assert!(!request.headers().contains_key(header::ACCEPT_ENCODING));
        // A Host the request names itself is kept.
        let explicit = build_request(
            "GET",
            "ledgeline://localhost/",
            &[Header::new("Host", "elsewhere")],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(explicit.headers()[header::HOST], "elsewhere");
    }

    #[test]
    fn build_request_rejects_garbage() {
        assert!(build_request("GET", "not a url", &[], Vec::new()).is_err());
        assert!(build_request("G E T", "ledgeline://localhost/", &[], Vec::new()).is_err());
        assert!(
            build_request(
                "GET",
                "ledgeline://localhost/",
                &[Header::new("bad header", "x")],
                Vec::new()
            )
            .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bodies_stream_in_frames_and_report_a_broken_body() {
        let frames = futures::stream::iter(
            (0..10u8).map(|i| Ok::<_, std::io::Error>(Bytes::from(vec![i; 1000]))),
        );
        let response = Response::new(Body::from_stream(frames));
        let streamed =
            LedgelineResponse::from_response(response, &tokio::runtime::Handle::current());
        assert_eq!(streamed.status(), 200);
        let first = streamed.next_chunk().await.unwrap().unwrap();
        assert_eq!(first, vec![0; 1000]);
        let rest = streamed.read_all().await.unwrap();
        assert_eq!(rest.len(), 9000);
        assert_eq!(streamed.next_chunk().await.unwrap(), None);

        let broken = futures::stream::iter(vec![
            Ok(Bytes::from_static(b"ok")),
            Err(std::io::Error::other("vanished")),
        ]);
        let response = LedgelineResponse::from_response(
            Response::new(Body::from_stream(broken)),
            &tokio::runtime::Handle::current(),
        );
        assert_eq!(response.next_chunk().await.unwrap().unwrap(), b"ok");
        assert!(response.next_chunk().await.is_err());
    }
}
