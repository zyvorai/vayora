//! The payer's device, read from their own request, in the words payment partners ask for.
//! A guess from the `User-Agent`: good enough to choose between an intent and a QR, never used to decide
//! anything about money. Anything it cannot tell becomes `others`, and an unknown device is treated as a phone
//! because an intent link degrades better than a QR nobody can scan.

use axum::http::HeaderMap;
use zyvor_providers::ClientEnv;

pub fn from_headers(h: &HeaderMap) -> ClientEnv {
    let ua = h
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    from_user_agent(ua)
}

pub fn from_user_agent(ua: &str) -> ClientEnv {
    let l = ua.to_ascii_lowercase();
    let has = |s: &str| l.contains(s);
    let os = if has("android") {
        "android"
    } else if has("iphone") || has("ipad") || has("ipod") {
        "ios"
    } else if has("windows") {
        "windows"
    } else if has("mac os x") || has("macintosh") {
        "macos"
    } else if has("linux") || has("x11") {
        "linux"
    } else {
        "others"
    };
    let device = if has("ipad") || has("tablet") {
        "tablet"
    } else if has("mobile") || os == "android" || os == "ios" || ua.is_empty() {
        "mobile"
    } else {
        "desktop"
    };
    // In-app web views: Android's `; wv)` marker, and iOS without a `Safari/` token.
    let webview = has("; wv)") || (os == "ios" && !has("safari/"));
    let browser = if has("edg/") || has("edga/") || has("edgios/") {
        "edge"
    } else if has("firefox/") || has("fxios/") {
        "firefox"
    } else if has("chrome/") || has("crios/") {
        "chrome"
    } else if has("safari/") {
        "safari"
    } else {
        "others"
    };
    ClientEnv {
        device: device.into(),
        os: os.into(),
        rendering: if device == "desktop" {
            "mweb".into() // not meaningful off mobile; the partner only asks for it on mobile
        } else if webview {
            "webview".into()
        } else {
            "mweb".into()
        },
        browser: browser.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ua: &str) -> (String, String, String, String) {
        let e = from_user_agent(ua);
        (e.device, e.os, e.rendering, e.browser)
    }

    #[test]
    fn phones_tablets_and_desktops() {
        let s = |a: &str, b: &str, c: &str, d: &str| (a.into(), b.into(), c.into(), d.into());
        assert_eq!(
            t(
                "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36"
            ),
            s("mobile", "android", "mweb", "chrome")
        );
        assert_eq!(
            t(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1"
            ),
            s("mobile", "ios", "mweb", "safari")
        );
        assert_eq!(
            t("Mozilla/5.0 (iPad; CPU OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1").0,
            "tablet"
        );
        assert_eq!(
            t(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
            ),
            s("desktop", "macos", "mweb", "chrome")
        );
        assert_eq!(
            t("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0").3,
            "edge"
        );
        assert_eq!(
            t("Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0"),
            s("desktop", "linux", "mweb", "firefox")
        );
    }

    #[test]
    fn web_views_are_flagged_and_unknown_clients_get_the_phone_flow() {
        assert_eq!(
            t("Mozilla/5.0 (Linux; Android 14; Pixel 8; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/126.0.0.0 Mobile Safari/537.36").2,
            "webview"
        );
        assert_eq!(
            t("Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148").2,
            "webview"
        );
        assert_eq!(from_user_agent("").device, "mobile");
        assert_eq!(from_user_agent("curl/8").os, "others");
    }
}
