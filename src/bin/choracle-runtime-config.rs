use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

const DEFAULT_PARENT_CID: u32 = 3;
const DEFAULT_FQDN_CONFIG_PORT: u32 = 11_001;
const MAX_FQDN_BYTES: usize = 253;

#[derive(Debug, Parser)]
#[command(about = "Serve or fetch Choracle runtime configuration over Nitro vsock")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    FetchFqdn {
        #[arg(long, default_value_t = DEFAULT_PARENT_CID)]
        cid: u32,
        #[arg(long, default_value_t = DEFAULT_FQDN_CONFIG_PORT)]
        port: u32,
        #[arg(long, default_value_t = 60)]
        retries: u32,
        #[arg(long, default_value_t = 1_000)]
        retry_delay_millis: u64,
    },
    ServeFqdn {
        #[arg(long)]
        fqdn: String,
        #[arg(long, default_value_t = DEFAULT_FQDN_CONFIG_PORT)]
        port: u32,
    },
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::FetchFqdn {
            cid,
            port,
            retries,
            retry_delay_millis,
        } => {
            let fqdn = fetch_fqdn_with_retry(cid, port, retries, retry_delay_millis)?;
            println!("{fqdn}");
            Ok(())
        }
        Command::ServeFqdn { fqdn, port } => serve_fqdn(validate_fqdn(&fqdn)?, port),
    }
}

fn fetch_fqdn_with_retry(
    cid: u32,
    port: u32,
    retries: u32,
    retry_delay_millis: u64,
) -> Result<String> {
    let attempts = retries.max(1);
    let mut last_error = None;
    for _ in 0..attempts {
        match fetch_fqdn_once(cid, port) {
            Ok(fqdn) => return Ok(fqdn),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(retry_delay_millis));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("failed to fetch FQDN runtime config")))
}

fn fetch_fqdn_once(cid: u32, port: u32) -> Result<String> {
    let mut stream = vsock::connect(cid, port)
        .with_context(|| format!("failed to connect to parent CID {cid} vsock port {port}"))?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .with_context(|| "failed to read FQDN runtime config")?;
    if bytes.len() > MAX_FQDN_BYTES + 1 {
        bail!("FQDN runtime config exceeded {MAX_FQDN_BYTES} bytes");
    }
    let value = std::str::from_utf8(&bytes)
        .with_context(|| "FQDN runtime config was not UTF-8")?
        .trim_end_matches(['\r', '\n']);
    validate_fqdn(value)
}

fn serve_fqdn(fqdn: String, port: u32) -> Result<()> {
    let listener =
        vsock::bind(port).with_context(|| format!("failed to bind vsock port {port}"))?;
    eprintln!("serving Choracle FQDN runtime config on vsock port {port}");
    loop {
        let mut stream = listener
            .accept()
            .with_context(|| "failed to accept vsock client")?;
        stream
            .write_all(fqdn.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .with_context(|| "failed to write FQDN runtime config")?;
    }
}

fn validate_fqdn(value: &str) -> Result<String> {
    if value.is_empty() {
        bail!("proof FQDN cannot be empty");
    }
    if value.len() > MAX_FQDN_BYTES {
        bail!("proof FQDN cannot exceed {MAX_FQDN_BYTES} bytes");
    }
    if value.starts_with("http://") || value.starts_with("https://") {
        bail!("proof FQDN must be a bare DNS name, not a URL");
    }
    if value.ends_with('.') || value.contains('/') || value.contains(':') {
        bail!("proof FQDN must be a bare DNS name");
    }
    if !value.is_ascii() || value.chars().any(|ch| ch.is_ascii_whitespace()) {
        bail!("proof FQDN must contain only non-whitespace ASCII characters");
    }

    let labels: Vec<&str> = value.split('.').collect();
    if labels.len() < 2 {
        bail!("proof FQDN must contain at least two DNS labels");
    }
    for label in labels {
        if label.is_empty() {
            bail!("proof FQDN labels cannot be empty");
        }
        if label.len() > 63 {
            bail!("proof FQDN labels cannot exceed 63 bytes");
        }
        if label.starts_with('-') || label.ends_with('-') {
            bail!("proof FQDN labels cannot start or end with hyphen");
        }
        if !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            bail!("proof FQDN labels may contain only letters, digits, and hyphens");
        }
    }
    Ok(value.to_string())
}

#[cfg(target_os = "linux")]
mod vsock {
    use anyhow::{Context, Result};
    use std::io::{Read, Write};
    use std::mem;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    pub struct VsockStream {
        fd: OwnedFd,
    }

    impl Read for VsockStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let ret = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    buf.as_mut_ptr().cast::<libc::c_void>(),
                    buf.len(),
                )
            };
            if ret < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(ret as usize)
            }
        }
    }

    impl Write for VsockStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let ret = unsafe {
                libc::write(
                    self.fd.as_raw_fd(),
                    buf.as_ptr().cast::<libc::c_void>(),
                    buf.len(),
                )
            };
            if ret < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(ret as usize)
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub struct VsockListener {
        fd: OwnedFd,
    }

    impl VsockListener {
        pub fn accept(&self) -> Result<VsockStream> {
            let fd = unsafe {
                libc::accept4(
                    self.fd.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_CLOEXEC,
                )
            };
            owned_fd(fd, "accept vsock connection").map(|fd| VsockStream { fd })
        }
    }

    pub fn connect(cid: u32, port: u32) -> Result<VsockStream> {
        let fd = socket()?;
        let addr = sockaddr(cid, port);
        let ret = unsafe {
            libc::connect(
                fd.as_raw_fd(),
                (&addr as *const libc::sockaddr_vm).cast::<libc::sockaddr>(),
                mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        };
        if ret < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| "connect failed");
        }
        Ok(VsockStream { fd })
    }

    pub fn bind(port: u32) -> Result<VsockListener> {
        let fd = socket()?;
        let addr = sockaddr(libc::VMADDR_CID_ANY, port);
        let ret = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&addr as *const libc::sockaddr_vm).cast::<libc::sockaddr>(),
                mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        };
        if ret < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| "bind failed");
        }
        let ret = unsafe { libc::listen(fd.as_raw_fd(), 16) };
        if ret < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| "listen failed");
        }
        Ok(VsockListener { fd })
    }

    fn socket() -> Result<OwnedFd> {
        let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        owned_fd(fd, "open vsock socket")
    }

    fn owned_fd(fd: libc::c_int, context: &'static str) -> Result<OwnedFd> {
        if fd < 0 {
            Err(std::io::Error::last_os_error()).with_context(|| context)
        } else {
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    fn sockaddr(cid: u32, port: u32) -> libc::sockaddr_vm {
        let mut addr = unsafe { mem::zeroed::<libc::sockaddr_vm>() };
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_cid = cid;
        addr.svm_port = port;
        addr
    }
}

#[cfg(not(target_os = "linux"))]
mod vsock {
    use anyhow::{bail, Result};
    use std::io::{Read, Write};

    pub struct VsockStream;

    impl Read for VsockStream {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for VsockStream {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Ok(0)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub struct VsockListener;

    impl VsockListener {
        pub fn accept(&self) -> Result<VsockStream> {
            bail!("vsock runtime config is only supported on Linux")
        }
    }

    pub fn connect(_cid: u32, _port: u32) -> Result<VsockStream> {
        bail!("vsock runtime config is only supported on Linux")
    }

    pub fn bind(_port: u32) -> Result<VsockListener> {
        bail!("vsock runtime config is only supported on Linux")
    }
}

#[cfg(test)]
mod tests {
    use super::validate_fqdn;

    #[test]
    fn accepts_bare_dns_names() {
        assert_eq!(
            validate_fqdn("proof.example.com").unwrap(),
            "proof.example.com"
        );
        assert_eq!(
            validate_fqdn("Proof-1.example.co").unwrap(),
            "Proof-1.example.co"
        );
    }

    #[test]
    fn rejects_urls_and_paths() {
        assert!(validate_fqdn("https://proof.example.com").is_err());
        assert!(validate_fqdn("proof.example.com/path").is_err());
        assert!(validate_fqdn("proof.example.com:443").is_err());
    }

    #[test]
    fn rejects_invalid_dns_labels() {
        assert!(validate_fqdn("").is_err());
        assert!(validate_fqdn("localhost").is_err());
        assert!(validate_fqdn("-proof.example.com").is_err());
        assert!(validate_fqdn("proof-.example.com").is_err());
        assert!(validate_fqdn("proof..example.com").is_err());
        assert!(validate_fqdn("proof_example.com").is_err());
        assert!(validate_fqdn("proof.example.com.").is_err());
    }

    #[test]
    fn rejects_whitespace_and_non_ascii() {
        assert!(validate_fqdn("proof example.com").is_err());
        assert!(validate_fqdn("proof.example.com\n").is_err());
        assert!(validate_fqdn("próof.example.com").is_err());
    }
}
