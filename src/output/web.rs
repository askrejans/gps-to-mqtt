//! Built-in web dashboard (`web/`), embedded in the binary at compile time.
//!
//! Static assets are public; the page reads data from `/api/v1/*` and sends
//! the bearer token itself, so the API's protection is unchanged. The strict
//! CSP allows no inline script, no remote origins and no framing.

use axum::{
    Router,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};

const INDEX: &str = include_str!("../../web/index.html");
const CSS: &str = include_str!("../../web/app.css");
const JS: &str = include_str!("../../web/app.js");
const ICON: &str = include_str!("../../web/icon.svg");

/// Content-Security-Policy: everything from this origin only, plus map tile
/// images from the configured tile origin when the map is enabled.
pub fn csp(map_origin: Option<&str>) -> String {
    let tiles = map_origin.map(|o| format!(" {o}")).unwrap_or_default();
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:{tiles}; \
         connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    )
}

fn asset(body: &'static str, content_type: &'static str, csp: &HeaderValue) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CONTENT_SECURITY_POLICY, csp.clone()),
            (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
        ],
        body,
    )
        .into_response()
}

pub fn router<S: Clone + Send + Sync + 'static>(map_origin: Option<&str>) -> Router<S> {
    // map_origin is validated by config::map_origin, so this cannot fail;
    // fall back to the map-less policy rather than panic.
    let csp = HeaderValue::from_str(&csp(map_origin))
        .unwrap_or_else(|_| HeaderValue::from_str(&csp(None)).expect("static CSP"));
    let route = |body: &'static str, ct: &'static str| {
        let csp = csp.clone();
        get(move || async move { asset(body, ct, &csp) })
    };
    Router::new()
        .route("/", route(INDEX, "text/html; charset=utf-8"))
        .route("/app.css", route(CSS, "text/css; charset=utf-8"))
        .route("/app.js", route(JS, "text/javascript; charset=utf-8"))
        .route("/icon.svg", route(ICON, "image/svg+xml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn serves_assets_with_csp() {
        let app: Router = router(Some("https://tile.openstreetmap.org"));
        for (path, ct) in [
            ("/", "text/html"),
            ("/app.js", "text/javascript"),
            ("/app.css", "text/css"),
        ] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert!(
                res.headers()[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with(ct)
            );
            assert!(
                res.headers()[header::CONTENT_SECURITY_POLICY]
                    .to_str()
                    .unwrap()
                    .contains("img-src 'self' data: https://tile.openstreetmap.org;")
            );
        }
    }

    #[test]
    fn csp_without_map_has_no_remote_origin() {
        assert!(!csp(None).contains("http"));
        assert!(csp(None).contains("img-src 'self' data:;"));
    }

    #[test]
    fn page_has_no_inline_script_or_remote_resources() {
        assert!(
            !INDEX.contains("<script>"),
            "inline scripts are blocked by CSP"
        );
        for text in [INDEX, CSS, JS] {
            assert!(
                !text.contains("http://") && !text.contains("https://"),
                "no remote resources"
            );
        }
        assert!(
            !JS.contains("innerHTML"),
            "render untrusted text with textContent only"
        );
    }
}
