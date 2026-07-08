// Pure static SPA: the hledger-web API URL is runtime state from localStorage,
// so nothing can be server-rendered or prerendered.
export const ssr = false;
export const prerender = false;
