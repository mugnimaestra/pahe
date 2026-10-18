# Local CONNECT proxy in front of the Resolver webview

animepahe.pw is DNS-hijacked by the user's ISP and sits behind a Cloudflare JS challenge: a plain HTTP client resolves to a block page, and even with DoH it gets a 403 "Just a moment" page, so resolution requires a real browser engine. A webview cannot override per-host DNS the way Chrome's `--host-resolver-rules` can. We therefore run a local HTTP CONNECT proxy inside the app that resolves every requested hostname through DoH (Cloudflare JSON API, `1.1.1.1` fallback), and attach it to the Resolver webview with Tauri's `proxy_url`. wry implements that per platform: `WKWebsiteDataStore.proxyConfigurations` on macOS (requires the `macos-proxy` feature), WebView2 `--proxy-server` on Windows, WebKitGTK `NetworkProxySettings` on Linux. TLS stays end-to-end; the proxy only tunnels bytes and never intercepts certificates.

Rejected alternatives: changing system DNS (global and invasive), an `/etc/resolver` daemon (needs root), a bundled Chrome/patchright sidecar (bundle weight), and curl-impersonate (cannot execute the JS challenge).

Consequences: resolution now depends on DoH at runtime; on Linux the proxy attaches to the shared `WebContext`, so it is app-wide rather than per-webview; the UI is served from a custom scheme, so it never traverses the proxy.
