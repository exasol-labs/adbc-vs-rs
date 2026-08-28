//! Drives driver provisioning end to end against local doubles: a stub
//! registry, a stub BucketFS endpoint, and a directory standing in for the
//! read-only BucketFS mount a UDF sees.
//!
//! Nothing here reaches the public registry, a real BucketFS, or a container.
//! Every failure the adapter can hit on the way to an installed driver is
//! provoked deliberately, because each one of them surfaces to an operator as
//! a failed `CREATE VIRTUAL SCHEMA` and has to say what to do next.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use adbc_vs_adapter::provisioning::{
    ensure_driver, Bucket, Credentials, Registry, DRIVER_LIBRARY, DRIVER_VERSION,
};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

const BUCKET_PATH: &str = "/default";
const DRIVER_PUT_PATH: &str = "/default/adbc-drivers/libadbc_driver_mysql.so";
const INDEX_PATH: &str = "/index.yaml";
const PACKAGE_PATH: &str = "/mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz";
const LIBRARY_BYTES: &[u8] = b"\x7fELF a driver, for the purposes of this test";
const WRITE_USER: &str = "w";
const WRITE_PASSWORD: &str = "M4b7-bucketfs-write";

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[test]
fn present_driver_skips_registry() {
    let mount = MountDirectory::new();
    mount.install(b"a driver an operator placed here");
    let registry = StubServer::serving(routes(&[]));
    let bucketfs = StubServer::serving(routes(&[]));

    let installed = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect("an installed driver must provision without any network call");

    assert_eq!(installed, mount.driver_path());
    assert!(
        registry.requests().is_empty(),
        "an installed driver must not be looked up in the registry: {:?}",
        registry.requests()
    );
    assert!(
        bucketfs.requests().is_empty(),
        "an installed driver must not be written to BucketFS: {:?}",
        bucketfs.requests()
    );
}

#[test]
fn manifest_filename_must_match_constant() {
    let mount = MountDirectory::new();
    let package = package_declaring("libadbc_driver_mysql_v2.so", LIBRARY_BYTES);
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing_the_pinned_package()),
        (PACKAGE_PATH, package.clone()),
    ]));
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &registry.as_registry_expecting(&digest_of(&package)),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a package whose library is named differently must not install");

    let message = error.to_string();
    assert!(
        message.contains("libadbc_driver_mysql_v2.so"),
        "the error must name the library the package declares: {message}"
    );
    assert!(
        message.contains(DRIVER_LIBRARY),
        "the error must name the library adbc-vs is compiled against: {message}"
    );
    assert!(
        bucketfs.requests().is_empty(),
        "a package adbc-vs cannot address must not be written to BucketFS"
    );
}

#[test]
fn registry_client_rejects_untrusted_certificate() {
    let mount = MountDirectory::new();
    let registry = UntrustedTlsServer::start();
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &Registry::at(&registry.url(), DRIVER_VERSION, &digest_of(b"unreached")),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a registry certificate no root store signs must not be trusted");

    let message = error.to_string();
    assert_eq!(
        registry.served_requests(),
        0,
        "the registry client must hang up on a certificate it cannot verify, before \
         asking that host for anything: {message}"
    );
    assert!(
        message.contains(&registry.url()),
        "the error must name the registry it refused to trust: {message}"
    );
    assert!(
        !mount.driver_path().exists(),
        "no driver may be installed from a registry that could not be authenticated"
    );
    assert!(
        bucketfs.requests().is_empty(),
        "an unauthenticated registry must not lead to a BucketFS write"
    );
}

#[test]
fn checksum_mismatch_aborts() {
    let mount = MountDirectory::new();
    let package = package_declaring(DRIVER_LIBRARY, LIBRARY_BYTES);
    let expected = digest_of(b"the package this release pins");
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing_the_pinned_package()),
        (PACKAGE_PATH, package.clone()),
    ]));
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &registry.as_registry_expecting(&expected),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a package that hashes differently must not install");

    let message = error.to_string();
    assert!(
        message.contains(&expected),
        "the error must report the expected digest: {message}"
    );
    assert!(
        message.contains(&digest_of(&package)),
        "the error must report the computed digest: {message}"
    );
    assert!(
        bucketfs.requests().is_empty(),
        "an unverified package must not be written to BucketFS"
    );
    assert!(
        !mount.driver_path().exists(),
        "an unverified package must leave no driver behind"
    );
}

#[test]
fn unreachable_registry_names_fallback() {
    let mount = MountDirectory::new();
    let closed = closed_port();
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(
        &Registry::at(
            &format!("http://127.0.0.1:{closed}"),
            DRIVER_VERSION,
            &digest_of(b"unreached"),
        ),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a registry nothing answers for must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains(&format!("127.0.0.1:{closed}")),
        "the error must name the registry it could not reach: {message}"
    );
    assert!(
        message.contains(&mount.driver_path().display().to_string()),
        "the error must state the path an operator pre-loads the driver at: {message}"
    );
    assert!(
        message.contains(DRIVER_LIBRARY),
        "the error must name the file an operator pre-loads: {message}"
    );
}

#[test]
fn missing_platform_package_errors() {
    let mount = MountDirectory::new();
    let index = concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: v0.6.0\n",
        "        packages:\n",
        "          - platform: darwin_arm64\n",
        "            url: mysql/v0.6.0/mysql_darwin_arm64_v0.6.0.tar.gz\n",
    );
    let registry = StubServer::serving(routes(&[(INDEX_PATH, index.as_bytes().to_vec())]));
    let bucketfs = StubServer::serving(routes(&[]));

    let error = ensure_driver(&registry.as_registry(), &bucketfs.as_bucket(&mount))
        .expect_err("an index without a linux_amd64 package must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains("linux_amd64"),
        "the error must name the platform it looked for: {message}"
    );
    assert!(
        message.contains(DRIVER_VERSION),
        "the error must name the version it looked in: {message}"
    );
    assert_eq!(
        registry.requested_paths(),
        vec![INDEX_PATH.to_string()],
        "no package may be downloaded once the index publishes none for this platform"
    );
}

#[test]
fn bucketfs_write_failure_hides_credential() {
    let mount = MountDirectory::new();
    let package = package_declaring(DRIVER_LIBRARY, LIBRARY_BYTES);
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing_the_pinned_package()),
        (PACKAGE_PATH, package.clone()),
    ]));
    let bucketfs = StubServer::serving(Arc::new(|_: &StubRequest| StubReply {
        status: 403,
        body: b"forbidden".to_vec(),
    }));

    let error = ensure_driver(
        &registry.as_registry_expecting(&digest_of(&package)),
        &bucketfs.as_bucket(&mount),
    )
    .expect_err("a BucketFS endpoint that rejects the write must fail provisioning");

    let message = error.to_string();
    assert!(
        message.contains("403"),
        "the error must report the status BucketFS returned: {message}"
    );
    assert!(
        !message.contains(WRITE_PASSWORD),
        "the error must not contain the credential it attempted: {message}"
    );
    assert!(
        !format!("{error:?}").contains(WRITE_PASSWORD),
        "the debug rendering must not contain the credential either: {error:?}"
    );

    let attempt = bucketfs
        .requests()
        .into_iter()
        .next()
        .expect("the driver upload must have been attempted");
    assert_eq!(
        attempt.method, "PUT",
        "the driver is written to BucketFS with a PUT"
    );
    assert!(
        attempt.authorization.is_some(),
        "the upload must authenticate with the BUCKETFS_CONNECTION credentials"
    );
}

#[test]
fn locked_bucketfs_path_is_retried_until_the_lock_clears() {
    let mount = MountDirectory::new();
    let package = package_declaring(DRIVER_LIBRARY, LIBRARY_BYTES);
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing_the_pinned_package()),
        (PACKAGE_PATH, package.clone()),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes_after_locking(2));

    let installed = ensure_driver(
        &registry.as_registry_expecting(&digest_of(&package)),
        &bucketfs.as_bucket(&mount),
    )
    .expect("a locked path is BucketFS asking to be retried, not a refusal");

    assert_eq!(installed, mount.driver_path());
    assert_eq!(
        bucketfs.requests().len(),
        3,
        "the upload must be retried once per lock and then accepted"
    );
}

#[test]
fn visibility_wait_times_out() {
    let mount = MountDirectory::new();
    let package = package_declaring(DRIVER_LIBRARY, LIBRARY_BYTES);
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing_the_pinned_package()),
        (PACKAGE_PATH, package.clone()),
    ]));
    let bucketfs = StubServer::serving(Arc::new(|_: &StubRequest| StubReply {
        status: 200,
        body: Vec::new(),
    }));
    let deadline = Duration::from_millis(300);

    let started = Instant::now();
    let error = ensure_driver(
        &registry.as_registry_expecting(&digest_of(&package)),
        &bucketfs.as_bucket(&mount).visible_within(deadline),
    )
    .expect_err("a driver that never appears on the mount must not provision");
    let waited = started.elapsed();

    let message = error.to_string();
    assert!(
        message.contains(&mount.driver_path().display().to_string()),
        "the error must name the path it waited on: {message}"
    );
    assert!(
        message.contains(&format!("{deadline:?}")),
        "the error must name the deadline that elapsed: {message}"
    );
    assert!(
        waited >= deadline,
        "the wait must run to its deadline before failing, not give up at once"
    );
}

#[test]
fn verified_driver_is_installed_and_its_path_returned() {
    let mount = MountDirectory::new();
    let package = package_declaring(DRIVER_LIBRARY, LIBRARY_BYTES);
    let registry = StubServer::serving(routes(&[
        (INDEX_PATH, index_publishing_the_pinned_package()),
        (PACKAGE_PATH, package.clone()),
    ]));
    let bucketfs = StubServer::serving(mount.accepting_writes());

    let installed = ensure_driver(
        &registry.as_registry_expecting(&digest_of(&package)),
        &bucketfs.as_bucket(&mount),
    )
    .expect("a verified package must install");

    assert_eq!(installed, mount.driver_path());
    assert_eq!(
        fs::read(&installed).expect("the installed driver must be readable"),
        LIBRARY_BYTES,
        "the bytes the archive holds must be the bytes BucketFS receives"
    );
    assert_eq!(
        bucketfs.requested_paths(),
        vec![DRIVER_PUT_PATH.to_string()],
        "the driver must be written once, at the path the presence check reads"
    );
}

// ---------------------------------------------------------------------------
// The BucketFS mount a UDF reads, standing in for /buckets/<service>/<bucket>
// ---------------------------------------------------------------------------

struct MountDirectory {
    root: tempfile::TempDir,
}

impl MountDirectory {
    fn new() -> Self {
        Self {
            root: tempfile::TempDir::new().expect("a mount directory must be creatable"),
        }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn driver_path(&self) -> PathBuf {
        self.root.path().join("adbc-drivers").join(DRIVER_LIBRARY)
    }

    fn install(&self, bytes: &[u8]) {
        let path = self.driver_path();
        fs::create_dir_all(path.parent().expect("the driver path has a directory"))
            .expect("the driver directory must be creatable");
        fs::write(path, bytes).expect("the driver must be writable");
    }

    /// A BucketFS stub that stores what it is sent where the mount exposes it,
    /// which is what the real endpoint does once replication catches up.
    fn accepting_writes(&self) -> Handler {
        let driver_path = self.driver_path();
        Arc::new(move |request: &StubRequest| {
            fs::create_dir_all(
                driver_path
                    .parent()
                    .expect("the driver path has a directory"),
            )
            .expect("the driver directory must be creatable");
            fs::write(&driver_path, &request.body).expect("the uploaded driver must be writable");
            StubReply {
                status: 200,
                body: Vec::new(),
            }
        })
    }
    /// A BucketFS stub that answers `locked` requests with HTTP 423 before it
    /// accepts anything, which is what the real endpoint does while it is
    /// still finishing an earlier write or delete of the same object.
    fn accepting_writes_after_locking(&self, locked: usize) -> Handler {
        let accepting = self.accepting_writes();
        let remaining = Arc::new(Mutex::new(locked));
        Arc::new(move |request: &StubRequest| {
            let mut remaining = remaining.lock().expect("the lock counter is not poisoned");
            if *remaining > 0 {
                *remaining -= 1;
                return StubReply {
                    status: 423,
                    body: b"locked".to_vec(),
                };
            }
            accepting(request)
        })
    }
}

// ---------------------------------------------------------------------------
// Package and index fixtures
// ---------------------------------------------------------------------------

/// The digest a registry would have to serve a package under, computed here
/// rather than borrowed from the code under test.
fn digest_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn index_publishing_the_pinned_package() -> Vec<u8> {
    concat!(
        "drivers:\n",
        "  - path: mysql\n",
        "    pkginfo:\n",
        "      - version: v0.6.0\n",
        "        packages:\n",
        "          - platform: linux_amd64\n",
        "            url: mysql/v0.6.0/mysql_linux_amd64_v0.6.0.tar.gz\n",
    )
    .as_bytes()
    .to_vec()
}

/// A gzipped tar package whose `MANIFEST` declares `library` as its driver.
fn package_declaring(library: &str, bytes: &[u8]) -> Vec<u8> {
    let manifest = format!("[Files]\ndriver = \"{library}\"\n");
    let members: [(&str, &[u8]); 3] = [
        ("LICENSE", b"Apache-2.0"),
        ("MANIFEST", manifest.as_bytes()),
        (library, bytes),
    ];

    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (name, content) in members {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(content.len() as u64);
        builder
            .append_data(&mut header, name, content)
            .expect("appending an archive member must succeed");
    }
    builder
        .into_inner()
        .expect("finishing the tar stream must succeed")
        .finish()
        .expect("finishing the gzip stream must succeed")
}

// ---------------------------------------------------------------------------
// A stub HTTP server, and a TLS server no root store signs for
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StubRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

struct StubReply {
    status: u16,
    body: Vec<u8>,
}

type Handler = Arc<dyn Fn(&StubRequest) -> StubReply + Send + Sync>;

/// Answers the given bodies at the given paths, and 404 everywhere else.
fn routes(paths: &[(&str, Vec<u8>)]) -> Handler {
    let table: HashMap<String, Vec<u8>> = paths
        .iter()
        .map(|(path, body)| ((*path).to_owned(), body.clone()))
        .collect();

    Arc::new(
        move |request: &StubRequest| match table.get(&request.path) {
            Some(body) => StubReply {
                status: 200,
                body: body.clone(),
            },
            None => StubReply {
                status: 404,
                body: b"not found".to_vec(),
            },
        },
    )
}

/// The request log and shutdown flag a running [`StubServer`] and its worker
/// thread share, held behind one `Arc` so both sides mutate the same state.
struct ServerState {
    requests: Mutex<Vec<StubRequest>>,
    stopped: AtomicBool,
}

struct StubServer {
    port: u16,
    shared: Arc<ServerState>,
    worker: Option<JoinHandle<()>>,
}

impl StubServer {
    fn serving(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a stub server must be bindable");
        let port = listener
            .local_addr()
            .expect("a bound listener has an address")
            .port();
        listener
            .set_nonblocking(true)
            .expect("a stub listener must poll rather than block its shutdown");

        let shared = Arc::new(ServerState {
            requests: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false),
        });
        let worker = thread::spawn({
            let shared = Arc::clone(&shared);
            move || Self::serve(&listener, &handler, &shared)
        });

        Self {
            port,
            shared,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn as_registry(&self) -> Registry {
        self.as_registry_expecting(&digest_of(b"a package no test serves"))
    }

    fn as_registry_expecting(&self, digest: &str) -> Registry {
        Registry::at(&self.url(), DRIVER_VERSION, digest)
    }

    fn as_bucket(&self, mount: &MountDirectory) -> Bucket {
        Bucket::new(
            &format!("{}{BUCKET_PATH}", self.url()),
            mount.path(),
            Credentials::new(WRITE_USER, WRITE_PASSWORD),
        )
    }

    fn requests(&self) -> Vec<StubRequest> {
        self.shared
            .requests
            .lock()
            .expect("the request log is not poisoned")
            .clone()
    }

    fn requested_paths(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.path)
            .collect()
    }

    fn serve(listener: &TcpListener, handler: &Handler, shared: &ServerState) {
        while !shared.stopped.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_nonblocking(false)
                        .expect("an accepted connection reads blocking");
                    let Some(request) = read_request(&mut stream) else {
                        continue;
                    };
                    let reply = handler(&request);
                    shared
                        .requests
                        .lock()
                        .expect("the request log is not poisoned")
                        .push(request);
                    write_reply(&mut stream, &reply);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return,
            }
        }
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<StubRequest> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => head.push(byte[0]),
        }
    }

    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.lines();
    let mut start = lines.next()?.split(' ');
    let method = start.next()?.to_owned();
    let path = start.next()?.to_owned();

    let mut length = 0_usize;
    let mut authorization = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap_or(0),
            "authorization" => authorization = Some(value.trim().to_owned()),
            _ => {}
        }
    }

    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).ok()?;

    Some(StubRequest {
        method,
        path,
        authorization,
        body,
    })
}

fn write_reply(stream: &mut TcpStream, reply: &StubReply) {
    let head = format!(
        "HTTP/1.1 {} STUB\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}

/// A port nothing listens on: bound to learn its number, then released.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port must be bindable");
    listener
        .local_addr()
        .expect("a bound listener has an address")
        .port()
}

/// How many requests reached the endpoint behind the certificate, and the
/// shutdown flag, shared between a running [`UntrustedTlsServer`] and its
/// worker thread behind one `Arc`.
struct TlsState {
    served: Mutex<usize>,
    stopped: AtomicBool,
}

/// A TLS endpoint presenting a certificate no root store signs for, which is
/// what a host impersonating the registry would present.
///
/// It serves a valid registry index to any client that completes the
/// handshake, so a client that would accept the certificate gets a working
/// registry rather than a broken one. What it counts is how many requests it
/// was ever asked to serve.
struct UntrustedTlsServer {
    port: u16,
    shared: Arc<TlsState>,
    worker: Option<JoinHandle<()>>,
}

impl UntrustedTlsServer {
    fn start() -> Self {
        let certified = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()])
            .expect("a self-signed certificate must be generatable");
        let certificate = certified.cert.der().clone();
        let key =
            rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
        let config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], key.into())
                .expect("a generated certificate and its own key must pair"),
        );

        let listener = TcpListener::bind("127.0.0.1:0").expect("a TLS server must be bindable");
        let port = listener
            .local_addr()
            .expect("a bound listener has an address")
            .port();
        listener
            .set_nonblocking(true)
            .expect("a TLS listener must poll rather than block its shutdown");

        let shared = Arc::new(TlsState {
            served: Mutex::new(0),
            stopped: AtomicBool::new(false),
        });
        let worker = thread::spawn({
            let shared = Arc::clone(&shared);
            move || Self::present_certificate(&listener, &config, &shared)
        });

        Self {
            port,
            shared,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    /// How many requests reached the endpoint behind the certificate.
    fn served_requests(&self) -> usize {
        *self
            .shared
            .served
            .lock()
            .expect("the served count is not poisoned")
    }

    /// Offers the certificate to the first client that connects and, if that
    /// client accepts it, answers the request it then sends.
    fn present_certificate(
        listener: &TcpListener,
        config: &Arc<rustls::ServerConfig>,
        shared: &TlsState,
    ) {
        while !shared.stopped.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_nonblocking(false)
                        .expect("an accepted connection handshakes blocking");
                    let Ok(mut connection) = rustls::ServerConnection::new(Arc::clone(config))
                    else {
                        return;
                    };
                    let mut tls = rustls::Stream::new(&mut connection, &mut stream);

                    let mut head = Vec::new();
                    let mut byte = [0_u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match tls.read(&mut byte) {
                            Ok(0) | Err(_) => return,
                            Ok(_) => head.push(byte[0]),
                        }
                    }

                    *shared
                        .served
                        .lock()
                        .expect("the served count is not poisoned") += 1;
                    let index = index_publishing_the_pinned_package();
                    let reply = format!(
                        "HTTP/1.1 200 STUB\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        index.len()
                    );
                    let _ = tls.write_all(reply.as_bytes());
                    let _ = tls.write_all(&index);
                    let _ = tls.flush();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return,
            }
        }
    }
}

impl Drop for UntrustedTlsServer {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
