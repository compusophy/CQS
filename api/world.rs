//! The one Vercel function: `GET /api/world?token=…` for a view,
//! `POST /api/world {token, name?, words?}` to join and to speak. Everything
//! real lives in the `host` crate; this file only turns HTTP into calls.
//!
//! Built by Vercel's Rust runtime on Linux. `vercel_runtime` does not compile
//! on Windows, so on anything but unix this is a stub and the workspace still
//! builds; the real thing is checked by the deploy.

#[cfg(unix)]
mod function {
    use std::sync::OnceLock;

    use http_body_util::BodyExt;
    use vercel_runtime::{run, service_fn, Error, Request, Response, ResponseBody};

    static HOST: OnceLock<Result<host::Host, String>> = OnceLock::new();

    pub async fn serve() -> Result<(), Error> {
        run(service_fn(handler)).await
    }

    async fn handler(req: Request) -> Result<Response<ResponseBody>, Error> {
        let method = req.method().as_str().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let body = req.into_body().collect().await?.to_bytes();
        let body = String::from_utf8_lossy(&body).to_string();

        // The host is blocking (plain HTTPS to the ledger and the model); keep
        // it off the async executor.
        let reply =
            tokio::task::spawn_blocking(move || match HOST.get_or_init(host::Host::from_env) {
                Ok(h) => h.handle(&method, &query, &body),
                Err(e) => host::Reply {
                    status: 500,
                    body: host::error_body(e),
                },
            })
            .await?;

        Ok(Response::builder()
            .status(reply.status)
            .header("content-type", "application/json; charset=utf-8")
            .header("cache-control", "no-store")
            .body(ResponseBody::from(reply.body.to_string()))?)
    }
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    function::serve().await
}

#[cfg(not(unix))]
fn main() {
    eprintln!("the cqs function runs on Vercel (Linux); use `cargo run -p cqs` locally");
}
