//! Read-only local browser suggestions. This protocol intentionally has no password operation.
//!
//! Windows ACLs restrict the pipe to the current user and SYSTEM. They do not authenticate
//! a particular extension or native-host executable: a process running as the same user
//! can request these login summaries. Origins must still be validated by the vault handler.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_NATIVE_REQUEST_BYTES: usize = 8 * 1024;
pub const MAX_FRAME_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Match { version: u32, origin: String },
}

impl Request {
    pub fn validate(&self) -> io::Result<()> {
        match self {
            Self::Match { version, origin }
                if *version == PROTOCOL_VERSION
                    && !origin.is_empty()
                    && origin.len() <= 4096
                    && !origin.contains('\0') =>
            {
                Ok(())
            }
            _ => Err(invalid("Unsupported or invalid suggestion request.")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Locked,
    Unavailable,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Match {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub status: Status,
    pub matches: Vec<Match>,
}

impl Response {
    pub fn empty(status: Status) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            status,
            matches: Vec::new(),
        }
    }
    fn validate(&self) -> io::Result<()> {
        if self.version != PROTOCOL_VERSION
            || (self.status != Status::Ok && !self.matches.is_empty())
        {
            return Err(invalid("Invalid suggestion response."));
        }
        Ok(())
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn check_length(length: usize, maximum: usize) -> io::Result<()> {
    if length == 0 || length > maximum.min(MAX_FRAME_BYTES) {
        return Err(invalid("Local message exceeds its allowed size."));
    }
    Ok(())
}

/// Read a Chromium Native Messaging / local-pipe frame. EOF and partial headers
/// or payloads are errors. The allocation is bounded before reading.
/// This generic synchronous helper cannot impose deadlines on its supplied reader.
pub fn read_frame(reader: &mut impl Read, maximum: usize) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    loop {
        match reader.read(&mut header[..1]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Local message ended before its header.",
                ))
            }
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    reader.read_exact(&mut header[1..])?;
    let length = u32::from_le_bytes(header) as usize;
    check_length(length, maximum)?;
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    check_length(bytes.len(), MAX_FRAME_BYTES)?;
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}

#[cfg(windows)]
mod windows {
    use super::*;
    #[cfg(test)]
    mod tests;
    use sha2::{Digest, Sha256};
    use std::{ffi::c_void, ptr, sync::mpsc, thread, time::Duration};
    use tokio::{
        io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
        net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
        runtime::Builder,
        sync::oneshot,
        time::{sleep, timeout},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, ERROR_INSUFFICIENT_BUFFER, ERROR_PIPE_BUSY, HANDLE},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    const DEADLINE: Duration = Duration::from_secs(3);

    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct LocalAllocation(*mut c_void);
    impl Drop for LocalAllocation {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }

    fn current_sid() -> io::Result<String> {
        // The kernel determines the token buffer size. u64 storage provides alignment
        // for TOKEN_USER on all supported Windows targets.
        unsafe {
            let mut handle = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = Token(handle);
            let mut needed = 0;
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed);
            if io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
                || needed < std::mem::size_of::<TOKEN_USER>() as u32
                || needed > 65536
            {
                return Err(invalid("Couldn't determine local account identity."));
            }
            let mut buffer = vec![0_u64; (needed as usize).div_ceil(8)];
            if GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut text = ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
                return Err(io::Error::last_os_error());
            }
            let allocation = LocalAllocation(text.cast());
            let mut length = 0;
            while length < 256 && *text.add(length) != 0 {
                length += 1;
            }
            if length == 256 {
                return Err(invalid("Invalid local account identity."));
            }
            let sid = String::from_utf16(std::slice::from_raw_parts(text, length))
                .map_err(|_| invalid("Invalid local account identity."))?;
            drop(allocation);
            Ok(sid)
        }
    }

    fn pipe_name(sid: &str) -> String {
        let digest = Sha256::digest(sid.as_bytes());
        let suffix: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        format!(r"\\.\pipe\fakkarni-browser-v1-{suffix}")
    }

    fn create_server(sid: &str, name: &str, first: bool) -> io::Result<NamedPipeServer> {
        // Protected DACL; no inherited Everyone/Authenticated Users entries.
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})\0")
            .encode_utf16()
            .collect();
        unsafe {
            let mut descriptor = ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let allocation = LocalAllocation(descriptor);
            let mut attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let server = ServerOptions::new()
                .first_pipe_instance(first)
                .max_instances(2)
                .reject_remote_clients(true)
                .in_buffer_size(MAX_FRAME_BYTES as u32)
                .out_buffer_size(MAX_FRAME_BYTES as u32)
                .create_with_security_attributes_raw(
                    name,
                    (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
                );
            drop(allocation);
            server
        }
    }

    async fn read_async(
        reader: &mut (impl AsyncRead + Unpin),
        maximum: usize,
    ) -> io::Result<Vec<u8>> {
        let length = reader.read_u32_le().await? as usize;
        check_length(length, maximum)?;
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).await?;
        Ok(bytes)
    }
    async fn write_async(
        writer: &mut (impl AsyncWrite + Unpin),
        bytes: &[u8],
        maximum: usize,
    ) -> io::Result<()> {
        check_length(bytes.len(), maximum)?;
        writer.write_u32_le(bytes.len() as u32).await?;
        writer.write_all(bytes).await?;
        writer.flush().await
    }

    async fn exchange(
        server: &mut NamedPipeServer,
        handler: &impl Fn(Request) -> Response,
    ) -> io::Result<()> {
        let bytes = read_async(server, MAX_NATIVE_REQUEST_BYTES).await?;
        let response = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) if request.validate().is_ok() => {
                // A faulty handler cannot kill the bridge. Handlers must be nonblocking
                // (use try_lock for the vault); Tokio deadlines cannot preempt sync work.
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(request)))
                    .unwrap_or_else(|_| Response::empty(Status::Error))
            }
            _ => Response::empty(Status::Error),
        };
        let bytes = if response.validate().is_ok() {
            serde_json::to_vec(&response).map_err(|_| invalid("Couldn't serialize suggestions."))?
        } else {
            serde_json::to_vec(&Response::empty(Status::Error)).expect("fixed response serializes")
        };
        if bytes.len() > MAX_FRAME_BYTES {
            let error = serde_json::to_vec(&Response::empty(Status::Error))
                .expect("fixed response serializes");
            write_async(server, &error, MAX_FRAME_BYTES).await?;
        } else {
            write_async(server, &bytes, MAX_FRAME_BYTES).await?;
        }
        // DisconnectNamedPipe discards unread data. Wait until the client has read
        // its response and closed, within the enclosing per-connection deadline.
        let mut extra = [0_u8];
        let _ = server.read(&mut extra).await;
        Ok(())
    }

    /// Dropping the guard stops accepting requests and cancels pending I/O. It does
    /// not wait for the worker or synchronously erase summaries already delivered.
    pub struct ServerGuard {
        shutdown: Option<oneshot::Sender<()>>,
    }
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            if let Some(sender) = self.shutdown.take() {
                let _ = sender.send(());
            }
        }
    }

    /// Start the current user's metadata-only pipe. The handler MUST return promptly
    /// and must enforce current vault lock state and trusted origin matching itself.
    pub fn start_server(
        handler: impl Fn(Request) -> Response + Send + Sync + 'static,
    ) -> io::Result<ServerGuard> {
        let sid = current_sid()?;
        let name = pipe_name(&sid);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        thread::Builder::new()
            .name("fakkarni-browser-bridge".into())
            .spawn(move || {
                let runtime = match Builder::new_current_thread().enable_all().build() {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let mut server = match create_server(&sid, &name, true) {
                        Ok(value) => value,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    if ready_tx.send(Ok(())).is_err() {
                        return;
                    }
                    // Create the next instance BEFORE dropping the connected one.
                    // At least one ACL-protected handle always claims this pipe name.
                    loop {
                        tokio::select! {
                            _ = &mut shutdown_rx => break,
                            connected = server.connect() => { if connected.is_err() { break; } }
                        }
                        let next = match create_server(&sid, &name, false) {
                            Ok(value) => value,
                            Err(_) => break,
                        };
                        tokio::select! {
                            _ = &mut shutdown_rx => break,
                            _ = timeout(DEADLINE, exchange(&mut server, &handler)) => {}
                        }
                        server = next;
                    }
                });
            })?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(ServerGuard {
                shutdown: Some(shutdown_tx),
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Couldn't start local suggestion bridge.",
            )),
        }
    }

    /// Synchronous native-host client. Call from a regular thread, not a Tokio task.
    pub fn request_desktop(request: &Request) -> io::Result<Response> {
        request.validate()?;
        let bytes = serde_json::to_vec(request)
            .map_err(|_| invalid("Couldn't serialize suggestion request."))?;
        check_length(bytes.len(), MAX_NATIVE_REQUEST_BYTES)?;
        let name = pipe_name(&current_sid()?);
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(async {
            timeout(DEADLINE, async {
                let mut pipe = loop {
                    match ClientOptions::new().open(&name) {
                        Ok(pipe) => break pipe,
                        Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                            sleep(Duration::from_millis(25)).await
                        }
                        Err(error) => return Err(error),
                    }
                };
                write_async(&mut pipe, &bytes, MAX_NATIVE_REQUEST_BYTES).await?;
                let response = read_async(&mut pipe, MAX_FRAME_BYTES).await?;
                let response: Response = serde_json::from_slice(&response)
                    .map_err(|_| invalid("Invalid suggestion response."))?;
                response.validate()?;
                Ok(response)
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Local suggestion request timed out.",
                )
            })?
        })
    }
}

#[cfg(windows)]
pub use windows::{request_desktop, start_server, ServerGuard};

#[cfg(not(windows))]
pub struct ServerGuard;
#[cfg(not(windows))]
pub fn start_server(
    _: impl Fn(Request) -> Response + Send + Sync + 'static,
) -> io::Result<ServerGuard> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Local browser bridge currently requires Windows.",
    ))
}
#[cfg(not(windows))]
pub fn request_desktop(_: &Request) -> io::Result<Response> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Local browser bridge currently requires Windows.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_roundtrip_and_clean_eof() {
        let payload = br#"{"version":1,"action":"match","origin":"https://example.com"}"#;
        let mut buffer = Vec::new();
        write_frame(&mut buffer, payload).unwrap();
        let mut reader = buffer.as_slice();
        assert_eq!(
            read_frame(&mut reader, MAX_NATIVE_REQUEST_BYTES).unwrap(),
            payload
        );
        assert_eq!(
            read_frame(&mut reader, MAX_NATIVE_REQUEST_BYTES)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    #[test]
    fn malformed_or_oversized_frames_are_rejected() {
        for header in [0_u32, (MAX_NATIVE_REQUEST_BYTES + 1) as u32, u32::MAX] {
            assert!(read_frame(
                &mut header.to_le_bytes().as_slice(),
                MAX_NATIVE_REQUEST_BYTES
            )
            .is_err());
        }
        assert!(read_frame(&mut [1_u8, 0].as_slice(), MAX_NATIVE_REQUEST_BYTES).is_err());
        assert!(read_frame(
            &mut [2_u8, 0, 0, 0, b'x'].as_slice(),
            MAX_NATIVE_REQUEST_BYTES
        )
        .is_err());
        assert!(write_frame(&mut Vec::new(), &[]).is_err());
    }
    #[test]
    fn protocol_has_no_secret_or_unknown_operations() {
        for bytes in [
            r#"{"action":"reveal","version":1,"id":"x"}"#,
            r#"{"action":"match","version":1,"origin":"https://example.com","password":"x"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bytes).is_err());
        }
        assert!(Request::Match {
            version: 2,
            origin: "https://example.com".into()
        }
        .validate()
        .is_err());
        assert!(Response {
            version: 1,
            status: Status::Locked,
            matches: vec![Match {
                id: "x".into(),
                title: "x".into(),
                username: "x".into(),
                url: "x".into()
            }]
        }
        .validate()
        .is_err());
    }
}
