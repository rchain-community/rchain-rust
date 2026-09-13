//! REPL client interface (port of `effects/ReplClient.scala`).

use std::sync::Mutex;

use rchain_models::proto::repl::repl_client::ReplClient as TonicReplClient;
use rchain_models::proto::repl::{CmdRequest, EvalRequest};

/// A thin REPL client (port of `ReplClient[F]`; the `F[_]` effect is simplified to synchronous
/// calls and `Either[Throwable, String]` becomes `Result<String, String>`).
pub trait ReplClient {
    fn run(&self, line: &str) -> Result<String, String>;

    fn eval(
        &self,
        file_names: &[String],
        print_unmatched_sends_only: bool,
    ) -> Vec<Result<String, String>>;
}

/// A tonic-backed REPL client (port of `GrpcReplClient`).
///
/// The trait is synchronous, so each call blocks on the stored [`tokio::runtime::Handle`]; the REPL
/// loop is expected to run on a blocking thread (e.g. under `tokio::task::spawn_blocking`), where
/// `Handle::block_on` is legal.
pub struct GrpcReplClient {
    handle: tokio::runtime::Handle,
    inner: Mutex<TonicReplClient<tonic::transport::Channel>>,
}

impl GrpcReplClient {
    pub async fn connect(host: &str, port: i32, max_message_size: i32) -> Result<Self, String> {
        let handle = tokio::runtime::Handle::current();
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{host}:{port}"))
            .map_err(|e| e.to_string())?;
        let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
        let max_message_size = usize::try_from(max_message_size)
            .map_err(|_| format!("negative max message size: {max_message_size}"))?;
        let inner = TonicReplClient::new(channel).max_decoding_message_size(max_message_size);
        Ok(Self {
            handle,
            inner: Mutex::new(inner),
        })
    }

    fn eval_one(
        &self,
        file_name: &str,
        print_unmatched_sends_only: bool,
    ) -> Result<String, String> {
        let content = std::fs::read_to_string(file_name)
            .map_err(|_| format!("File not found: {file_name}"))?;
        self.handle.block_on(async {
            let mut client = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let response = client
                .eval(EvalRequest {
                    program: content,
                    print_unmatched_sends_only,
                })
                .await
                .map_err(|s| s.to_string())?;
            Ok(response.into_inner().output)
        })
    }
}

impl ReplClient for GrpcReplClient {
    fn run(&self, line: &str) -> Result<String, String> {
        self.handle.block_on(async {
            let mut client = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let response = client
                .run(CmdRequest {
                    line: line.to_string(),
                })
                .await
                .map_err(|s| s.to_string())?;
            Ok(response.into_inner().output)
        })
    }

    fn eval(
        &self,
        file_names: &[String],
        print_unmatched_sends_only: bool,
    ) -> Vec<Result<String, String>> {
        file_names
            .iter()
            .map(|f| self.eval_one(f, print_unmatched_sends_only))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    use rchain_models::proto::repl::repl_server::{Repl, ReplServer};
    use rchain_models::proto::repl::ReplResponse;
    use tonic::{Request, Response, Status};

    /// A REPL service that echoes what it was asked, so a test can see that the *file's contents*
    /// reached the server (rather than only that some call succeeded). The log is shared by handle
    /// rather than by wrapping the service in an `Arc`, because tonic's `ReplServer::new` takes the
    /// service by value.
    #[derive(Clone, Default)]
    struct EchoRepl {
        seen: std::sync::Arc<StdMutex<Vec<String>>>,
    }

    #[tonic::async_trait]
    impl Repl for EchoRepl {
        async fn run(&self, request: Request<CmdRequest>) -> Result<Response<ReplResponse>, Status> {
            let line = request.into_inner().line;
            self.seen.lock().unwrap().push(line.clone());
            Ok(Response::new(ReplResponse {
                output: format!("ran: {line}"),
            }))
        }

        async fn eval(
            &self,
            request: Request<EvalRequest>,
        ) -> Result<Response<ReplResponse>, Status> {
            let req = request.into_inner();
            self.seen.lock().unwrap().push(req.program.clone());
            Ok(Response::new(ReplResponse {
                output: format!(
                    "eval[unmatched={}]: {}",
                    req.print_unmatched_sends_only, req.program
                ),
            }))
        }
    }

    /// Serve the echo REPL on an ephemeral port and connect a client to it. The client's trait is
    /// synchronous (`Handle::block_on`), so the test drives it from a blocking thread — which is
    /// where the REPL loop is documented to run.
    async fn client_on_ephemeral_port(
        max_message_size: i32,
    ) -> (GrpcReplClient, std::sync::Arc<StdMutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let service = EchoRepl::default();
        let seen = service.seen.clone();
        let server = tonic::transport::Server::builder()
            .add_service(ReplServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
        tokio::spawn(server);

        let client = GrpcReplClient::connect("127.0.0.1", port as i32, max_message_size)
            .await
            .expect("connect");
        (client, seen)
    }

    /// A file for the eval case, under the platform temp dir (the integration tests roll the same
    /// thing by hand; there is no `tempfile` dependency in this workspace).
    fn scratch_file(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rchain-repl-{name}-{}-{}.rho",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, b"new x in { x!(1) }").expect("write");
        path
    }

    /// `run` performs the RPC and returns the server's output. The client is synchronous over an
    /// async transport (it blocks on the stored handle), so this runs on a blocking thread — the
    /// same arrangement the REPL loop uses.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_round_trips_through_the_service() {
        let (client, seen) = client_on_ephemeral_port(4 * 1024 * 1024).await;

        let output = tokio::task::spawn_blocking(move || client.run("1 + 1"))
            .await
            .expect("blocking task")
            .expect("run");
        assert_eq!(output, "ran: 1 + 1");
        assert_eq!(seen.lock().unwrap().as_slice(), ["1 + 1"]);
    }

    /// `eval` reads each **file** and sends its contents, and the flag rides along — the server
    /// echoes the program back, so a client that sent the file *name* instead of its contents (or
    /// dropped the flag) fails here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_sends_the_file_contents_and_the_flag() {
        let (client, seen) = client_on_ephemeral_port(4 * 1024 * 1024).await;
        let path = scratch_file("eval");
        let name = path.display().to_string();

        let results = tokio::task::spawn_blocking(move || client.eval(&[name], true))
            .await
            .expect("blocking task");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].as_ref().expect("eval result"),
            "eval[unmatched=true]: new x in { x!(1) }"
        );
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["new x in { x!(1) }"],
            "the file's contents were sent, not its name"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A file that does not exist is an error naming it, checked **before** any RPC — the REPL
    /// reports "File not found: …" for a missing script rather than sending an empty program.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_reports_a_missing_file_without_calling_the_service() {
        let (client, seen) = client_on_ephemeral_port(4 * 1024 * 1024).await;
        let missing = "/definitely/not/here.rho".to_string();
        let expected = format!("File not found: {missing}");

        let results = tokio::task::spawn_blocking(move || client.eval(&[missing], false))
            .await
            .expect("blocking task");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].as_ref().expect_err("no such file"), &expected);
        assert!(
            seen.lock().unwrap().is_empty(),
            "the service was not called"
        );
    }

    /// One result per file, **in order**: a mix of a readable file and a missing one yields an `Ok`
    /// and an `Err` in the order asked, so the caller can line the results up with the arguments.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn eval_returns_one_result_per_file_in_order() {
        let (client, _) = client_on_ephemeral_port(4 * 1024 * 1024).await;
        let path = scratch_file("order");
        let name = path.display().to_string();
        let missing = "/definitely/not/here.rho".to_string();

        let names = vec![name.clone(), missing.clone()];
        let results = tokio::task::spawn_blocking(move || client.eval(&names, false))
            .await
            .expect("blocking task");

        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok(), "the first file exists");
        assert!(results[1].is_err(), "the second does not");

        let _ = std::fs::remove_file(&path);
    }

    /// A negative `max_message_size` is refused **by name** rather than cast to a huge `usize` —
    /// the conversion is checked, so a bad configuration cannot silently become an unbounded
    /// decoder.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_negative_max_message_size_is_refused_by_name() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tonic::transport::Server::builder()
            .add_service(ReplServer::new(EchoRepl::default()))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
        tokio::spawn(server);

        let err = GrpcReplClient::connect("127.0.0.1", port as i32, -1)
            .await
            .map(|_| ())
            .expect_err("a negative size");
        assert_eq!(err, "negative max message size: -1");
    }
}
