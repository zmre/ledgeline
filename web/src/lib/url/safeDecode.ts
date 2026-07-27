// Shared URL-decoding primitive for the query-string codecs (SEC-12).
//
// `decodeURIComponent` THROWS `URIError` on a malformed percent sequence — a
// bare `%`, a non-hex pair like `%zz`, or a truncated UTF-8 byte run like
// `%E0`. Both `filters/urlCodec.ts` and `holdings/ui/urlCodec.ts` are called
// from `onMount`, where an uncaught throw kills the whole page mount: a URL as
// trivial as `/?acct=%` rendered the app blank.
//
// A URL segment is untrusted input, so a codec must not treat it as fatal. The
// contract here is total: never throws, and on a malformed segment yields the
// raw text unchanged. Worst case the user sees a literal `%` in an account
// name that then matches nothing — the filter is a no-op instead of an outage.
//
// NOTE `URLSearchParams` has its own, LENIENT percent-decoder that already
// replaces undecodable bytes with U+FFFD, so it never throws. The throw comes
// from the SECOND, explicit decode the codecs apply to each comma-joined
// account segment (names are individually encoded before the join so that
// names containing commas survive the round trip).

/** `decodeURIComponent` that returns the raw segment instead of throwing `URIError`. Total: never throws. */
export function safeDecode(segment: string): string {
    try {
        return decodeURIComponent(segment);
    } catch (err) {
        // Only URIError means "malformed percent escape". Anything else is a
        // real bug (or an OOM) and must not be silently swallowed.
        if (err instanceof URIError) return segment;
        throw err;
    }
}
