// The WebView's user agent identifies the host OS (WebView2 reports "Windows NT").
// Used only to choose platform-specific instructions, never for security decisions.
export const isWindows = typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent);
